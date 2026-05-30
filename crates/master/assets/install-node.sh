#!/usr/bin/env bash
# Zhuanfa 节点一键安装脚本。
#   curl -fsSL <MASTER>/install.sh | bash -s -- \
#     --master https://<MASTER_HTTP> \
#     --token <ENROLLMENT_TOKEN> \
#     [--binary /path/to/zhuanfa-node] \
#     [--install-dir /opt/zhuanfa] \
#     [--data-addr 0.0.0.0:7444]
set -euo pipefail

MASTER=""
TOKEN=""
BINARY=""
INSTALL_DIR="/opt/zhuanfa"
DATA_ADDR=""

while [ $# -gt 0 ]; do
  case "$1" in
    --master)      MASTER="$2"; shift 2 ;;
    --token)       TOKEN="$2"; shift 2 ;;
    --binary)      BINARY="$2"; shift 2 ;;
    --install-dir) INSTALL_DIR="$2"; shift 2 ;;
    --data-addr)   DATA_ADDR="$2"; shift 2 ;;
    *) echo "未知参数: $1" >&2; exit 1 ;;
  esac
done

[ -z "$MASTER" ] && { echo "缺少 --master <http url>" >&2; exit 1; }
[ -z "$TOKEN" ]  && { echo "缺少 --token <enrollment token>" >&2; exit 1; }

command -v curl >/dev/null || { echo "需要 curl" >&2; exit 1; }
command -v python3 >/dev/null || command -v jq >/dev/null || {
  echo "需要 python3 或 jq 解析 JSON" >&2; exit 1; }

echo "==> 向 master 兑换证书 ($MASTER)..."
RESP=$(curl -fsS -X POST "$MASTER/api/nodes/enroll" \
  -H 'content-type: application/json' \
  -d "{\"token\":\"$TOKEN\"}")

# 优先 jq，否则 python3
parse() {
  if command -v jq >/dev/null; then
    echo "$RESP" | jq -r ".$1"
  else
    echo "$RESP" | python3 -c "import json,sys; print(json.load(sys.stdin)['$1'])"
  fi
}

NODE_ID=$(parse node_id)
MASTER_GRPC=$(parse master_grpc)
HINT_ADDR=$(parse data_addr_hint)
[ -z "$DATA_ADDR" ] && DATA_ADDR="$HINT_ADDR"
echo "    节点 ID: $NODE_ID"
echo "    控制面: $MASTER_GRPC"
echo "    数据面监听: $DATA_ADDR"

CERT_DIR="$INSTALL_DIR/certs"
echo "==> 写入证书到 $CERT_DIR"
mkdir -p "$CERT_DIR" "$INSTALL_DIR/data"
# 用 base64 中转避免 shell 转义问题
for f in ca_pem cert_pem key_pem; do
  case "$f" in
    ca_pem)   OUT="ca.pem" ;;
    cert_pem) OUT="client.pem" ;;
    key_pem)  OUT="client-key.pem" ;;
  esac
  parse "$f" > "$CERT_DIR/$OUT"
  chmod 600 "$CERT_DIR/$OUT"
done

# 二进制：如果没显式提供，期望已在 PATH 或 INSTALL_DIR
if [ -n "$BINARY" ]; then
  echo "==> 复制二进制 $BINARY → $INSTALL_DIR/zhuanfa-node"
  cp "$BINARY" "$INSTALL_DIR/zhuanfa-node"
  chmod +x "$INSTALL_DIR/zhuanfa-node"
elif [ -x "$INSTALL_DIR/zhuanfa-node" ]; then
  echo "==> 检测到已存在 $INSTALL_DIR/zhuanfa-node"
else
  echo "⚠️  未提供 --binary，且 $INSTALL_DIR/zhuanfa-node 不存在"
  echo "    请先把 zhuanfa-node 二进制放到 $INSTALL_DIR/，或重跑时加 --binary <路径>"
  exit 1
fi

# 写一个简单的 env 文件
cat > "$INSTALL_DIR/.env" <<EOF
ZF_NODE_ID=$NODE_ID
ZF_DATA_ADDR=$DATA_ADDR
ZF_CERT_DIR=$CERT_DIR
ZF_MASTER=$MASTER_GRPC
EOF
chmod 600 "$INSTALL_DIR/.env"

# 检测 systemd 写 unit；否则给出手动启动命令
if command -v systemctl >/dev/null && [ -d /etc/systemd/system ]; then
  echo "==> 安装 systemd 服务 zhuanfa-node.service"
  cat > /etc/systemd/system/zhuanfa-node.service <<UNIT
[Unit]
Description=Zhuanfa Node ($NODE_ID)
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
EnvironmentFile=$INSTALL_DIR/.env
ExecStart=$INSTALL_DIR/zhuanfa-node
Restart=always
RestartSec=3
WorkingDirectory=$INSTALL_DIR

[Install]
WantedBy=multi-user.target
UNIT
  systemctl daemon-reload
  systemctl enable --now zhuanfa-node
  echo "✅ 已启动。journalctl -u zhuanfa-node -f 看日志"
else
  echo "==> 无 systemd，手动启动："
  echo "    cd $INSTALL_DIR && env \$(cat .env | xargs) ./zhuanfa-node"
fi
