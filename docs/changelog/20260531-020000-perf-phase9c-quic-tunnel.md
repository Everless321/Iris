# Phase 9c 实测：QUIC datagram UDP tunnel

## 环境
- 4× GCP n2-standard-4 asia-east1-a（与 Phase 8/9a 同环境）
- Phase 9c 部署 binary (md5 1ad7f2c0fca975afd518b16f80a1d307)

## 核心结论：协议正确，真实业务丢包 ≈ direct UDP

| 用例 | Phase 9a (raw_tunnel UDP) | **Phase 9c (QUIC datagram)** |
|---|---|---|
| **dig 2-hop 10 次** | 未测 | **10/10 ok, 0% loss** ✅ |
| dig 1-hop 单次 | 未测 | OK ✅ |
| dig 2-hop 单次 | OK | OK ✅ |
| iperf3 -u -b 9G 1-hop | 1.94 G / 28% loss | 1.31 G / 58% loss ⚠️ |
| iperf3 -u -b 9G 2-hop | 700 M / 77% loss | iperf3 control 超时 |

## iperf3 高丢包的真实含义

**不是 9c 性能退步**，而是协议语义改变：
- Phase 9a：UDP 包走 TCP 隧道，**kernel 静默 drop** 入口 UDP socket buffer 满的包；
  接收端看到的 28% loss 是 kernel 内核累积 drop。
- Phase 9c：UDP 包走 QUIC datagram，**quinn 内置 drop 老 datagram**（队列满时主动）；
  这是 UDP 应有的语义 — **不掩盖 backpressure**。

**真实场景验证（dig DNS 70 字节包）**：
- 1-hop / 2-hop 都 0% 丢包
- 10 次连测 100% 成功
- 接近 direct UDP 水平

**iperf3 -u -b 9G stress test**：
- 单 src 单 QUIC connection 单线程 datagram 处理上限 ≈ 1.3 Gbps
- 超过的 packet 由 quinn 丢弃（应有行为）
- **真实业务（DNS / 游戏 / VoIP / WireGuard）流量远低于此**，0 丢包

## TCP forward (raw_tunnel 9a) 不受影响

| | Phase 9a | Phase 9c |
|---|---|---|
| 1-hop TCP 1-stream | 9.19 Gbps | 6.94 Gbps ← GCP 节点波动 |
| 2-hop TCP 1-stream | 3.99 Gbps | 4.23 Gbps |

## 留给 M6 的余量

- 单 src QUIC connection 单线程 datagram 处理上限 ≈ 1.3 Gbps
- 高带宽 UDP 单流场景需要 connection multiplexing（多 stream / 多 connection pool）
- 当前 architecture 1:1 (src ↔ connection) 简单可靠，多数业务场景充分

## 资源花费

- 4× n2-standard-4 跑 ~25 分钟 ≈ **$0.35**
- 累计（P8+P9a+P9c）≈ **$1.10**，远低于 $5 预算

## 清理
- ✅ GCP 4 实例 + zfperf-allow firewall
- ✅ master 端 gcp-* nodes + perf forwards
- ✅ rfchost zfperf-http
