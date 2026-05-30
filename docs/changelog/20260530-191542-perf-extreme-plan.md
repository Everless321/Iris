# 极致性能优化 — 总规划（暂存，未执行）

## 决策（已 Hard Stop 通过）
| 项 | 选择 |
|---|---|
| 目标 | **C：单流 + 多流都接近 10 Gbps line rate** |
| 范围 | **3：L1 + L2 + L3 全做** |
| 测试预算 | **$5 上限**（GCP n2-standard-4 × N 次迭代）|
| 执行时机 | 暂停，待用户额度恢复后启动 |

## 当前已知瓶颈（来自 20260530-183805 + 20260530-170823 测试）

n2-standard-2 实测：
- 直连基线：9695 Mbps
- 1-hop 单流：**5876 Mbps**（**单核 mTLS 极限**）
- 2-hop 单流：2078 Mbps（relay 节点 1 字节做 2 次 crypto）
- 1-hop 8 流：9314 Mbps（96% 线速）
- 2-hop 8 流：4536 Mbps（CPU 186%+181% / 200%）
- 内存：每 zhuanfa-node 进程 RSS **10-21 MB**

## 执行顺序

### Phase 0：Profile 实锤（必做，~1h，$0.40）

> **不 profile 直接优化 = 猜。所有后续优化都要先看到这一步的 flamegraph**

1. 起 2× n2-standard-4（GCP asia-east1-a）
2. 在 entry 节点装 `linux-perf` + `cargo install flamegraph`
3. 跑 1-hop + 2-hop 多流满载
4. 抓 `perf record -F 99 -p <pid> -g` 30 秒
5. 出 flamegraph，定位 top 5 热点函数

**期望发现**：
- AES-GCM `ring::aead::seal_in_place`
- HTTP/2 `h2::codec::framed_write::FramedWrite::poll_ready`
- protobuf `prost::Message::decode`
- tokio mpsc / wake 路径
- 内存拷贝（memcpy / Vec::clone）

### Phase 1：L1 Quick Wins（**预估 +20-40%**，2-3h，$1）

文件：`crates/node/src/dataplane.rs` + 入口/出口路径

| 改动 | 实现 | 风险 |
|---|---|---|
| **BUF 16K → 64K**（或 1MB）| 改常量 | 极低 |
| **`Vec<u8>` → `bytes::Bytes`** | `Chunk { data: bytes::Bytes }`，tonic 原生支持，去 `.to_vec()` 拷贝 | 低 — 改 proto + struct |
| **SO_RCVBUF/SNDBUF 4MB** | `socket2` crate 调用 setsockopt | 低 |
| **TCP_NODELAY** | 入口 + 出口 `TcpStream::set_nodelay(true)` | 极低 |
| **release profile 强化** | Cargo.toml 加 `[profile.release] lto="fat", codegen-units=1, panic="abort"` | 编译变慢 |

**验证**：跑同样 8 流 1-hop/2-hop 对比 phase 0 数据

### Phase 2：L2 协议剥层（**预估 +2-3×**，1-2 天，$2）

**思路**：节点间数据面跳过 gRPC HTTP/2 framing，用裸 mTLS TCP + 8-byte length-prefix

具体动作：
1. **新增 `crates/node/src/raw_tunnel.rs`**：
   - 头帧：`[u8; 4] header_len + bincode(TunnelHeader)`
   - 数据帧：`[u8; 4] frame_len + bytes`
   - 双向 splice TLS stream
2. **`DataPlane` gRPC 服务保留作 fallback**，新加 `RawTunnel` 端口（如 7445）
3. **`connect_dataplane` 优先尝试 raw 模式**，失败降级 gRPC
4. **`ForwardRule.use_raw` 字段控制**（兼容老节点）

**风险**：
- 自己写 wire format，要小心 partial read / buffer 边界
- mTLS 仍然走 rustls，但抛掉 hyper/h2/tonic
- 调试比 gRPC 难（没现成的工具）

**期望**：单流 ~9 Gbps（AES-GCM 单核满），2-hop 多流 ~7-8 Gbps

### Phase 3：L3 内核栈替换（**预估 +5-10×**，几天，$1.5）

只做最高 ROI 一项：**kTLS**

1. **依赖**：Linux 5.0+（n2-2 默认 6.1 ✓）
2. **方案**：rustls + ktls crate（社区实现，已在 cloudflare 用）
   ```toml
   ktls = "6"
   ```
3. **替换 tokio-rustls 的 `TlsStream` → ktls 的 `KtlsStream`**
4. **完成后用 `tcpdump -i any -w cap -s 0` 验证密文流确实由内核加密**
5. **opt-out**：环境变量 `ZF_DISABLE_KTLS=1` 走回 rustls，保留兼容

**期望**：
- TLS 加密由 NIC offload（如果支持）或 kernel TCP_ULP
- syscall 翻倍效率
- 单流 ~10 Gbps（线速）
- 2-hop 多流 ~8-9 Gbps

**不做的 L3 项目**（保留候选，看 Phase 3 是否还不够）：
- ❌ io_uring：tokio-uring 仍 experimental，破坏现有 tokio 生态
- ❌ splice() 零拷贝：和 mTLS 不兼容（密文必须经 userspace 解）
- ❌ 自写 AEAD：风险极高、收益边际
- ❌ NUMA 绑核：n2-2 只有 2 vCPU，无 NUMA 拓扑

## 验收标准

每个 Phase 完成后必须满足：

| 项 | 标准 |
|---|---|
| 编译 | `cargo build --release --target x86_64-unknown-linux-musl` 通过 |
| 现有测试 | 1-hop / 2-hop / 多 target 链路测试都还能通 |
| 不破协议 | master ↔ 老节点 sync_config 仍工作（兼容滚动）|
| 性能 | 该 Phase 预估范围内 |
| 记录 | 实测数字 + flamegraph 截图 + diff 摘要回贴到本 changelog |

## 资源管理

每次起 GCP 都必须：
1. 开机前：列预算 + 预估时长
2. 跑完立即：`gcloud compute instances delete --quiet` + firewall 删除
3. master 端清理：删 perf node + perf forwards

实测预算 ≈ $5：
- Phase 0 profile：~$0.40
- Phase 1 测试：~$1
- Phase 2 测试 × 2-3 次迭代：~$2
- Phase 3 测试 × 2-3 次迭代：~$1.5
- Buffer：$0.10

## 当前状态

- ✅ GCP：0 实例 / 0 firewall rule（已确认）
- ✅ master：4 原节点（nosla-hk / hawaii / att / rfchost），0 forward
- ⏸ **执行：暂停，等用户额度恢复**
- 📍 恢复后第一步：Phase 0 profile

## 待办（恢复后照此顺序）

- [ ] 起 2× n2-standard-4 + 装 perf + cargo install flamegraph
- [ ] Phase 0 跑 1-hop / 2-hop 多流满载 + 抓 flamegraph 30s
- [ ] 把 top 5 热点 append 到本 changelog
- [ ] 执行 Phase 1（5 小项），跑回归 + 实测
- [ ] 执行 Phase 2 raw_tunnel.rs，跑回归 + 实测
- [ ] 执行 Phase 3 kTLS，跑回归 + 实测
- [ ] 最终对比表 + 销毁所有 GCP

## 跨 session 提示

恢复此任务时，重读本文件即可。所有上下文（瓶颈、决策、预算、目标、待办）都在这里。

```bash
cat docs/changelog/20260530-191542-perf-extreme-plan.md
```
