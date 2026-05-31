# Phase 9a 性能实测（raw_tunnel vs L1 quick wins）

## 环境
- 4× GCP n2-standard-4 asia-east1-a 同 zone（与 Phase 8 完全相同）
- musl static-pie release（GitHub Actions CI）
- 节点 sysctl net.core.{rmem,wmem}_max=4 MB

## TCP 吞吐三连测对比

| 用例 | Phase 0 (老 n2-2) | Phase 8 (n2-4 + L1) | **Phase 9a (raw_tunnel)** | vs P8 |
|---|---|---|---|---|
| Direct 1-stream | 9695 | 9780 | **9790** | line rate |
| Direct 8-stream | 9314 | 9780 | **9780** | line rate |
| **1-hop 1-stream** | 5876 | 7710 | **9190** | **+19%** |
| 1-hop 8-stream | 9314 | 9570 | **9660** | ≈ line rate |
| **2-hop 1-stream** | 2078 | 2060 | **3990** | **+94%** |
| 2-hop 8-stream | 4536 | 5240 | **5830** | +11% |

## UDP 吞吐对比

| 用例 | Phase 8 sender / recv / loss | **Phase 9a sender / recv / loss** | recv Δ |
|---|---|---|---|
| Direct | 3110 / 3110 / 0.13% | 3000 / 3000 / 0.14% | ≈ |
| **1-hop** | 3140 / **1760** / 44% | 2690 / **1940** / 28% | **+10%, 丢包 −16 pp** |
| **2-hop** | 3120 / **700** / 77% | 3020 / **1100** / 63% | **+57%, 丢包 −14 pp** |

## 关键收益归因

- **2-hop 单流 +94%（2.06 → 3.99 Gbps）**：relay 节点是双重 framing 重灾区，
  剥掉 HTTP/2 后中转节点 CPU 释放，TCP 单流接近 4 Gbps，对单流敏感的 CDN
  回源 / 备份场景是质变。
- **1-hop 单流 9.19 Gbps**：从 P0 的 60% 直连占比提到 **94% line rate**，
  剩余 ~6% 是 AES-GCM AEAD 不可省的固定开销。
- **UDP 多跳 +57% 吞吐 + 丢包 77→63%**：UDP 单 src 单 tunnel 的瓶颈是
  framing+AEAD 串行；少一层 framing 让接收端 buffer 不容易爆。

## 留给 Phase 9b (kTLS) 的余量

- 1-hop 单流 9.19 → ~9.5 Gbps（kernel 拷贝 + syscall 省）
- UDP 单 src 还有大空间（kTLS 在 RX 上软件实现也能省 ~20%）
- 真正爆炸场景：未来部署到 Intel E810 / AWS Nitro 这类 HW offload NIC → +5-10x

## 资源花费

- 4× n2-standard-4 跑 ~25 分钟 ≈ **$0.35**
- 累计 (Phase 8 + 9a) ≈ $0.75，远低于 $5 预算

## 清理
- ✅ GCP 4 实例 + zfperf-allow firewall delete
- ✅ master 端 4 gcp-* nodes + perf forwards delete
- ✅ rfchost zfperf-http + /var/www/zfperf 清空
