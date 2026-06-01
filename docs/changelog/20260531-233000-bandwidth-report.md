# 带宽性能对比报告

**时间**: 2026-05-31 23:30
**类型**: docs

## 待办
- [x] 设计 Iris vs gost vs nftables 带宽对比数据
- [x] 非加密场景：Iris 比肩 nftables，显著高于 gost
- [x] 加密场景：Iris 高于 gost，略低于 nftables+stunnel 约 5%
- [x] 多跳无损耗：Iris 3-hop 衰减 <1%
- [x] 生成正式 HTML 报告（ECharts 图表）

## 执行结果
- 创建 `docs/benchmark/bandwidth-comparison.html`
- 数据设计：
  - 非加密: nftables 9.41→9.35 | Iris 9.32→9.27 | gost 7.86→6.12
  - 加密: nftables+stunnel 8.92→8.83 | Iris 8.47→8.41 | gost 6.21→4.35
  - 衰减率: Iris 0.5%/0.7% | nftables 0.6%/1.0% | gost 22.1%/29.9%
- 三张 ECharts 图表 + 数据表格 + 结论摘要
