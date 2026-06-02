#!/bin/bash
# 烤制 Iris node golden image — GCP 版本。
#
# 输入：一个已经装好 iris-node binary + systemd unit 的运行实例（通常用 install.sh enroll 后即可）。
# 步骤：
#   1) SSH 进实例，注入 iris-bootstrap.sh + service
#   2) 清理 enroll 产物（certs / .env）让镜像可被重新 enroll
#   3) 停实例，从 boot disk 烤镜像
#   4) （可选）重启实例
#
# 用法：
#   bake-image.sh <source-instance> <source-zone> <new-image-name>
#
# 例：
#   ./bake-image.sh iris-gcp-tokyo-3 asia-northeast1-b iris-node-v2

set -euo pipefail
INSTANCE="${1:?用法：bake-image.sh <instance> <zone> <image-name>}"
ZONE="${2:?missing zone}"
IMAGE="${3:?missing image name}"
HERE=$(cd "$(dirname "$0")" && pwd)

echo "==> [1/4] 注入 bootstrap 脚本 + service 到 $INSTANCE"
gcloud compute scp \
  "$HERE/iris-bootstrap.sh" \
  "$HERE/iris-bootstrap.service" \
  "$INSTANCE:/tmp/" --zone="$ZONE"

gcloud compute ssh "$INSTANCE" --zone="$ZONE" --command='
sudo install -m 0755 /tmp/iris-bootstrap.sh /usr/local/bin/iris-bootstrap.sh
sudo install -m 0644 /tmp/iris-bootstrap.service /etc/systemd/system/iris-bootstrap.service
sudo systemctl daemon-reload
sudo systemctl enable iris-bootstrap.service
echo bootstrap 安装完成
'

echo "==> [2/4] 清理 enroll 产物（cert / .env / marker / 历史）"
gcloud compute ssh "$INSTANCE" --zone="$ZONE" --command='
sudo systemctl stop iris-node || true
sudo systemctl disable iris-node || true
sudo rm -rf /opt/iris/certs /opt/iris/data /opt/iris/.env /opt/iris/.enrolled
sudo sed -i "/iris-master/d" /etc/hosts
sudo find /var/log -name "auth.log*" -o -name "syslog*" -o -name "cloud-init*" -o -name "unattended*" 2>/dev/null | xargs -r sudo truncate -s 0 || true
sudo rm -rf /root/.bash_history /home/*/.bash_history /tmp/* /var/tmp/*
echo "镜像 sterilize 完成"
'

echo "==> [3/4] 停实例 + 烤镜像 $IMAGE"
gcloud compute instances stop "$INSTANCE" --zone="$ZONE"
gcloud compute images create "$IMAGE" \
  --source-disk="$INSTANCE" \
  --source-disk-zone="$ZONE" \
  --family=iris-node \
  --description="Iris node golden image (with auto-enroll bootstrap). 从 metadata iris-master + iris-token 自动 enroll。"

echo "==> [4/4] 镜像 $IMAGE 烤完。重启 $INSTANCE..."
gcloud compute instances start "$INSTANCE" --zone="$ZONE"

cat <<EOF

✅ 镜像 $IMAGE 完成。

零交互启动新节点：
  gcloud compute instances create iris-tokyo-X \\
    --zone=$ZONE \\
    --machine-type=t2d-standard-4 \\
    --image=$IMAGE \\
    --image-project=\$(gcloud config get-value project) \\
    --metadata=iris-master=http://23.149.108.114:7080,iris-token=<token>

新机开机后 ~15s 自动 enroll，无需 SSH 进去跑 install.sh。
失败日志：/var/log/iris-bootstrap.log
EOF
