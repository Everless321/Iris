#!/bin/bash
# Iris node 零交互启动脚本 — 装入 golden image 内 /usr/local/bin/iris-bootstrap.sh
#
# 第一次开机：从云厂商 metadata 读 master URL + enrollment token，自动调 install.sh enroll。
# 后续开机：检测 /opt/iris/.enrolled marker 立即退出（已 enroll 过）。
#
# Metadata keys（首先 GCP，未来可加 AWS / Hetzner）：
#   iris-master   : master URL，例 http://23.149.108.114:7080 或 https://iris.example.com
#   iris-token    : enrollment token（一次性，master 生成）
#   iris-extra    : 可选，附加 install.sh 参数（例 "--no-host-alias"）
#
# 失败：写 /var/log/iris-bootstrap.log + 阻止 systemd 进入 active（让 admin 看到）

set -euo pipefail
LOG=/var/log/iris-bootstrap.log
MARKER=/opt/iris/.enrolled

exec > >(tee -a "$LOG") 2>&1
echo "[$(date -Is)] iris-bootstrap start"

if [ -f "$MARKER" ]; then
  echo "已 enroll 过（marker $MARKER 存在），跳过"
  exit 0
fi

# ---- 1) 探测 metadata 来源（V1 仅 GCP）----
MASTER=""
TOKEN=""
EXTRA=""

if curl -fsS --max-time 2 -H "Metadata-Flavor: Google" \
   http://metadata.google.internal/computeMetadata/v1/instance/id >/dev/null 2>&1; then
  echo "云厂商：GCP"
  fetch() {
    curl -fsS --max-time 5 -H "Metadata-Flavor: Google" \
      "http://metadata.google.internal/computeMetadata/v1/instance/attributes/$1" 2>/dev/null || true
  }
  MASTER=$(fetch iris-master)
  TOKEN=$(fetch iris-token)
  EXTRA=$(fetch iris-extra)
else
  echo "未识别云厂商或 metadata 不可达 — 等手动 install.sh"
  exit 0
fi

if [ -z "$MASTER" ] || [ -z "$TOKEN" ]; then
  echo "metadata 缺 iris-master 或 iris-token — 等手动 install.sh"
  exit 0
fi
echo "MASTER=$MASTER  TOKEN=${TOKEN:0:8}...  EXTRA=$EXTRA"

# ---- 2) 调 install.sh enroll（用同 commit / latest 的脚本） ----
INSTALL_SH_URL="${IRIS_INSTALL_SH_URL:-https://raw.githubusercontent.com/Everless321/Iris/main/install.sh}"
echo "拉 install.sh: $INSTALL_SH_URL"

# shellcheck disable=SC2086
curl -fsSL "$INSTALL_SH_URL" | bash -s -- --master "$MASTER" --token "$TOKEN" $EXTRA

# ---- 3) 写 marker ----
touch "$MARKER"
echo "[$(date -Is)] iris-bootstrap done — enrolled，marker 写入 $MARKER"
