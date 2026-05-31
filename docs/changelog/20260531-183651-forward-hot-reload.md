# forward 热加载：sync_config 时 reconcile listener，无需 restart node

## 背景

bench 测试时发现：master 加 forward 后，node 端**不会自动启动 listener**——必须 restart iris-node 才能拉到新 forward。根因：`crates/node/src/main.rs` 启动时**一次性** spawn forward listener，心跳循环的 sync_config 只更新 `ctx.nodes`（节点视图），不动 forwards。

## 改造

### 新增数据结构
```rust
struct ActiveForward {
    rule: ForwardRule,           // 全量快照，用于 diff
    handles: Vec<JoinHandle<()>>, // TCP/UDP listener task abort handles
}
```

### 新增函数（main.rs）
- `spawn_forward(f, node_id, ctx, lb, target_router) -> Option<ActiveForward>`
  - 提取原启动循环里的 spawn 逻辑（TCP/UDP × 单跳/多跳 4 种 case）
  - 仅当本节点是 hops[0] 且 targets 非空时启动
- `reconcile_forwards(new_forwards, &mut active, node_id, ...)`
  - 删除消失的 forward：abort handles + 从 map 移除
  - 处理新增 / 改动：rule == old 则复用；否则 abort + 重 spawn
  - 不再是入口节点：abort + 移除

### 主流程改动
- 启动时：用 `reconcile_forwards(...)` 替代原 `for f in reply.forwards { ... }` 循环
- 心跳循环每 5s `sync_config` 后：调用 `reconcile_forwards(...)` 同步 listener 状态

### 关键设计点
- **rule 比较**：直接用 `ForwardRule == ForwardRule`（prost 默认 derive PartialEq），任何字段变化都触发重启
- **socket 端口复用**：`sock::tcp_listen` 已设 `SO_REUSEADDR`（line 23），abort 后立即同端口 spawn 不会 EADDRINUSE
- **抖动节流**：sync_config 周期是 5s，reconcile 计算 O(n) 不会爆

## 测试场景

1. **新增**：master 加 forward → 5s 内 node `ss -tnlp` 出现 listen port ✓
2. **删除**：master 删 forward → 5s 内 listen port 消失 ✓
3. **改 listen_port**：18388 → 18389 → 老端口消失新端口出现 ✓
4. **不再是入口**：把当前节点从 hops[0] 移走 → 5s 内 listener 消失 ✓
5. **再次成为入口**：加回 hops[0] → 5s 内 listener 重新出现 ✓
6. **回归**：原有 forward 工作 + 心跳全部不变 ✓

## 改动文件

| 文件 | 改动 |
|------|------|
| `crates/node/src/main.rs` | +180 / -85 行（提取 spawn_forward + reconcile_forwards + 主流程改造） |

## 验证

- [x] cargo check --workspace 通过
- [x] cargo test --workspace 通过（11/11 lb tests）
- [x] GitHub Actions musl build (run 26710296077)
- [x] prod 4 节点部署 (commit 24d5c1e)
- [x] 5 场景手动验证 **5/5 全 PASS**

## 最终结果

```
=== 场景 1: 新增 forward → 5s 内 listener 出现 ===  ✅ LISTEN
=== 场景 2: 删除 forward → 5s 内 listener 消失 ===  ✅ MISSING
=== 场景 3: 改 listen_port → 老消失新出现       ===  ✅ MISSING / LISTEN
=== 场景 4: 把节点从 hops[0] 移走 → listener 消失 === ✅ MISSING
=== 场景 5: 再加回 hops[0] → listener 重新出现   === ✅ LISTEN
```

热加载机制按设计工作：
- 5s sync_config 周期内完成 listener 启停
- SO_REUSEADDR 让 port 立即可用，未触发 EADDRINUSE
- ForwardRule 字段比较精确识别"完全不变"vs"任何字段变化"
- 不影响心跳 / 现有 forward / 数据面
