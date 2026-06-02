#!/usr/bin/env bash
# Iris 节点一键安装脚本 V2
#
# 安装新节点（首次）：
#   curl -fsSL <MASTER>/install.sh | sudo bash -s -- \
#     --master https://<MASTER> --token <ENROLLMENT_TOKEN>
#
# 升级已有节点（替换 binary + 自动重启）：
#   curl -fsSL <MASTER>/install.sh | sudo bash -s -- --upgrade
#
# 升级 master（替换 iris-master + 重启 iris-master.service）：
#   curl -fsSL <MASTER>/install.sh | sudo bash -s -- --upgrade-master
#
# 卸载节点（备份 + 移除）：
#   curl -fsSL <MASTER>/install.sh | sudo bash -s -- --uninstall
#
# 可选：
#   --binary <path>        本地二进制（跳过下载）
#   --binary-url <url>     自定义下载 URL
#   --release-tag <tag>    指定 GitHub Release（默认 latest）
#   --install-dir <dir>    安装目录（默认 /opt/iris）
#   --data-addr <addr>     数据面监听地址（默认从 enroll API 推断）
#   --master-host <name>   /etc/hosts 别名（默认 iris-master）。换 master IP 只需改 /etc/hosts 一行。
#   --no-host-alias        关闭 hosts 别名机制，.env 直接写 --master 的 host
#
set -euo pipefail

# ---- defaults ----
RELEASE_REPO="${IRIS_RELEASE_REPO:-Everless321/Iris}"
RELEASE_TAG="latest"
INSTALL_DIR="/opt/iris"
DATA_ADDR=""
MASTER=""
TOKEN=""
BINARY=""
BINARY_URL=""
ACTION="install"
MASTER_HOST_ALIAS="iris-master"   # /etc/hosts 别名；--master 是 IP 时自动启用
USE_HOST_ALIAS="auto"              # auto | always | never

# ---- parse args ----
while [ $# -gt 0 ]; do
  case "$1" in
    --master)       MASTER="$2"; shift 2 ;;
    --token)        TOKEN="$2"; shift 2 ;;
    --binary)       BINARY="$2"; shift 2 ;;
    --binary-url)   BINARY_URL="$2"; shift 2 ;;
    --release-tag)  RELEASE_TAG="$2"; shift 2 ;;
    --install-dir)  INSTALL_DIR="$2"; shift 2 ;;
    --data-addr)    DATA_ADDR="$2"; shift 2 ;;
    --master-host)  MASTER_HOST_ALIAS="$2"; USE_HOST_ALIAS="always"; shift 2 ;;
    --no-host-alias) USE_HOST_ALIAS="never"; shift ;;
    --upgrade)        ACTION="upgrade"; shift ;;
    --upgrade-master) ACTION="upgrade-master"; shift ;;
    --uninstall)      ACTION="uninstall"; shift ;;
    --help|-h)      sed -n '1,/^set/p' "$0" | head -n -1 | sed 's/^# \?//'; exit 0 ;;
    *)              echo "未知参数: $1" >&2; exit 1 ;;
  esac
done

# ---- helpers ----
die()  { echo "❌ $*" >&2; exit 1; }
info() { echo "==> $*"; }
ok()   { echo "✅ $*"; }
warn() { echo "⚠️  $*" >&2; }

# ---- root check (install/uninstall need /etc/systemd + /opt/iris) ----
if [ "$(id -u)" -ne 0 ]; then
  die "需要 root 权限：请用 sudo 运行（例如 curl ... | sudo bash -s -- ...）"
fi

# ---- arch detection ----
ARCH=$(uname -m)
case "$ARCH" in
  x86_64|amd64) ARCH_TAG="musl-x86_64" ;;
  *) die "暂不支持架构: $ARCH（目前仅 x86_64；aarch64 待支持）" ;;
esac

# ---- deps ----
command -v curl >/dev/null || die "需要 curl"
command -v tar  >/dev/null || die "需要 tar"
JSON_PARSER=""
if command -v jq >/dev/null;      then JSON_PARSER="jq"
elif command -v python3 >/dev/null; then JSON_PARSER="python3"
else die "需要 jq 或 python3 解析 JSON"
fi
parse_json() {
  if [ "$JSON_PARSER" = "jq" ]; then
    jq -r ".$1"
  else
    python3 -c "import json,sys; print(json.load(sys.stdin)['$1'])"
  fi
}

