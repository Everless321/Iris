# GCP 自定义镜像 + 流量统计跨区测试

## 目标
1. 解决每次开 GCP 节点都要 scp 二进制的痛点：构建 `iris-node-base-v1` 镜像，启动节点 30s 上线
2. 用新镜像跑 3 节点流量统计测试（TCP 单跳 / TCP 双跳 / UDP），验证 #31 在多区/多跳场景下精度

## 设计要点
- 零 master 代码改动（不加 preauth API，复用现有 enroll token 一次性语义）
- GCP metadata 注入 4 变量：`iris-master / iris-enroll-token / iris-node-id / iris-data-addr`
- bootstrap.sh oneshot → enroll → 写文件 → 起 node service
- 镜像存全局（image 是 project-global 资源），节点跨 region 共用

## 待办

### Part 1: 镜像构建
- [ ] 下载最新 musl iris-node 二进制（GitHub Actions run 26731541059）
- [ ] 写 `deploy/gcp/bootstrap.sh`（metadata 读取 + enroll + 启动）
- [ ] 写 `deploy/gcp/iris-bootstrap.service` systemd unit
- [ ] 写 `deploy/gcp/iris-node.service` systemd unit
- [ ] 起 builder VM (debian-12, e2-micro, us-central1-a)
- [ ] 部署文件 + 清理 cloud-init / SSH host keys / machine-id
- [ ] 关机后 `gcloud compute images create iris-node-base-v1`
- [ ] 删除 builder VM
- [ ] 起 `iris-validate` 验证镜像（30s 内上线）

### Part 2: 流量统计测试
- [ ] mint 3 个 enroll token（gcp-iad / gcp-tyo / gcp-fra）
- [ ] 起 3 节点（不同 region），等 enroll 完成
- [ ] 创建 3 个测试 forward：
  - 19001 tcp 单跳 iad→fra:5201
  - 19002 tcp 双跳 iad→tyo→fra:5201
  - 19003 udp 单跳 iad→fra:5201
- [ ] fra 节点起 iperf3 server
- [ ] Case A: iperf3 -t 30 tcp 通过 19001，对账 bytes_in/out
- [ ] Case B: iperf3 -t 30 tcp 通过 19002，验证仅入口 iad 计数（中转 tyo 该 forward_id master 端无累加）
- [ ] Case C: iperf3 -u -b 100M -t 20 通过 19003，对账 bytes_in/out
- [ ] Case D: 观察 web UI 流量列每 5s 增长（截图）
- [ ] 删除 3 测试 forward（保留 hawaii forward_id=33 trf-test 不动）
- [ ] 销毁 3 测试节点

## 安全防线
- ⚠️ 绝不动 `hawaii:18810 trf-test` (forward_id=33)
- 测试 forward 名前缀 `gcp-test-`，便于批量识别
- enroll token 30min 有效 + 一次性
- GCP metadata 仅 instance 内可读

## 结果
（待执行后填）
