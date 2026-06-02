# Iris node golden image — 零交互上线

GCP custom image，开机从 metadata 自动 enroll。第二台+节点不再需要 SSH 进去跑 install.sh。

## 文件

| 文件 | 用途 |
|---|---|
| `iris-bootstrap.sh` | 首次开机脚本：读 metadata → 调 install.sh enroll → 写 marker |
| `iris-bootstrap.service` | systemd oneshot，开机跑一次（marker 存在则 skip） |
| `bake-image.sh` | 从一台运行中的 GCP 实例烤镜像 |

## 烤镜像

```bash
./bake-image.sh iris-gcp-tokyo-3 asia-northeast1-b iris-node-v2
```

实例需要：
- 已经 install.sh 装过（有 binary + systemd）
- 不在跑生产流量（脚本会清 certs / 重启）

## 用镜像起新节点

### 1. 在 master 上生成 enrollment token

```bash
TOKEN=$(curl -fsS -X POST http://127.0.0.1:7080/api/auth/login \
  -H "content-type: application/json" \
  -d '{"username":"admin","password":"..."}' \
  | jq -r .token)

curl -X POST http://127.0.0.1:7080/api/nodes \
  -H "content-type: application/json" \
  -H "authorization: Bearer $TOKEN" \
  -d '{"id":"iris-tokyo-5","name":"Tokyo 5","addr":"0.0.0.0:7444","weight":1}'

NEW_TOKEN=$(curl -X POST http://127.0.0.1:7080/api/nodes/iris-tokyo-5/enrollment \
  -H "authorization: Bearer $TOKEN" | jq -r .token)
```

### 2. 启实例 + 注入 metadata

```bash
gcloud compute instances create iris-tokyo-5 \
  --zone=asia-northeast1-b \
  --machine-type=t2d-standard-4 \
  --image=iris-node-v2 \
  --image-project=$(gcloud config get-value project) \
  --metadata="iris-master=http://23.149.108.114:7080,iris-token=$NEW_TOKEN"
```

开机后 ~15 秒自动 enroll，无需 SSH。

## metadata key 清单

| key | 必需 | 用途 |
|---|---|---|
| `iris-master` | ✅ | master URL，例 `http://...:7080` 或 `https://iris.example.com` |
| `iris-token` | ✅ | enrollment token（一次性） |
| `iris-extra` | | install.sh 附加参数，例 `--no-host-alias` |

## 调试

```bash
# 看 bootstrap 日志
gcloud compute ssh iris-tokyo-5 -- sudo cat /var/log/iris-bootstrap.log

# 强制重 enroll（删 marker → 重启）
gcloud compute ssh iris-tokyo-5 -- "sudo rm -f /opt/iris/.enrolled && sudo systemctl restart iris-bootstrap"
```

## 失败处理

bootstrap 失败不阻止 ssh 登录 — 进去看 `/var/log/iris-bootstrap.log`，手动跑 install.sh 兜底。

## 未来扩展（V2）

- AWS 支持：探测 `http://169.254.169.254/latest/meta-data/iam/info`
- Hetzner Cloud 支持：探测 `http://169.254.169.254/metadata/v1.json`
- master URL 写到 image build 里（避免每台都要传 metadata）；token 仍保留 metadata