# ---- host alias for master URL ----
# 检测字符串是否为 IPv4 / IPv6 字面量（粗粒度，够区分常见 case）。
is_ip_literal() {
  case "$1" in
    *[a-zA-Z]*) return 1 ;;                              # 含字母 → 域名
    [0-9]*.[0-9]*.[0-9]*.[0-9]*) return 0 ;;            # IPv4
    \[*\]) return 0 ;;                                   # 带方括号的 IPv6
    *:*) return 0 ;;                                     # 含冒号且无字母 → IPv6
    *) return 1 ;;
  esac
}

# 从 --master 提取 host（去 scheme、去端口）。失败返回空。
extract_master_host() {
  local url="$1"
  echo "$url" | sed -E 's|^https?://||; s|/.*$||; s|:[0-9]+$||'
}

# 写入 /etc/hosts pin 行（idempotent，删旧行再写新行）。
pin_master_host() {
  local ip="$1"
  local alias="$2"
  local marker="# iris-master alias (managed by install.sh) — change IP here to migrate master"
  if [ ! -w /etc/hosts ]; then die "/etc/hosts 不可写"; fi
  # 删除旧 pin 行（以 marker 标记或别名标记）
  sed -i.bak -E "/${marker//\//\\/}/d" /etc/hosts 2>/dev/null || true
  sed -i.bak -E "/[[:space:]]${alias}\$/d; /[[:space:]]${alias}[[:space:]]/d" /etc/hosts 2>/dev/null || true
  rm -f /etc/hosts.bak
  printf "%s\n%s %s\n" "$marker" "$ip" "$alias" >> /etc/hosts
}

# 撤销 /etc/hosts pin 行（uninstall 用）。
unpin_master_host() {
  local alias="${1:-iris-master}"
  local marker="# iris-master alias (managed by install.sh) — change IP here to migrate master"
  [ -w /etc/hosts ] || return 0
  sed -i.bak -E "/${marker//\//\\/}/d" /etc/hosts 2>/dev/null || true
  sed -i.bak -E "/[[:space:]]${alias}\$/d; /[[:space:]]${alias}[[:space:]]/d" /etc/hosts 2>/dev/null || true
  rm -f /etc/hosts.bak
}

# ---- binary source resolution ----
resolve_binary_url() {
  local bin="${1:-iris-node}"
  if [ -n "$BINARY_URL" ]; then
    echo "$BINARY_URL"
  elif [ "$RELEASE_TAG" = "latest" ]; then
    echo "https://github.com/${RELEASE_REPO}/releases/latest/download/${bin}-${ARCH_TAG}"
  else
    echo "https://github.com/${RELEASE_REPO}/releases/download/${RELEASE_TAG}/${bin}-${ARCH_TAG}"
  fi
}

download_binary() {
  local target="$1"
  local bin="${2:-iris-node}"
  if [ -n "$BINARY" ]; then
    info "复制本地二进制 $BINARY → $target"
    cp "$BINARY" "$target"
  else
    local url
    url=$(resolve_binary_url "$bin")
    info "下载二进制：$url"
    if ! curl -fsSL --retry 3 --retry-delay 2 -o "$target" "$url"; then
      rm -f "$target"
      die "下载失败。如尚未发布 GitHub Release，请：(1) --binary <path> 用本地文件；(2) --binary-url <url> 自定义；(3) 联系维护者发布 release。"
    fi
  fi
  [ -s "$target" ] || die "下载的二进制为空：$target"
  chmod 755 "$target"
}

# ---- systemd unit writer ----
write_systemd_unit() {
  local node_id="$1"
  cat > /etc/systemd/system/iris-node.service <<UNIT
[Unit]
Description=Iris Node ($node_id)
Documentation=https://github.com/$RELEASE_REPO
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
EnvironmentFile=$INSTALL_DIR/.env
ExecStart=$INSTALL_DIR/iris-node
WorkingDirectory=$INSTALL_DIR
# cert 续签时节点 std::process::exit(0)，依赖 Restart=always 把它拉起来
Restart=always
RestartSec=3
LimitNOFILE=65535
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=multi-user.target
UNIT
  systemctl daemon-reload
}

