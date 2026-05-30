# 性能基线测试（GCP asia-east1，已销毁）

## 环境
- 临时申请 2× GCP e2-standard-2（2 vCPU / 8GB RAM）asia-east1-a 同 zone
- `zhuanfa-perf-a`：作为 zhuanfa 节点加入现有 master
- `zhuanfa-perf-b`：作为 iperf3 server + 负载源
- 现有 master + 4 节点（nosla-hk, rfchost, hawaii, att）
- 全部测试完销毁，账单 ≈ $0.09

## 关键结果

### 单流吞吐
| 路径 | 吞吐 (Mbps) | 瓶颈 |
|---|---|---|
| Direct perf-b → perf-a (intra-zone) | **4024 / 4019** | GCP e2-standard-2 内网上限 |
| 1-hop zhuanfa | **3989 / 3960** | 同上（差距 < 2% 噪声）|
| 2-hop 经 nosla（100M） | 95 / 90 | **nosla 网络出口 100Mbps 硬上限** |

**结论：mTLS + gRPC + Rust 实现的码层吞吐开销小于 1%，4 Gbps 单流不是 CPU/framing 瓶颈。**

### 并发建链
| 项 | 数 |
|---|---|
| 500 并发 TCP 连接建立成功率 | 100% |
| 建链速率 | 459 conn/s |
| 500 连接挂着的额外内存 | ~30 MB（< 60 KB/连接）|

### 健康探针延迟（master 视角，跨大洲）
- nosla-hk → gcp-perf-a: 42ms（HK ↔ TW）
- master → hawaii: 183ms
- master → att: 164ms

## 验证的关键假设
1. ✅ 单跳代码开销 ≈ 0
2. ✅ 多跳吞吐瓶颈是网络，不是软件
3. ✅ mTLS 握手不影响连接建立速率
4. ✅ 每连接内存占用极低（< 60KB），节点可支持成千上万并发

## 未测的
- 单节点 CPU 满载时的极限吞吐（4Gbps 还没把 e2-standard-2 打满，需要 N2 系列或 16Gbps 网卡）
- 10k 级并发的真实容量上限
- 故障切换（kill 中间节点）的真实切走时间
- 小包 RTT 抖动（nping/socat ping 未装到位）
- 长时段稳定性（>1 hr）

## 工具
- iperf3 3.12（apt 装）
- Python 3 socket 并发 fixture（rolled in-line）
- `/api/forwards/test` 自带探针
- 直接 ssh + gcloud cli

## 成本
$0.09（2× e2-standard-2 在线 40 分钟）。完整 4 小时压测预算 < $1。
