# P1 · 控制骨架 + 数据模型

日期：2026-05-29

## 决策变更
- **数据库改用 SQLite**（原计划 PostgreSQL）
  - 理由：10-50 客户规模够用、零运维、单文件备份、免起容器
  - 取舍：master 不可多实例水平扩展；高频写靠 WAL 缓解
  - 退路：sqlx 抽象层保留，500+ 客户时再迁 PostgreSQL

## 子任务
- [x] 1.1 SQLite schema（nodes / forwards(path JSON) / users）+ sqlx 接入，启动自动建表 ✅ 5 表建出，WAL
- [x] 1.2 proto 扩展 SyncConfig：node 按 node_id 拉取相关 forward ✅ node(a) 拉到规则
- [x] 1.3 axum HTTP API：节点 / 转发 CRUD ✅ curl 增删查通过，心跳更新节点 online
- [x] 1.4 TCP 数据面：copy_bidirectional 单跳透传 ✅ curl 经入口透传返回 ok
- [x] 1.5 多跳隧道：node 间 gRPC stream 递归承载 ✅ 2 跳 + 3 跳 (a→b→c) 端到端稳定
- [x] 1.6 CLI：zhuanfa-cli add-node/add-forward/list ✅ 走 HTTP API，完整流程验证

## 技术决策
- sqlx 0.8 runtime API（非编译期宏），避免编译期连库依赖
- DB 文件 `data/zhuanfa.db`，WAL 模式，create_if_missing
- forward.path 存 TEXT(JSON 数组)，支持任意 N 跳
- 迁移用 sqlx::migrate! 内嵌 migrations/

## 执行结果
- **P1 全部 6 子任务完成**，控制面 + 数据面端到端打通
- 数据面架构：path 数组 + 递归 Tunnel（每跳剥一层 remaining_path），天然支持任意 N 跳
- 节点对称设计：同一份代码，既是 DataPlane server（被上游连）又是 client（连下游）
- 节点间通信全程 mTLS（复用 P0 的 CA + client 证书）
- 验证矩阵：单跳 ✓、2 跳 ✓、3 跳 ✓、CLI 增删查 ✓、心跳更新节点 online ✓
- 新增 crate：cli（走 HTTP API，与 master 解耦）

## 已知限制（留待后续阶段）
- 配置变更需重启 node 才生效（热更新 → P2）
- 节点间假设可直连公网地址（家宽反向隧道 → P2）
- 无健康探测 / 故障转移（→ P3）
- DataPlane server 复用 client 证书作 server 身份（P2 改为 master 动态签发节点专属证书）

## 文件清单
- crates/cli（新增，clap + ureq）
- crates/node/src/{dataplane.rs（新增）, forward.rs（新增）, main.rs（重写）}
- crates/master/src/{api.rs, models.rs, db.rs（新增）, main.rs}
- crates/master/migrations/0001_init.sql
- crates/proto/proto/control.proto（扩展 SyncConfig / DataPlane / NodeAddr）