# ---- action: install ----
do_install() {
  [ -z "$MASTER" ] && die "缺少 --master <http(s) url>"
  [ -z "$TOKEN" ]  && die "缺少 --token <enrollment token>"

  if [ -x "$INSTALL_DIR/iris-node" ]; then
    warn "$INSTALL_DIR/iris-node 已存在。"
    warn "升级请用：curl ... | sudo bash -s -- --upgrade"
    warn "重装请先：curl ... | sudo bash -s -- --uninstall"
    exit 1
  fi

  mkdir -p "$INSTALL_DIR/certs" "$INSTALL_DIR/data"

  info "向 master 兑换证书 ($MASTER) ..."
  local resp
  resp=$(curl -fsS -X POST "$MASTER/api/nodes/enroll" \
    -H 'content-type: application/json' \
    -d "{\"token\":\"$TOKEN\"}") \
    || die "enroll API 失败（token 已过期/已用/无效？或网络不通）"

  local node_id hint_addr
  node_id=$(echo "$resp"   | parse_json node_id)
  hint_addr=$(echo "$resp" | parse_json data_addr_hint)
  [ -z "$DATA_ADDR" ] && DATA_ADDR="$hint_addr"

  # 推导 master gRPC URL。enroll API 返回的 master_grpc 字段依赖 master 端
  # IRIS_PUBLIC_GRPC env，未设时默认 127.0.0.1:7443，不可用。这里强制从 --master 参数
  # 推导：取 host，加端口 7443（master gRPC 监听固定 7443）。
  local master_host master_grpc env_host
  master_host=$(extract_master_host "$MASTER")

  # Host 别名机制（默认 auto）：--master 是 IP 字面量 → 写 /etc/hosts 别名 + .env 用 hostname。
  # 未来 master 换 IP 只改 /etc/hosts 一行，无需 systemctl restart（gRPC client 自动重连）。
  # 域名直连不走 hosts 别名（DNS 已经是间接层）。
  env_host="$master_host"
  case "$USE_HOST_ALIAS" in
    always)
      pin_master_host "$master_host" "$MASTER_HOST_ALIAS"
      env_host="$MASTER_HOST_ALIAS"
      info "/etc/hosts pinned: $master_host → $MASTER_HOST_ALIAS（换 master IP 只改这一行）"
      ;;
    never)
      info "禁用 hosts 别名（.env 直接用 --master 的 host = $master_host）"
      ;;
    auto)
      if is_ip_literal "$master_host"; then
        pin_master_host "$master_host" "$MASTER_HOST_ALIAS"
        env_host="$MASTER_HOST_ALIAS"
        info "/etc/hosts pinned: $master_host → $MASTER_HOST_ALIAS（换 master IP 只改这一行）"
      else
        info "--master 是域名（$master_host），跳过 hosts 别名（DNS 已是间接层）"
      fi
      ;;
  esac
  master_grpc="https://${env_host}:7443"

  echo "    节点 ID: $node_id"
  echo "    控制面: $master_grpc"
  echo "    数据面监听: $DATA_ADDR"

  info "写入证书 → $INSTALL_DIR/certs/"
  echo "$resp" | parse_json ca_pem   > "$INSTALL_DIR/certs/ca.pem"
  echo "$resp" | parse_json cert_pem > "$INSTALL_DIR/certs/client.pem"
  echo "$resp" | parse_json key_pem  > "$INSTALL_DIR/certs/client-key.pem"
  chmod 600 "$INSTALL_DIR/certs/ca.pem" "$INSTALL_DIR/certs/client.pem" "$INSTALL_DIR/certs/client-key.pem"
  chmod 700 "$INSTALL_DIR/certs"

  download_binary "$INSTALL_DIR/iris-node"

  info "写入 $INSTALL_DIR/.env"
  cat > "$INSTALL_DIR/.env" <<EOF
IRIS_NODE_ID=$node_id
IRIS_DATA_ADDR=$DATA_ADDR
IRIS_CERT_DIR=$INSTALL_DIR/certs
IRIS_MASTER=$master_grpc
RUST_LOG=info
EOF
  chmod 600 "$INSTALL_DIR/.env"

  if command -v systemctl >/dev/null && [ -d /etc/systemd/system ]; then
    info "安装 systemd 服务 iris-node.service"
    write_systemd_unit "$node_id"
    systemctl enable --now iris-node.service
    sleep 2
    if systemctl is-active --quiet iris-node.service; then
      ok "节点已上线。journalctl -u iris-node -f 看实时日志"
      ok "Cert 1 年到期前 30 天自动续签（IRIS_AUTO_RENEW=0 可关闭）"
    else
      die "服务启动失败：journalctl -u iris-node --no-pager -n 30 查错"
    fi
  else
    warn "无 systemd，手动启动："
    echo "    cd $INSTALL_DIR && env \$(cat .env | xargs) ./iris-node"
  fi
}

