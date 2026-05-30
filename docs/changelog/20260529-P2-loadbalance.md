# P2 · 负载均衡 + 数据模型演进

日期：2026-05-29

## 需求（用户定）
- 每一跳可为节点组，组内负载均衡
- 策略：加权轮询、会话保持(源IP哈希)、最小连接数、延迟最优
- 节点配权重
- 故障重试留到 P3
- 策略每个节点组单独配

## 架构决策
- **入口集中选路**：入口一次性为整条路径每跳选定节点 → 具体路径 → 走 P1 隧道（数据面不改）
- per-connection 选路（TCP 长连接铁律：选定后锁死整条连接）
- 数据模型 path → hops（节点组+策略+权重），向后兼容旧 path 数组
- 延迟最优 P2 降级为加权轮询（无探测数据，P3 接入）
- 最小连接数 P2 按本入口活跃连接计数（近似，P3 全局精确）

## 子任务（本轮 2.1-2.5）
- [x] 2.1 数据模型 hops + nodes.weight + 迁移 + 旧格式兼容 ✅
- [x] 2.2 master API/sync 支持 hops，下发节点权重 ✅
- [x] 2.3 node LoadBalancer 选路器（4 策略 + 5 单测全过）✅
- [x] 2.4 数据面适配：加权分流验证 8 连接 = 6:2 ✅
- [x] 2.5 CLI 支持 hops DSL（a | b1:3,b2:1@weighted）✅
- [x] 2.6 Workflow 对抗式多维审查 + 修复确认问题 ✅ 15/16 确认问题全修

## 执行结果
- **P2 全部完成**：负载均衡核心 + 对抗式审查修复
- LB 4 策略：加权轮询✅ 源IP哈希✅ 最小连接✅(近似) 延迟最优⚠️(降级)
- 验证：8 连接加权分流 6:2 精确、单测 7 个全过、3跳+LB 两 demo 通过

### Workflow 审查（21 agent / 119万 token / 5维度并行 + 对抗验证）
候选 16 → 确认 15 → 修复 15（1 个误报"循环无限循环"被对抗验证纠正为有限次优）

**High（4）全修**：
1. heartbeat 错误吞掉 → match + warn/error 日志
2. hops JSON 解析失败静默 → warn 日志含 forward_id/raw
3. sync_config 空 hops 无声过滤 → warn 日志
4. CLI weight 无上界 OOM → clamp ≤1000（CLI+master+lb 三层防护）

**Medium（8）全修**：
5/6/13. 双向转发关闭协调 → 引入 link() helper（select 任一结束 abort 另一端）+ handle_entry_conn 改 select!
7. least_conn 平局热点 → 按 node id 字典序 tiebreaker
8/9. 出口首帧写 / 中转首帧 send 错误静默 → 传播为 Status::unavailable
10. 循环路径 → API HashSet 检测重复节点拒绝
11. listen_port 范围 → API 校验 1-65535
12. LB 选中节点未注册 → 入口校验 node_addrs，缺失则放弃

**Low（3）全修**：
14. expand 大权重内存 → MAX_WEIGHT 上界
15. CLI 空节点 id → filter_map 过滤

### 新增回归测试
- least_conn_tie_is_deterministic_by_id（平局确定性）
- huge_weight_is_capped（权重上界）

## 文件清单
- crates/node/src/{lb.rs（新增 LoadBalancer+7测试）, dataplane.rs（LB入口+link协调）, main.rs}
- crates/master/src/{models.rs, api.rs, main.rs}（hops + 校验 + 日志）
- crates/master/migrations/0002_node_weight.sql
- crates/proto/proto/control.proto（ForwardRule hops/Hop/HopNode）
- crates/cli/src/main.rs（hops DSL + 防御）
- scripts/demo-lb.sh（加权 LB 演示）
