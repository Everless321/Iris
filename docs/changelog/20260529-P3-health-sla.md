# P3 · 健康探测 + 故障转移 + SLA

日期：2026-05-29

## 产品参数（用户定）
- 探测频率：可配置 env ZF_PROBE_INTERVAL，默认 15s
- 故障阈值：连续 2 次探测失败 → unhealthy 切流（env ZF_FAIL_THRESHOLD 默认 2）
- SLA 指标：可用率、平均延迟、故障次数/时长、实时在线节点数（全做）

## 技术方案
- master 探测调度器：周期 TCP connect 每个节点 addr，测 RTT + 存活
- 探测结果存 nodes 表：health/latency_ms/fail_count/probe_total/probe_ok/fail_events/down_since/downtime_ms
- 健康+延迟随 sync 下发，node 周期拉取更新本地视图（部分热更新）
- 入口 LB：跳过 unhealthy 节点；latency 策略用真实延迟；全挂则退化尽力转发
- 建连故障重试：入口连下一跳失败 → 遍历组内健康节点重试
- SLA 导出：/metrics (Prometheus) + /api/sla (JSON)

## 子任务
- [x] 3.1 探测调度器 + nodes 探测字段迁移 ✅ 周期 TCP 探测状态机
- [x] 3.2 健康下发 + node 周期更新视图 + LB 跳过不健康 ✅
- [x] 3.3 latency 策略落地（真实延迟）✅ latency_picks_lowest 测试过
- [x] 3.4 建连故障重试 ✅ 杀 b1 后 8/8 切 b2
- [x] 3.5 SLA 指标 + /metrics + /api/sla ✅ Prometheus + JSON
- [x] 3.6 Workflow 对抗审查 + 修复 ✅ 14 候选→7 确认→全修

## Workflow 审查（19 agent / 110万 token / 5维度）
候选 14 → 确认 7 → 剔除 7（对抗验证剔除率 50%）

**架构升级**（应对 high #1/#2/#3）：入口集中选路 → **每跳分布式选路 + 各跳 failover**
- TunnelHeader 改传 remaining_hops(节点组)+client_ip+forward_id+hop_index
- connect_next 统一入口/中转：组内有序候选逐个 failover
- 一举修复：#1 连接计数（ConnGuard 跟随实际节点）、#2 中转 failover、#3 source_hash 拓扑稳定（改 Rendezvous/HRW 哈希）

**其他修复**：
- #4 Prometheus label 转义（esc 函数）
- #5 unhealthy 节点 latency_ms 置 NULL
- #6/#7 竞态与 Option::take（分布式重构后自然消解）
- 日志 ANSI 按 is_terminal 自动开关

## 验证
- 单测 11 个全过（新增 rendezvous 拓扑稳定、ConnGuard 计数、各策略 ordered）
- demo-lb 6:2 / demo-failover 出口切换 8/8 / demo-midfail 中转切换 8/8
- SLA：/api/sla + /metrics（label 已转义）

## 验证（demo-failover.sh）
- 正常分流 b1:4 b2:4
- 杀 b1 → 8/8 成功全切 b2（探测 2 次失败标记 unhealthy + node 周期刷新视图 + LB 跳过）
- SLA：b1 unhealthy/fail_events=1/uptime 0.22，b2 healthy 0.89，online 2/3
- 单测 10 个全过（新增 unhealthy_skip / latency_pick / all_unhealthy_degrade）

## 已知小限制
- latency_ms 本地探测恒为 0（RTT<1ms 取整），生产环境有真实值

## 执行结果
（回填）
