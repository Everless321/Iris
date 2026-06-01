# Cert 自动续签 V1 (#33)

## 背景
节点 mTLS cert 默认有效期 1 年（rcgen 默认），到期前没有自动续签机制 → 1 年后节点集体掉线。商用 blocker。

## 设计
节点定时自检 cert.not_after，剩 ≤30 天调 master.RenewCert RPC 续签 → 写新 cert → 进程退出让 systemd 重启 → 2s 内回归。
RenewCert 用现有 mTLS cert 认证（旧 cert 仍有效）。Master handler 从 `tonic::Request::peer_certs()` 提取 client 证书 CN，校验 == request.node_id（防越权）。

## 改动清单

### proto
- `control.proto`：service Control 加 `RenewCert(RenewCertRequest) returns (RenewCertResponse)`
- HeartbeatRequest 加 `int64 cert_not_after_ms = 6`（UI 显示倒计时）

### common
- 加 `x509-parser` 依赖
- 加 `pub fn cert_not_after_ms(pem: &[u8]) -> Result<i64>` 解析 cert 到期时间

### master
- Cargo.toml 加 `x509-parser`（用于 peer cert CN 解析）
- ControlSvc 加 `cert_dir: String` 字段
- 实现 `renew_cert(req)`：
  - `req.peer_certs()` 取首张 client cert
  - 用 x509-parser 解析 subject CN（格式 `iris-node-<id>`）
  - 校验 CN 末段 == request.node_id
  - 调 `iris_common::sign_node_cert(cert_dir, node_id)` 签新 cert
  - 返回 (cert_pem, key_pem, valid_until_ms)
- heartbeat handler 把 `cert_not_after_ms` 写进 `nodes` 表（新列）
- migration `0009_node_cert_expiry.sql` 加 `cert_not_after_ms INTEGER NULL`
- API `/api/nodes` DTO 加 `cert_not_after_ms`

### node
- 启动时读 client.pem → 调 `cert_not_after_ms()` 记到内存（`Arc<AtomicI64>`）
- heartbeat 每次带上这个值（让 UI 看见倒计时）
- 启动 background renew task：
  - 每 59-61min 随机抖动检查
  - 剩 ≤30 天 → 调 `client.renew_cert()`
  - 成功 → 原子写 `client.pem.new` → rename → 同样 client-key.pem → `std::process::exit(0)` 让 systemd 重启
  - 失败 → 日志 warn，1h 后重试
- env `IRIS_AUTO_RENEW=0` 关掉自检（默认开）

### UI
- `web/src/lib/api.ts` Node type 加 `cert_not_after_ms?: number`
- `web/src/pages/Nodes.tsx` 表加「cert 到期」列：
  - 剩 >30d → 绿 `327d`
  - 剩 ≤30d → 黄 `25d ⚠`
  - 剩 ≤7d → 红 `5d ❌`
  - null → 灰 `—`

### sign_node_cert
- 显式 `not_before = now()`，`not_after = now() + 365d`（rcgen 默认值，显式好审计）

## 待办
- [ ] proto 改动
- [ ] common: x509-parser 依赖 + cert_not_after_ms 函数
- [ ] master: ControlSvc 加字段 + RenewCert handler + heartbeat 写库 + migration + API DTO
- [ ] node: 启动读 cert + heartbeat 带字段 + background renew task + restart
- [ ] UI: Node type + 列
- [ ] cargo build 验证
- [ ] 部署到 master + 1 个 GCP 节点
- [ ] e2e: 节点强制 patch cert not_after=now+25d，等 1h 续签自动触发
- [ ] commit + push

## 安全
- 续签 RPC 用 mTLS peer cert 校验 CN == request.node_id（防 A 借 B cert 申请 B 的新 cert）
- 续签失败不影响节点运行，仅日志告警
- UI 倒计时让 admin 提前知道有问题
- env `IRIS_AUTO_RENEW=0` 应急回滚

## 结果
（执行后填）
