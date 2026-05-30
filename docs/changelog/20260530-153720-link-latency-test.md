# 拓扑链路全链路延迟测试

## 背景
用户在拓扑编辑器里需要"测试链路"，看每一段是否走得通 + 多慢。

## 待办
- [x] proto: DataPlane 新增 `ProbeReach(addr, timeout_ms)` RPC
- [x] node: 实现 probe_reach（TCP connect 计时）
- [x] master: 启动时构造 master→node 的 mTLS ClientTlsConfig（复用现有 client.pem）
- [x] master: 新增 `POST /api/forwards/test`，编排笛卡尔积探测
- [x] web: 顶栏加"测试链路"按钮
- [x] web: 自定义 ReactFlow Edge 显示 chip（ms · n/m ✓）
- [x] web: 点 chip 弹 Drawer 显示该边每一对探测结果
- [x] 全链编译通过 + 端到端烟测 200/400/401 路径正确

## 实施

### Proto (`crates/proto/proto/control.proto`)
DataPlane 增加 `ProbeReach`：节点对外做 TCP connect，返回 `{ ok, latency_ms, error }`。

### Node (`crates/node/src/dataplane.rs`)
`probe_reach` 实现：`tokio::time::timeout(t, TcpStream::connect(addr))`，超时上限 10s，默认 2s。

### Master
- `main.rs`: 启动时用现有 `client.pem` + `ca.pem` 构造 `ClientTlsConfig`，存进 `AppState`
- `api.rs`: 新增 `POST /api/forwards/test`
  - 入参：`{ hops, target }`（编辑态，未保存也能测）
  - 编排：hop[i].nodes × hop[i+1].nodes + last_hop.nodes × target 的所有边
  - 按 from_node 分桶，每个 from_node 建一次 mTLS channel 串行 probe（减少重复握手）
  - 整体超时 8s，单 probe 2s
  - 节点未注册 → 立即标记错误，不阻塞其他探测

### Frontend (`web/src/pages/TopologyEditor.tsx`)
- 顶栏加 `测试链路` 按钮（loading 状态）
- 测试中：edge chip "测试中…" + 蓝色虚线动效
- 完成：aggregate by edge → chip 显示 `min~max ms · n/m`
- 颜色规则：
  - max < 50ms → 绿
  - max < 200ms → 黄
  - ≥ 200ms 或部分失败 → 橙
  - 全失败 → 红
- 点 chip → Drawer 显示该边的逐对结果矩阵（from → to addr / latency / error）

## 验证
- `cargo build -p zhuanfa-master -p zhuanfa-node` 通过
- `pnpm build` 通过（bundle `index-CwEyISAy.js`）
- 端到端烟测：
  - 未认证 → 401 ✓
  - 认证 + 空 hops → 400「hops 不能为空…」✓
  - 认证 + 真链路 (4 条边) → 200 + results 形状正确 ✓
  - 节点不可达 → `ok=false, error="连不上节点: transport error"` ✓
- 实际 ms 数字未现网验证（容器已停），需在真节点上点按钮看

## 影响范围
| 文件 | 变化 |
|---|---|
| crates/proto/proto/control.proto | +ProbeReach RPC + 2 messages |
| crates/node/src/dataplane.rs | +probe_reach impl |
| crates/master/src/main.rs | +ClientTlsConfig 构造，去重 dir/paths |
| crates/master/src/api.rs | +AppState.node_caller_tls, +test_forward 路由+handler |
| web/src/lib/api.ts | +EdgeProbe, TestResponse 类型 |
| web/src/pages/TopologyEditor.tsx | +测试按钮、自定义 Edge、第二个 Drawer |

零新增依赖（@xyflow/react 已用 BaseEdge/EdgeLabelRenderer/getBezierPath）。

## 安全补丁（同任务追加）

后台自动安全审查命中 `test_forward` 存在 SSRF / 内网端口扫描风险：认证用户可以
拿节点群当扫描器去探主节点本机或节点所处的内网（如 169.254.169.254 云元数据
端点、192.168.x 内网服务）。

### 防御
1. **master 侧 target 解析后 IP 白名单**（`check_external_target` + `is_disallowed_ip`）
   - 解析 `req.target` 的所有 IP，任一命中 denylist 就拒
   - denylist：回环、RFC1918 私网（10/8、172.16/12、192.168/16）、链路本地（169.254/16、
     fe80::/10）、多播、未指定、广播；IPv6 ULA fc00::/7；IPv6 回环 ::1
2. **dev 旁路**：`ZF_ALLOW_PRIVATE_TARGETS=1` 放行（容器化本地测试用）
3. **node 侧不加 denylist**：node-to-node 探测合法用 DB 里的内网 addr
   （host.docker.internal、10.x、172.16.x），加 denylist 会把合法链路打死。
   master 是信任根（签所有证书），"防御被攻陷的 master"边际收益过低
4. **不改 AdminClaims**：客户自助 create/edit 是产品方向，测试是它的一部分。
   靠目标白名单保护，不靠身份门禁

### 烟测
| 测试输入 | 结果 |
|---|---|
| `127.0.0.1:5432` (回环) | HTTP 400 ✓ |
| `localhost:22` (解析→::1) | HTTP 400 ✓ |
| `10.0.0.1:22` (RFC1918) | HTTP 400 ✓ |
| `192.168.1.1:80` (RFC1918) | HTTP 400 ✓ |
| `169.254.169.254:80` (云元数据 SSRF 经典靶) | HTTP 400 ✓ |
| `[::1]:22` (IPv6 回环) | HTTP 400 ✓ |
| `1.1.1.1:443` (公网) | HTTP 200 ✓ |
| `ZF_ALLOW_PRIVATE_TARGETS=1` + `127.0.0.1` | HTTP 200 ✓ |

### 未采纳的额外硬化
CGNAT (100.64/10) + benchmark (198.18/15) 未加入 denylist。两者
非标准但 std 没默认标识；保持最小覆盖匹配安全审查推荐项，
有需要再加。

## 后续
- 测试中 edge 加流动动画（CSS @keyframes dashoffset）
- 探测结果可缓存到 forward 行，下次编辑/查看时回放
- 给客户的 SLA 看板里也展示一次的探测快照
