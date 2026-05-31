#!/usr/bin/env bash
# 滚动部署 musl binary 到 master + N nodes。先全停再启，避免新老 proto 混跑。
# 用法：
#   scripts/deploy-prod.sh <dir-containing-zhuanfa-master-and-node>
#
# 主机清单从 $ZF_DEPLOY_HOSTS_FILE 读取（默认 ~/.zhuanfa/hosts.conf）。
# 文件格式：每行 `name:ip:port:password:roles`，roles ∈ {master_node, node}。
# 该文件含密码，**严禁入库**——脚本本身不含任何凭证。
set -uo pipefail
cd "$(dirname "$0")/.."

BIN_DIR=${1:-./artifact}
MASTER_BIN="$BIN_DIR/zhuanfa-master"
NODE_BIN="$BIN_DIR/zhuanfa-node"
[ -f "$MASTER_BIN" ] || { echo "缺 $MASTER_BIN"; exit 1; }
[ -f "$NODE_BIN" ]   || { echo "缺 $NODE_BIN"; exit 1; }
command -v sshpass >/dev/null || { echo "需要 sshpass: brew install sshpass"; exit 1; }

HOSTS_FILE=${ZF_DEPLOY_HOSTS_FILE:-$HOME/.zhuanfa/hosts.conf}
if [ ! -f "$HOSTS_FILE" ]; then
    cat <<EOF
缺主机清单：$HOSTS_FILE

示例内容（每行一台主机，# 开头为注释）：
  nosla-hk:MASTER_IP_REDACTED:22:<password>:master_node
  rfchost-172:NODE_RFCHOST_REDACTED:22:<password>:node

把文件权限设成 600，避免泄露：
  mkdir -p ~/.zhuanfa && chmod 700 ~/.zhuanfa
  chmod 600 ~/.zhuanfa/hosts.conf
EOF
    exit 1
fi
perm=$(stat -f '%Lp' "$HOSTS_FILE" 2>/dev/null || stat -c '%a' "$HOSTS_FILE" 2>/dev/null)
if [ "$perm" != "600" ] && [ "$perm" != "400" ]; then
    echo "⚠️  $HOSTS_FILE 权限是 $perm，建议 chmod 600（含明文密码）"
fi

PASS_DIR=$(mktemp -d)
trap "rm -rf '$PASS_DIR'" EXIT
chmod 700 "$PASS_DIR"

# 平行数组（兼容 bash 3.2，macOS 默认 /bin/bash）
NAMES=(); IPS=(); PORTS=(); PASSFILES=(); ROLES=()
MASTER_INDEX=-1

while IFS= read -r line || [ -n "$line" ]; do
    case "$line" in ''|\#*) continue ;; esac
    IFS=: read -r _n _i _p _w _r <<< "$line"
    [ -z "${_n:-}" ] && continue
    f="$PASS_DIR/$_n"
    printf '%s' "$_w" > "$f"
    chmod 600 "$f"
    NAMES+=("$_n"); IPS+=("$_i"); PORTS+=("$_p"); PASSFILES+=("$f"); ROLES+=("$_r")
    if [ "$_r" = "master_node" ]; then
        MASTER_INDEX=$((${#NAMES[@]} - 1))
    fi
done < "$HOSTS_FILE"

N=${#NAMES[@]}
[ $N -gt 0 ] || { echo "$HOSTS_FILE 没有有效条目"; exit 1; }
[ $MASTER_INDEX -ge 0 ] || { echo "$HOSTS_FILE 未指定 roles=master_node 的主机"; exit 1; }
echo "==> 读取 $N 台主机；master = ${NAMES[$MASTER_INDEX]}"

SSH_OPTS=(-o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null
          -o ConnectTimeout=10 -o ServerAliveInterval=5
          -o PreferredAuthentications=password -o PubkeyAuthentication=no
          -o NumberOfPasswordPrompts=1)

ssh_run() {
    local i=$1; shift
    sshpass -f "${PASSFILES[$i]}" ssh "${SSH_OPTS[@]}" -p "${PORTS[$i]}" "root@${IPS[$i]}" "$@"
}
scp_to() {
    local i=$1 src=$2 dst=$3
    sshpass -f "${PASSFILES[$i]}" scp -O "${SSH_OPTS[@]}" -P "${PORTS[$i]}" "$src" "root@${IPS[$i]}:$dst"
}

run_all() {
    local cmd=$1
    for ((i=0; i<N; i++)); do
        ( echo "    [${NAMES[$i]}] $cmd"; ssh_run "$i" "$cmd" ) &
    done
    wait
}

echo "==> [1/6] 停所有 zhuanfa-node"
run_all "systemctl stop zhuanfa-node 2>/dev/null || true"

echo "==> [2/6] 停 zhuanfa-master @ ${NAMES[$MASTER_INDEX]}"
ssh_run "$MASTER_INDEX" "systemctl stop zhuanfa-master 2>/dev/null || true"

echo "==> [3/6] 推 node binary 到所有节点"
for ((i=0; i<N; i++)); do
    echo "    [${NAMES[$i]}] scp zhuanfa-node"
    scp_to "$i" "$NODE_BIN" "/opt/zhuanfa/zhuanfa-node.new"
    ssh_run "$i" "chmod +x /opt/zhuanfa/zhuanfa-node.new && mv -f /opt/zhuanfa/zhuanfa-node.new /opt/zhuanfa/zhuanfa-node && md5sum /opt/zhuanfa/zhuanfa-node | awk '{print \$1}'"
done

echo "==> [4/6] 推 master binary 到 ${NAMES[$MASTER_INDEX]}"
scp_to "$MASTER_INDEX" "$MASTER_BIN" "/opt/zhuanfa/zhuanfa-master.new"
ssh_run "$MASTER_INDEX" "chmod +x /opt/zhuanfa/zhuanfa-master.new && mv -f /opt/zhuanfa/zhuanfa-master.new /opt/zhuanfa/zhuanfa-master && md5sum /opt/zhuanfa/zhuanfa-master | awk '{print \$1}'"

echo "==> [5/6] 启 zhuanfa-master + 等 7080 ready"
ssh_run "$MASTER_INDEX" "systemctl start zhuanfa-master && sleep 3 && curl -fsS http://127.0.0.1:7080/healthz"

echo "==> [6/6] 启所有 zhuanfa-node"
run_all "systemctl start zhuanfa-node && sleep 2 && systemctl is-active zhuanfa-node"

echo
echo "==> 烟测：master 看节点心跳（15s 内）"
sleep 8
ssh_run "$MASTER_INDEX" "journalctl -u zhuanfa-master --since '15 sec ago' --no-pager | grep heartbeat | awk '{print \$NF}' | sort -u | head"

echo "✅ 部署完成"
