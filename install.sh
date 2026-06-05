#!/usr/bin/env bash
# Iris 节点一键安装脚本 V2
#
# 脚本来源（canonical）：https://raw.githubusercontent.com/Everless321/Iris/main/install.sh
# master HTTP `/install.sh` 也提供 302 redirect 兜底，但**主推 GitHub raw**：master 离线也能装新节点。
#
# 安装新节点（首次）：
#   curl -fsSL https://raw.githubusercontent.com/Everless321/Iris/main/install.sh | sudo bash -s -- \
#     --master https://<MASTER_IP> --token <ENROLLMENT_TOKEN>
#
# 升级已有节点（替换 binary + 自动重启）：
#   curl -fsSL https://raw.githubusercontent.com/Everless321/Iris/main/install.sh | sudo bash -s -- --upgrade
#
# 升级 master（替换 iris-master + 重启 iris-master.service）：
#   curl -fsSL https://raw.githubusercontent.com/Everless321/Iris/main/install.sh | sudo bash -s -- --upgrade-master
#
# 首次安装 master（生成 master.env + 起 service）：
#   curl -fsSL https://raw.githubusercontent.com/Everless321/Iris/main/install.sh | sudo bash -s -- --install-master \
#     [--admin-user admin]   # 默认 admin
#     [--admin-pass <pass>]  # 不传则 openssl rand 生成并打印
#     [--jwt-secret <hex>]   # 不传则 openssl rand 生成
#
# 卸载节点（备份 + 移除）：
#   curl -fsSL https://raw.githubusercontent.com/Everless321/Iris/main/install.sh | sudo bash -s -- --uninstall
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
ADMIN_USER=""                     # --install-master 时用，未指定默认 admin
ADMIN_PASS=""                     # --install-master 时用，未指定时随机生成并打印
JWT_SECRET=""                     # --install-master 时用，未指定时随机生成

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
    --install-master) ACTION="install-master"; shift ;;
    --admin-user)     ADMIN_USER="$2"; shift 2 ;;
    --admin-pass)     ADMIN_PASS="$2"; shift 2 ;;
    --jwt-secret)     JWT_SECRET="$2"; shift 2 ;;
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

# ---- kernel tuning (M5.0 conservative): 3 safe sysctls，全节点通用零副作用 ----
write_sysctl_drop_in() {
  cat > /etc/sysctl.d/99-iris.conf <<'SYSCTL'
# Iris node kernel tuning — conservative, 全节点通用
# 解决 fast path 高并发短连接 backlog 溢出（实测 5000 并发成功率 56% → 应 ~100%）
net.core.somaxconn = 65535
net.ipv4.tcp_max_syn_backlog = 8192
SYSCTL
  sysctl --system >/dev/null 2>&1 || sysctl -p /etc/sysctl.d/99-iris.conf >/dev/null 2>&1 || true
}

# ---- M8 watchdog wrapper ----
# 升级后启动时 .upgrade-pending 存在 → 60s 内检测 .heartbeat-state ok,
# 否则回滚到最近 .bak.<ts> + 重启。防止坏 binary 把 agent brick 掉。
write_watchdog() {
  cat > "$INSTALL_DIR/iris-node-watchdog.sh" <<'WD'
#!/bin/sh
# M8 self-heal wrapper —— 升级后看门狗
set -e
INSTALL_DIR=/opt/iris
BIN=$INSTALL_DIR/iris-node
PENDING=$INSTALL_DIR/.upgrade-pending
HB=$INSTALL_DIR/.heartbeat-state

# 启动前若发现 pending 标记 + 没有 heartbeat-state → 上次升级根本没起来,
# 这是 systemd 第 2+ 次拉起的场景；直接回滚。
maybe_rollback_pre_start() {
  [ -f "$PENDING" ] || return 0
  if [ ! -f "$HB" ] || ! grep -q ok "$HB" 2>/dev/null; then
    bak=$(ls -1t "$INSTALL_DIR"/iris-node.bak.* 2>/dev/null | head -1)
    if [ -n "$bak" ] && [ -x "$bak" ]; then
      echo "[watchdog] previous upgrade unhealthy, rolling back to $bak" >&2
      cp -f "$bak" "$BIN" 2>/dev/null || true
      rm -f "$PENDING"
    fi
  fi
}

# 启动后 60s 监控：如还在 pending 但心跳没起来，杀掉 + 回滚
post_start_watch() {
  child_pid="$1"
  [ -f "$PENDING" ] || return 0
  # 清掉旧的 heartbeat-state 让本次升级独立判定
  rm -f "$HB"
  # 等 60s
  i=0
  while [ $i -lt 60 ]; do
    sleep 1
    i=$((i+1))
    if [ -f "$HB" ] && grep -q ok "$HB" 2>/dev/null; then
      echo "[watchdog] upgrade verified healthy after ${i}s" >&2
      rm -f "$PENDING"
      return 0
    fi
    # 子进程意外退出，systemd 会接手 restart，本 wrapper 结束
    kill -0 "$child_pid" 2>/dev/null || return 0
  done
  echo "[watchdog] upgrade unhealthy after 60s, killing + rolling back" >&2
  kill "$child_pid" 2>/dev/null || true
  bak=$(ls -1t "$INSTALL_DIR"/iris-node.bak.* 2>/dev/null | head -1)
  if [ -n "$bak" ] && [ -x "$bak" ]; then
    mv "$BIN" "$INSTALL_DIR/iris-node.failed.$$" 2>/dev/null || true
    cp -f "$bak" "$BIN"
  fi
  rm -f "$PENDING"
  # systemd Restart=always 会重新拉起，这次 binary 是回滚后的旧版
  exit 1
}

maybe_rollback_pre_start
"$BIN" &
PID=$!
post_start_watch "$PID" &
WATCH=$!
wait $PID
EXIT=$?
kill $WATCH 2>/dev/null || true
exit $EXIT
WD
  chmod +x "$INSTALL_DIR/iris-node-watchdog.sh"
}

