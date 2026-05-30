# Zhuanfa · 高性能转发平台

基于 Rust 的多级连跳转发控制平台，主控/节点架构。差异化定位：**智能调度 + SLA 承诺 + 全链路故障转移**。

> 私有项目，暂不公开。

## 特性

- **任意 N 跳级联**：path 为节点组序列，支持任意拓扑
- **每跳负载均衡**：加权轮询 / 会话保持(Rendezvous 一致性哈希) / 最小连接 / 延迟最优
- **全链路故障转移**：入口与中转任意一跳节点故障，自动切换组内健康节点
- **健康探测 + SLA**：周期 TCP 探测，可用率/延迟/故障统计，`/metrics`(Prometheus) + `/api/sla`
- **全链路 mTLS**：master ↔ node、node ↔ node 双向证书，内置 CA 自动签发
- **L4 透传**：TCP/UDP，自动支持上层任意代理协议（SS/Trojan/VLESS 等对平台透明）

## 架构

```
                    Web UI (规划中)
                        ↕ REST
                   master (Rust)
              ├─ axum (HTTP API + SLA)
              ├─ tonic (gRPC · mTLS 控制面)
              ├─ probe (健康探测调度)
              └─ SQLite
                        ↕ gRPC mTLS
   ┌──────────┐   ┌──────────┐   ┌──────────┐
   │ 入口组   │←→│ 中转组   │←→│ 出口组   │   每跳分布式选路 + failover
   └──────────┘   └──────────┘   └──────────┘
```

| 层 | 技术 |
|---|---|
| 后端 | Rust · tokio · tonic · axum · sqlx + SQLite |
| 通信 | gRPC mTLS 双向流 |
| 部署 | master Docker Compose；node 静态二进制 + systemd |

## Crate 结构

```
crates/
  proto/    gRPC 协议定义（Control + DataPlane）
  common/   mTLS 证书生成（rcgen 内置 CA）
  master/   控制面：SQLite + HTTP API + 探测调度 + SLA
  node/     数据面：入口 LB + 隧道转发 + 故障转移
  cli/      命令行工具
```

## 快速开始（本地）

```bash
cargo build

# 一键演示
bash scripts/demo.sh           # 3 跳级联
bash scripts/demo-lb.sh        # 加权负载均衡
bash scripts/demo-failover.sh  # 出口故障转移 + SLA
bash scripts/demo-midfail.sh   # 中转故障转移
```

### 手动操作

```bash
# 启动 master（探测间隔可配）
ZF_PROBE_INTERVAL=15 ./target/debug/zhuanfa-master

# 注册节点（带权重）
zhuanfa-cli add-node --id a --name entry --addr 1.2.3.4:7444
zhuanfa-cli add-node --id b1 --name exit1 --addr 5.6.7.8:7444 --weight 3

# 创建负载均衡转发：a → [b1:3,b2:1 加权] → 目标
zhuanfa-cli add-forward --name relay --listen 10080 \
  --hops "a | b1:3,b2:1@weighted" --target 目标:端口

# 启动节点
ZF_NODE_ID=a ZF_DATA_ADDR=0.0.0.0:7444 ZF_MASTER=https://master:7443 ./target/debug/zhuanfa-node

# 查看 SLA
curl localhost:7080/api/sla
curl localhost:7080/metrics
```

## hops DSL

```
"a | b1:3,b2:1@weighted | c1,c2@source_hash"
 │   └ 第二跳：b1权重3/b2权重1，加权轮询
 │                          └ 第三跳：c1/c2，源IP哈希会话保持
 └ 入口
```

策略：`weighted`(默认) / `source_hash` / `least_conn` / `latency`

## 环境变量

| 变量 | 默认 | 说明 |
|---|---|---|
| `ZF_LISTEN` | `0.0.0.0:7443` | master gRPC 监听 |
| `ZF_HTTP` | `0.0.0.0:7080` | master HTTP API |
| `ZF_DB` | `sqlite://data/zhuanfa.db` | 数据库 |
| `ZF_PROBE_INTERVAL` | `15` | 探测间隔（秒）|
| `ZF_FAIL_THRESHOLD` | `2` | 连续失败几次判不健康 |
| `ZF_MASTER` | `https://127.0.0.1:7443` | node 连接的 master |
| `ZF_NODE_ID` | `node-dev-1` | 节点标识 |
| `ZF_DATA_ADDR` | `0.0.0.0:7444` | node 数据面监听 |

## 开发进度

- [x] P0 mTLS gRPC 骨架
- [x] P1 N 跳级联 + 控制面 + CLI
- [x] P2 每跳负载均衡（4 策略）
- [x] P3 健康探测 + 全链路故障转移 + SLA
- [ ] P4 Web UI + 拓扑可视化 + 客户视图
- [ ] P5 计费 + 套餐 + 流量统计

详见 `docs/changelog/`。

## License

AGPL-3.0
