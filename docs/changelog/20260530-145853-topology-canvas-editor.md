# 拓扑编辑器：表单+预览二合一

## 背景
用户反馈：
- 端口转发设置流程很难受（4 段式 Card：基础配置 / 入口 / 路径 / 预览）
- ReactFlow 自动排版输出的拓扑图很丑（节点位置乱、连线交叉、b1/b2 错位）
- 想"只在拓扑里设置"

## 待办
- [x] 删掉"入口/路径/预览"3 个 Card，合并为单一"拓扑"画布
- [x] 顶部紧凑工具条：name / listen_port / protocol / target 横排
- [x] 列式拓扑布局：每一列 = 一个 hop
- [x] 同列多节点 = 多入口/负载均衡，列顶显示策略 Select（入口列除外）
- [x] 列底"+"按钮：往该 hop 加节点
- [x] 列之间"+"按钮：插入新 hop
- [x] 最右终点列：跟随 target 实时渲染
- [x] 弃用 ReactFlow，改用 CSS Grid + SVG overlay
- [x] SVG 边：水平贝塞尔曲线，hover 高亮
- [x] 移除 @xyflow/react 依赖引用

## 实施
**前**：`<Form>` × 4 个 Card + ReactFlow 旁观
**后**：顶部 1 个紧凑 Card（4 字段横排） + 1 个画布 Card（编辑即预览）

### 数据结构
**不变**。`{ name, listen_port, protocol, target, hops: [{ strategy, nodes: [{ id, weight }] }] }` 完全沿用，后端零改动。

### 影响范围
- `web/src/pages/TopologyEditor.tsx`（重写）

### 技术细节
- SVG overlay 用 `useLayoutEffect` 计算节点 `getBoundingClientRect()`，画水平贝塞尔
- ResizeObserver 监听画布尺寸变化重新计算边路径
- 入口列特殊：无策略 Select，蓝色高亮，注释"客户端任选"
- 终点列特殊：渲染 target host:port，蓝色填充
- 节点芯片：节点名 + 权重 InputNumber（仅路径列） + 删除

### 视觉
- 颜色：入口/终点 `#1677ff`，中转 `#d9d9d9`
- 字体：节点 ID/权重/端口 用 mono
- 边：默认 `#bfbfbf` 0.6 opacity，连接入口/终点的边 `#1677ff`

## 验证
- pnpm build 通过 ✓
- 单页路由 `/forwards/new` 与 `/forwards/:id/edit` 均生效

## 后续
- 拖拽节点重新分组（V2）
- 拓扑只读视图（在 Forwards 列表点行展开）