# ---- action: upgrade ----
do_upgrade() {
  [ -x "$INSTALL_DIR/iris-node" ] \
    || die "未检测到已安装节点（$INSTALL_DIR/iris-node 不存在）。先做首次安装"

  local ts backup
  ts=$(date +%s)
  backup="$INSTALL_DIR/iris-node.bak.$ts"
  info "备份当前二进制 → $backup"
  cp "$INSTALL_DIR/iris-node" "$backup"

  download_binary "$INSTALL_DIR/iris-node.new"
  mv "$INSTALL_DIR/iris-node.new" "$INSTALL_DIR/iris-node"

  if command -v systemctl >/dev/null && systemctl is-active --quiet iris-node.service; then
    info "systemctl restart iris-node.service"
    systemctl restart iris-node.service
    sleep 2
    if systemctl is-active --quiet iris-node.service; then
      ok "升级完成"
    else
      warn "重启后服务异常，回滚二进制"
      mv "$backup" "$INSTALL_DIR/iris-node"
      systemctl restart iris-node.service
      die "已回滚。journalctl -u iris-node --no-pager -n 30 查错"
    fi
  else
    warn "服务未运行，已替换二进制。手动启动 systemctl start iris-node"
  fi
}

# ---- action: upgrade master ----
# 比 node upgrade 严格：master 是控制面单点,失败回滚必须成功。
# 用 stop → swap → start（而不是 restart）以避免 mv 替换被运行中的 binary 锁住。
do_upgrade_master() {
  [ -x "$INSTALL_DIR/iris-master" ] \
    || die "未检测到 master（$INSTALL_DIR/iris-master 不存在）"

  local ts backup
  ts=$(date +%s)
  backup="$INSTALL_DIR/iris-master.bak.$ts"
  info "备份当前 master → $backup"
  cp "$INSTALL_DIR/iris-master" "$backup"

  download_binary "$INSTALL_DIR/iris-master.new" "iris-master"

  if command -v systemctl >/dev/null && systemctl list-unit-files iris-master.service >/dev/null 2>&1; then
    info "stop iris-master.service"
    systemctl stop iris-master.service
    mv "$INSTALL_DIR/iris-master.new" "$INSTALL_DIR/iris-master"
    info "start iris-master.service"
    systemctl start iris-master.service
    sleep 3
    if systemctl is-active --quiet iris-master.service; then
      ok "master 升级完成"
    else
      warn "启动失败，回滚到 $backup"
      systemctl stop iris-master.service 2>/dev/null || true
      mv "$backup" "$INSTALL_DIR/iris-master"
      systemctl start iris-master.service
      die "已回滚。journalctl -u iris-master --no-pager -n 30 查错"
    fi
  else
    warn "无 systemd 或服务未注册，仅替换二进制：$INSTALL_DIR/iris-master"
    mv "$INSTALL_DIR/iris-master.new" "$INSTALL_DIR/iris-master"
  fi
}

# ---- action: uninstall ----
do_uninstall() {
  if [ ! -d "$INSTALL_DIR" ]; then
    warn "$INSTALL_DIR 不存在，无需卸载"
    exit 0
  fi

  if command -v systemctl >/dev/null && systemctl list-unit-files iris-node.service >/dev/null 2>&1; then
    info "停 + 禁用 systemd 服务"
    systemctl stop iris-node.service 2>/dev/null || true
    systemctl disable iris-node.service 2>/dev/null || true
    rm -f /etc/systemd/system/iris-node.service
    systemctl daemon-reload
  fi

  local ts backup
  ts=$(date +%Y%m%d-%H%M%S)
  backup="/tmp/iris-uninstall-$ts.tar.gz"
  info "打包 $INSTALL_DIR → $backup（含 cert + 配置，保留备查）"
  tar -czf "$backup" -C "$(dirname "$INSTALL_DIR")" "$(basename "$INSTALL_DIR")" 2>/dev/null \
    || warn "打包失败但继续卸载"

  info "删除 $INSTALL_DIR"
  rm -rf "$INSTALL_DIR"

  info "撤销 /etc/hosts iris-master 别名（若有）"
  unpin_master_host "$MASTER_HOST_ALIAS"

  ok "卸载完成。备份保留在 $backup（如确认无需，可手动 rm）"
}

# ---- dispatch ----
case "$ACTION" in
  install)        do_install ;;
  upgrade)        do_upgrade ;;
  upgrade-master) do_upgrade_master ;;
  uninstall)      do_uninstall ;;
esac