# ---- systemd unit writer ----
write_systemd_unit() {
  local node_id="$1"
  write_watchdog
  cat > /etc/systemd/system/iris-node.service <<UNIT
[Unit]
Description=Iris Node ($node_id)
Documentation=https://github.com/$RELEASE_REPO
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
EnvironmentFile=$INSTALL_DIR/.env
ExecStart=$INSTALL_DIR/iris-node-watchdog.sh
WorkingDirectory=$INSTALL_DIR
# cert 续签 / M8 升级时节点 std::process::exit(0)，依赖 Restart=always 把它拉起来
Restart=always
RestartSec=3
LimitNOFILE=1048576
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

  # binary 已存在但 certs 为空 / .env 缺失 → 视为"镜像首次启动 + enroll"场景：
  # 跳过下载，直接走 enroll + 写 cert + 写 .env + 起服务。
  # 真正"重复安装"（已有 cert+.env+running service）才报错让用户走 --upgrade。
  local from_image=0
  if [ -x "$INSTALL_DIR/iris-node" ]; then
    if [ -f "$INSTALL_DIR/.env" ] && [ -s "$INSTALL_DIR/certs/client.pem" ] 2>/dev/null; then
      warn "$INSTALL_DIR/iris-node 已存在且已 enroll 过。"
      warn "升级请用：curl ... | sudo bash -s -- --upgrade"
      warn "重装请先：curl ... | sudo bash -s -- --uninstall"
      exit 1
    fi
    info "检测到 baked binary（无 cert/.env） — 拉取最新 release 覆盖，避免使用陈旧二进制"
    # 旧 bug：直接 from_image=1 跳下载 → 重装机器（之前装过节点又 uninstall 没删 binary)
    # 会一直跑几个月前的旧 binary，新字段（node_version / advertised_addr / M8 命令流）
    # 全不发，UI 永远显示离线 + 无版本。统一覆盖一次解决。
    download_binary "$INSTALL_DIR/iris-node"
    from_image=1
  fi

  mkdir -p "$INSTALL_DIR/certs" "$INSTALL_DIR/data"

  info "向 master 兑换证书 ($MASTER) ..."
  # enroll 是 trust-on-first-use 一次性步骤：master 用自签 CA，本机此刻没该 CA。
  # 用 -k 跳过 cert 校验拿 response（含 CA + identity cert + key）；
  # 后续节点 ↔ master gRPC 用 response 里的 CA 严格校验，不再 insecure。
  # MITM 风险：仅限 enrollment 窗口（token 有效期内）+ 仅 1 token，可接受。
  local resp
  resp=$(curl -fsSk -X POST "$MASTER/api/nodes/enroll" \
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

  if [ "$from_image" -eq 0 ]; then
    download_binary "$INSTALL_DIR/iris-node"
  fi

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
    info "应用内核调参 /etc/sysctl.d/99-iris.conf (somaxconn, tcp_max_syn_backlog)"
    write_sysctl_drop_in
    info "安装 systemd 服务 iris-node.service (LimitNOFILE=1048576)"
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

  # M5.0: 升级时一并刷新内核调参 + systemd unit（旧机器需要拿到新 LimitNOFILE / sysctl）
  if command -v systemctl >/dev/null && [ -d /etc/systemd/system ]; then
    info "刷新内核调参 + systemd unit (M5.0)"
    write_sysctl_drop_in
    # 保留旧 node_id：从现有 unit 文件抽
    local cur_id
    cur_id=$(awk -F'[()]' '/^Description=Iris Node/ {print $2; exit}' /etc/systemd/system/iris-node.service 2>/dev/null)
    [ -n "$cur_id" ] && write_systemd_unit "$cur_id"
  fi

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

# ---- action: install master (首次部署) ----
# 装 iris-master 服务：生成 master.env + 装 systemd unit + 起 service + 健康检查。
# admin 密码 / JWT secret 不传时随机生成并明文打印（用户必须立即抄走）。
# 已存在 /opt/iris/iris-master 时拒绝，提示用 --upgrade-master。
do_install_master() {
  if [ -x "$INSTALL_DIR/iris-master" ]; then
    warn "$INSTALL_DIR/iris-master 已存在。"
    warn "升级请用：curl ... | sudo bash -s -- --upgrade-master"
    warn "重装请先：sudo systemctl stop iris-master.service && rm -rf $INSTALL_DIR"
    exit 1
  fi

  # 必备工具
  command -v openssl >/dev/null || die "需要 openssl 生成密钥"

  # 默认 admin user
  [ -z "$ADMIN_USER" ] && ADMIN_USER="admin"

  # 随机生成缺失的密钥（base64 去掉 = 号避免 shell 转义麻烦）
  local generated_pass=0 generated_jwt=0
  if [ -z "$ADMIN_PASS" ]; then
    ADMIN_PASS=$(openssl rand -base64 24 | tr -d '=\n')
    generated_pass=1
  fi
  if [ -z "$JWT_SECRET" ]; then
    JWT_SECRET=$(openssl rand -hex 32)
    generated_jwt=1
  fi
  # JWT_SECRET 长度校验（master 端 warn ≥16 字节，强制 ≥32 字节高熵）
  if [ ${#JWT_SECRET} -lt 32 ]; then
    die "--jwt-secret 至少 32 字节（建议 64 hex 字符）"
  fi

  info "创建 $INSTALL_DIR/{data,certs}"
  mkdir -p "$INSTALL_DIR/data" "$INSTALL_DIR/certs"
  chmod 700 "$INSTALL_DIR/certs"

  info "写入 $INSTALL_DIR/master.env (chmod 600)"
  cat > "$INSTALL_DIR/master.env" <<EOF
IRIS_ADMIN_USER=$ADMIN_USER
IRIS_ADMIN_PASS=$ADMIN_PASS
IRIS_JWT_SECRET=$JWT_SECRET
EOF
  chmod 600 "$INSTALL_DIR/master.env"

  download_binary "$INSTALL_DIR/iris-master" "iris-master"

  if command -v systemctl >/dev/null && [ -d /etc/systemd/system ]; then
    info "安装 systemd 服务 iris-master.service"
    cat > /etc/systemd/system/iris-master.service <<UNIT
[Unit]
Description=Iris Master (control plane + UI)
Documentation=https://github.com/$RELEASE_REPO
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
EnvironmentFile=$INSTALL_DIR/master.env
ExecStart=$INSTALL_DIR/iris-master
WorkingDirectory=$INSTALL_DIR
Restart=always
RestartSec=3
LimitNOFILE=65535
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=multi-user.target
UNIT
    systemctl daemon-reload
    systemctl enable --now iris-master.service
    sleep 3
    if systemctl is-active --quiet iris-master.service; then
      # 健康检查（不强制必须 200，master HTTP listener 还在初始化时可能短暂 connection refused）
      curl -fsS http://127.0.0.1:7080/healthz >/dev/null 2>&1 || sleep 2
      curl -fsS http://127.0.0.1:7080/healthz >/dev/null 2>&1 \
        && ok "master 已上线（/healthz=ok）" \
        || warn "service 已 active 但 /healthz 暂不通，看 journalctl -u iris-master -f"
    else
      die "服务启动失败：journalctl -u iris-master --no-pager -n 30 查错"
    fi
  else
    warn "无 systemd，手动启动：cd $INSTALL_DIR && env \$(cat master.env | xargs) ./iris-master"
  fi

  # 公网 IP / 主机名给用户拼访问 URL
  local pub_ip
  pub_ip=$(curl -fsS --max-time 3 https://ifconfig.me 2>/dev/null || hostname -I 2>/dev/null | awk '{print $1}')
  [ -z "$pub_ip" ] && pub_ip="<this-host>"

  echo
  echo "═══════════════════════════════════════════════════════════════"
  echo "  ✅ master 部署完成 — 请立即抄走以下凭据并妥善保存"
  echo "═══════════════════════════════════════════════════════════════"
  echo "  Web UI:    http://${pub_ip}:7080"
  echo "  Admin:     $ADMIN_USER / $ADMIN_PASS"
  if [ $generated_pass -eq 1 ]; then
    echo "             ↑ 随机生成，本次仅显示一次"
  fi
  echo
  echo "  JWT_SECRET 已写入 $INSTALL_DIR/master.env（chmod 600）"
  if [ $generated_jwt -eq 1 ]; then
    echo "             ↑ 随机生成，灾难恢复时必须保留这个文件！"
  fi
  echo
  echo "  下一步："
  echo "    1. 浏览器登录 → 添加节点 → 复制 enrollment token"
  echo "    2. 节点机器跑："
  echo "         curl -fsSL https://raw.githubusercontent.com/$RELEASE_REPO/main/install.sh | \\"
  echo "           sudo bash -s -- --master http://${pub_ip}:7080 --token <TOKEN>"
  echo "═══════════════════════════════════════════════════════════════"
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
  install-master) do_install_master ;;
  upgrade-master) do_upgrade_master ;;
  uninstall)      do_uninstall ;;
esac
