#!/usr/bin/env bash
# 滚动部署 musl binary 到 master + N nodes。先推 file → restart，避免 stop 失败 → start no-op。
# 用法：
#   scripts/deploy-prod.sh <dir-containing-zhuanfa-master-and-node>
#
# 认证：ed25519 key + 持久化 known_hosts（accept-new）。Task #17 后切到 key-only。
# 节点 sshd 需启用 PubkeyAuthentication=yes（rfchost 用 sshd_config.d/99-zfdeploy.conf）。
#
# 主机清单 $ZF_DEPLOY_HOSTS_FILE（默认 ~/.zhuanfa/hosts.conf），每行：
#   name:ip:port:roles      roles ∈ {master_node, node}
# （历史 5 字段含 password 的格式仍兼容：第 4 字段被忽略，roles 取第 5 字段）
#
# 凭证文件夹（$HOME/.zhuanfa/）由用户自管，不入库：
#   keys/zfdeploy{,.pub}    — ssh-keygen -t ed25519 -f keys/zfdeploy -N ""
#   known_hosts             — 首次 connect 自动写入（accept-new）
set -uo pipefail
cd "$(dirname "$0")/.."

BIN_DIR=${1:-./artifact}
MASTER_BIN="$BIN_DIR/zhuanfa-master"
NODE_BIN="$BIN_DIR/zhuanfa-node"
[ -f "$MASTER_BIN" ] || { echo "缺 $MASTER_BIN"; exit 1; }
[ -f "$NODE_BIN" ]   || { echo "缺 $NODE_BIN"; exit 1; }

HOSTS_FILE=${ZF_DEPLOY_HOSTS_FILE:-$HOME/.zhuanfa/hosts.conf}
KEY=${ZF_DEPLOY_KEY:-$HOME/.zhuanfa/keys/zfdeploy}
KH=${ZF_DEPLOY_KNOWN_HOSTS:-$HOME/.zhuanfa/known_hosts}

[ -f "$HOSTS_FILE" ] || { echo "缺主机清单：$HOSTS_FILE"; exit 1; }
[ -f "$KEY" ] || { echo "缺 SSH 私钥：$KEY (用 ssh-keygen -t ed25519 -f $KEY -N '')"; exit 1; }
mkdir -p "$(dirname "$KH")" && touch "$KH" && chmod 600 "$KH"

# 平行数组（兼容 bash 3.2）
NAMES=(); IPS=(); PORTS=(); ROLES=()
MASTER_INDEX=-1

while IFS= read -r line || [ -n "$line" ]; do
    case "$line" in ''|\#*) continue ;; esac
    # 兼容 4 字段 (name:ip:port:roles) 和 5 字段 (name:ip:port:password:roles)
    fields=()
    IFS=: read -ra fields <<< "$line"
    [ ${#fields[@]} -lt 4 ] && continue
    name=${fields[0]}; ip=${fields[1]}; port=${fields[2]}
    if [ ${#fields[@]} -ge 5 ]; then
        roles=${fields[4]}   # 5 字段：跳过 password
    else
        roles=${fields[3]}
    fi
    NAMES+=("$name"); IPS+=("$ip"); PORTS+=("$port"); ROLES+=("$roles")
    [ "$roles" = "master_node" ] && MASTER_INDEX=$((${#NAMES[@]} - 1))
done < "$HOSTS_FILE"

N=${#NAMES[@]}
[ $N -gt 0 ] || { echo "$HOSTS_FILE 没有有效条目"; exit 1; }
[ $MASTER_INDEX -ge 0 ] || { echo "$HOSTS_FILE 未指定 roles=master_node 的主机"; exit 1; }
echo "==> 读取 $N 台主机；master = ${NAMES[$MASTER_INDEX]}（auth: ed25519 key + known_hosts pinning）"

# /usr/bin/ssh 绕过本机 shell wrapper（zsh 等）
SSH_BIN=/usr/bin/ssh
SCP_BIN=/usr/bin/scp
SSH_OPTS=(-i "$KEY" -o IdentitiesOnly=yes
          -o StrictHostKeyChecking=accept-new -o UserKnownHostsFile="$KH"
          -o PasswordAuthentication=no -o PubkeyAuthentication=yes
          -o ConnectTimeout=10 -o ServerAliveInterval=5)

ssh_run() {
    local i=$1; shift
    "$SSH_BIN" "${SSH_OPTS[@]}" -p "${PORTS[$i]}" "root@${IPS[$i]}" "$@"
}
scp_to() {
    local i=$1 src=$2 dst=$3
    "$SCP_BIN" -O "${SSH_OPTS[@]}" -P "${PORTS[$i]}" "$src" "root@${IPS[$i]}:$dst"
}

run_all() {
    local cmd=$1
    for ((i=0; i<N; i++)); do
        ( echo "    [${NAMES[$i]}] $cmd"; ssh_run "$i" "$cmd" ) &
    done
    wait
}

echo "==> [1/4] 推 node binary 到所有节点"
for ((i=0; i<N; i++)); do
    echo "    [${NAMES[$i]}] scp zhuanfa-node"
    scp_to "$i" "$NODE_BIN" "/opt/zhuanfa/zhuanfa-node.new"
    ssh_run "$i" "chmod +x /opt/zhuanfa/zhuanfa-node.new && mv -f /opt/zhuanfa/zhuanfa-node.new /opt/zhuanfa/zhuanfa-node && md5sum /opt/zhuanfa/zhuanfa-node | awk '{print \$1}'"
done

echo "==> [2/4] 推 master binary 到 ${NAMES[$MASTER_INDEX]}"
scp_to "$MASTER_INDEX" "$MASTER_BIN" "/opt/zhuanfa/zhuanfa-master.new"
ssh_run "$MASTER_INDEX" "chmod +x /opt/zhuanfa/zhuanfa-master.new && mv -f /opt/zhuanfa/zhuanfa-master.new /opt/zhuanfa/zhuanfa-master && md5sum /opt/zhuanfa/zhuanfa-master | awk '{print \$1}'"

echo "==> [3/4] restart zhuanfa-master @ ${NAMES[$MASTER_INDEX]} + 等 7080 ready"
ssh_run "$MASTER_INDEX" "systemctl restart zhuanfa-master && sleep 3 && curl -fsS http://127.0.0.1:7080/healthz"

echo "==> [4/4] restart 所有 zhuanfa-node + 校验跑的是新 binary"
NODE_MD5=$(md5 -q "$NODE_BIN" 2>/dev/null || md5sum "$NODE_BIN" | awk '{print $1}')
echo "    expected node md5: $NODE_MD5"
for ((i=0; i<N; i++)); do
    name=${NAMES[$i]}
    for attempt in 1 2 3; do
        out=$(ssh_run "$i" "systemctl restart zhuanfa-node && sleep 3 && pid=\$(pgrep -x zhuanfa-node | head -1) && md5sum /proc/\$pid/exe | awk '{print \$1}'" 2>&1 | tail -1)
        if [ "$out" = "$NODE_MD5" ]; then
            echo "    ✅ $name → $out"
            break
        fi
        echo "    ⚠️  $name attempt $attempt got: $out  retry 5s..."
        sleep 5
    done
done

echo
echo "==> 烟测：master 看节点心跳"
sleep 8
ssh_run "$MASTER_INDEX" "journalctl -u zhuanfa-master --since '15 sec ago' --no-pager | grep -oE 'heartbeat node=\S+' | sort -u"

echo "✅ 部署完成"
