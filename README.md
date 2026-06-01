<div align="center">
  <img src="web/public/logo.png" alt="Iris" width="96" />
  <h1>Iris</h1>
  <p><strong>高性能、生产级的多级转发控制平台</strong></p>
  <p>智能调度 · 全链路 mTLS · 自动续签 · 实时流量统计</p>

[![CI](https://github.com/Everless321/Iris/actions/workflows/musl-build.yml/badge.svg)](https://github.com/Everless321/Iris/actions/workflows/musl-build.yml)
[![License](https://img.shields.io/badge/license-AGPL--3.0-orange.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.75+-orange.svg)](https://www.rust-lang.org/)
[![Platform](https://img.shields.io/badge/platform-Linux%20x86__64-lightgrey.svg)]()

</div>

---

## ✨ 特性

- **任意 N 跳级联** — path 为节点组序列，支持任意拓扑
- **每跳 4 种负载均衡** — 加权轮询 / 会话保持(Rendezvous 一致性哈希) / 最小连接 / 延迟最优
- **全链路 mTLS** — master ↔ node、node ↔ node 双向证书，内置 CA 自动签发
- **Cert 自动续签** — 节点 cert 365 天，剩 30 天自动调 RenewCert，零人工介入
- **全链路故障转移** — 入口/中转任意一跳节点故障自动切换组内健康节点
- **TCP + UDP 双协议** — UDP 走 QUIC datagram 避免 TCP backpressure（实测跨 region 丢包 <2%）
- **实时流量统计** — per-forward bytes_in/out 每 5s 上报，Prometheus `/metrics` + Web UI 表格
- **Web 管理面板** — 节点管理 / 转发编辑 / 流量监控 / SLA 看板 / 拓扑可视化
- **一键节点安装** — 面板生成命令，SSH 到目标服务器粘贴即可（自动下载 binary + 兑换证书 + systemd 服务）
- **L4 透传** — TCP/UDP，自动兼容上层任意代理协议（SS/Trojan/VLESS 等对平台透明）

## 🚀 快速开始

### 部署 master（一台）

```bash
# 1. 拉二进制（musl static-pie，所有 Linux 通杀）
curl -fsSLO https://github.com/Everless321/Iris/releases/latest/download/iris-master-musl-x86_64
chmod +x iris-master-musl-x86_64
sudo mv iris-master-musl-x86_64 /opt/iris/iris-master

# 2. 配置 master.env（admin 密码、JWT 密钥）
sudo mkdir -p /opt/iris
sudo tee /opt/iris/master.env <<EOF
IRIS_JWT_SECRET=$(openssl rand -hex 32)
IRIS_ADMIN_USER=admin
IRIS_ADMIN_PASS=$(openssl rand -base64 24)
EOF
sudo chmod 600 /opt/iris/master.env

# 3. 启 systemd service（unit 见 docs/deploy/iris-master.service 或自行编写）
sudo systemctl enable --now iris-master

# 4. 浏览器打开 http://<MASTER>:7080，admin 登录
```

### 加节点（一行命令）

在 Web UI 添加节点 → 系统生成 enrollment token → 复制安装命令到目标服务器执行：

```bash
curl -fsSL https://<MASTER>:7080/install.sh | sudo bash -s -- \
  --master https://<MASTER>:7080 \
  --token <ENROLLMENT_TOKEN>
```

脚本会：自动检测架构 → 下载 `iris-node` → 调 enroll API 兑换 mTLS 证书 → 写 systemd unit → 启动服务。**约 30 秒节点上线**。

**升级节点**：
```bash
curl -fsSL https://<MASTER>:7080/install.sh | sudo bash -s -- --upgrade
```

**卸载节点**（自动备份 cert + 配置到 `/tmp/iris-uninstall-*.tar.gz`）：
```bash
curl -fsSL https://<MASTER>:7080/install.sh | sudo bash -s -- --uninstall
```

## 🏗️ 架构

```
                Web UI (React + AntD)
                    ↕ REST + gzip/br
              master (Rust)
        ├─ axum (HTTP API + SLA + /metrics)
        ├─ tonic (gRPC · mTLS 控制面)
        ├─ probe (TCP 健康探测调度)
        └─ SQLite (节点 / forwards / 流量 / 用户 / cert 状态)
                    ↕ gRPC mTLS heartbeat (5s)
   ┌──────────┐   ┌──────────┐   ┌──────────┐
   │ 入口组   │←→│ 中转组   │←→│ 出口组   │   每跳分布式选路 + failover
   └──────────┘   └──────────┘   └──────────┘
   raw_tunnel (TCP) + quic_tunnel (UDP datagram)
```

| 层 | 技术栈 |
|---|---|
| 后端 | Rust 2021 · tokio · tonic 0.12 · axum · rustls 0.23 + aws-lc-rs · sqlx + SQLite |
| 数据面 | mTLS gRPC（控制面） · raw_tunnel（TCP, bypass HTTP/2 framing） · quinn 0.11（UDP, QUIC datagram） |
| 前端 | React 18 · TypeScript · AntD 5 · Vite |
| 部署 | musl static-pie 单二进制 + systemd（master + node 都是单文件） |

## 📦 Crate 结构

```
crates/
  proto/    gRPC 协议定义（Control + DataPlane + RenewCert）
  common/   mTLS 证书生成（rcgen 内置 CA）+ x509 cert 解析
  master/   控制面：HTTP API + gRPC + 探测 + SLA + Web UI 嵌入
  node/     数据面：入口 listener + 隧道转发 + 故障转移 + 自动续签
  cli/      命令行工具
```

## ⚙️ 关键环境变量

### master

| 变量 | 默认 | 说明 |
|---|---|---|
| `IRIS_LISTEN` | `0.0.0.0:7443` | gRPC mTLS 控制面 |
| `IRIS_HTTP` | `0.0.0.0:7080` | HTTP API + Web UI |
| `IRIS_DB` | `sqlite://data/iris.db` | 数据库路径 |
| `IRIS_CERT_DIR` | `certs` | CA + 服务器证书目录 |
| `IRIS_JWT_SECRET` | （随机） | 管理员 JWT 签名密钥，**生产必设 ≥32 字节** |
| `IRIS_ADMIN_USER` | — | 初始管理员用户名 |
| `IRIS_ADMIN_PASS` | — | 初始管理员密码 |
| `IRIS_PROBE_INTERVAL` | `15` | 节点探测间隔（秒）|
| `IRIS_FAIL_THRESHOLD` | `2` | 连续失败几次判不健康 |
| `IRIS_REQUIRE_TLS` | `0` | `1` = 强制 HTTPS（生产建议） |

### node

| 变量 | 默认 | 说明 |
|---|---|---|
| `IRIS_NODE_ID` | `node-dev-1` | 节点标识 |
| `IRIS_MASTER` | `https://127.0.0.1:7443` | master gRPC URL |
| `IRIS_DATA_ADDR` | `0.0.0.0:7444` | 数据面监听（gRPC=:7444 / raw_tunnel=:7445 / quic_tunnel=:7446） |
| `IRIS_CERT_DIR` | `certs` | mTLS 证书目录 |
| `IRIS_AUTO_RENEW` | `1` | `0` = 关闭自动续签（应急回滚） |

## 🔬 hops DSL（CLI 简写）

```
"a | b1:3,b2:1@weighted | c1,c2@source_hash"
 │   └ 第二跳：b1 权重 3 / b2 权重 1，加权轮询
 │                          └ 第三跳：c1/c2，源 IP 哈希会话保持
 └ 入口（单节点）
```

策略：`weighted`(默认) / `source_hash` / `least_conn` / `latency`

## 🛠️ 本地开发

```bash
# 编译（dev profile）
cargo build

# 一键 demo（自动起 master + 3 节点 + 创建 forwards）
bash scripts/demo.sh           # 3 跳级联
bash scripts/demo-lb.sh        # 加权负载均衡
bash scripts/demo-failover.sh  # 出口故障转移 + SLA
bash scripts/demo-midfail.sh   # 中转故障转移
```

发布构建（musl static-pie）见 `.github/workflows/musl-build.yml`。

## 📊 监控

- HTTP API `/api/sla` — JSON 节点可用率、延迟、故障统计
- HTTP API `/metrics` — Prometheus 格式，含 `iris_forward_bytes_in_total` / `iris_forward_bytes_out_total{id,name,port}` 等
- Web UI Forwards 表「流量」列实时显示每条 forward 的上下行字节数

## 🤝 贡献

PR、Issue、讨论欢迎：
- 提交代码前先 `cargo check --workspace`（CI 也会跑）
- 节点变更涉及 mTLS / cert 续签的代码改动需要 e2e 测试（见 `docs/testing/`）
- 详细 changelog 见 `docs/changelog/`

## 📚 文档索引

- `docs/changelog/` — 每次变更的设计 + 实测结果（按时间倒序）
- `docs/testing/` — 测试 spec（mTLS、性能、流量统计等）
- `docs/benchmark/` — 性能基准数据（带宽对比图等）
- `deploy/gcp/` — GCP 自定义镜像 + bootstrap 脚本参考

## 📜 License

[AGPL-3.0](LICENSE) · Copyright (c) 2026 Iris Contributors

> 强 copyleft 协议：任何修改、分发或通过网络提供本软件服务的衍生作品，
> 必须以同样 AGPL-3.0 协议公开完整源码。商业闭源使用请联系作者另购授权。
