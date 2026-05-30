# 拓扑编辑器：Dify/Coze 工作流风格

## 背景
上一版"列式 + SVG 自绘"画布仍然不舒服。用户希望像 Dify/Coze 那样：
- 节点是大卡片
- 节点右侧"+"直接追加下一步并自动连线
- 点节点弹右侧 Drawer 配置详情
- 节点可拖拽

## 待办
- [x] 复用 @xyflow/react，定义 3 种自定义节点：EntryNode / HopNode / TargetNode
- [x] 每种节点带顶部 accent 条 + 图标 + 标题 + 副标题 + 节点 chips 预览
- [x] HopNode 右上 ✕ 删除、右侧悬浮 + 追加下一 hop
- [x] EntryNode 右侧悬浮 + 追加下一 hop（不可删除）
- [x] TargetNode 蓝底实色，左侧接入 handle，目标地址跟随顶栏输入
- [x] 点击节点 → 右侧 Drawer 弹出详细配置（策略 + 节点列表 + 权重）
- [x] 节点可拖拽，位置在会话内保留（不持久化到后端）
- [x] 边自动连线（hop→hop 灰色虚线动效，hop→target 蓝色实线）
- [x] 顶部 4 字段工具条保留（name / port / protocol / target）
- [x] Drawer 内"删除此 hop"按钮（入口除外）

## 实施
**前**：列式自绘画布（节点列里堆 chips，列间小 "+" 按钮）
**后**：ReactFlow 自由画布 + 3 种 Dify 风格大卡片节点 + 右侧 Drawer 配置

### 数据结构
**不变**。`{ name, listen_port, protocol, target, hops: [{ strategy, nodes }] }` 完全沿用，后端零改动。

### 影响范围
- `web/src/pages/TopologyEditor.tsx`（重写）
- 无新增依赖

### 关键技术点
1. **回调稳定性**：节点 data 里的回调用 useCallback + 函数式 setState，避免 stale closure
2. **位置保留**：useEffect 同步 hops → rfNodes 时用 posMap 保留用户拖拽位置
3. **入口约束**：EntryNode 不可删除（removeHop 拒绝 hi=0）
4. **事件冒泡**：节点上的 +/✕ 按钮 onClick 都加 stopPropagation，避免触发 onNodeClick 打开 Drawer
5. **Handle 视觉**：白底蓝边小圆，与节点融合不喧宾夺主

## 验证
- pnpm build 通过
- master 重启后 bundle hash 刷新
