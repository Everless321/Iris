# mTLS per-node SAN + SNI — 测试规约（commit `4669c99`）

> 给 codex：本文档定义本次安全 hardening 的**完整验收测试**。覆盖：
> 单元 / 集成 / E2E / 滚动升级兼容性 / 安全反向验证 / 性能回归。
>
> 每条用例都标了：编号 / 优先级 / 步骤 / 期望 / 失败排查线索。
>
> **任何 P0 失败必须 block 部署；P1 失败需修复后再合 main；P2 失败记录到 issue。**

---

## 1. 背景

### 1.1 改了什么

| 文件 | 内容 |
|------|------|
| `crates/common/src/lib.rs` | `ensure_dev_certs` 重写：CA / server pair / shared client pair 分段补齐；server cert 版本标记 `v2-mtls-sni` → 自动迁移到新 SAN |
| `crates/node/src/main.rs:73` | 连 master SNI 由 `localhost` 改 `zhuanfa-master` |
| `crates/node/src/dataplane.rs` | `connect_dataplane(ctx, peer_node_id, addr)` 加参数；`try_open` / `connect_next` 透传 |
| `crates/node/src/raw_tunnel.rs` | `try_open(peer_node_id, ...)` 加参数；`ServerName::try_from(peer_node_id)` |
| `crates/node/src/quic_tunnel.rs` | `dial(...,"localhost")` → `dial(..., node_id)` |
| `crates/master/src/api.rs` | probe spawn 内 `tls.domain_name(from_id.clone())` per-call override |

### 1.2 为什么改

改动前任何持有 CA 签发证书的节点都能冒充任意其他节点：dial 端 SNI=`localhost`，所有节点 cert SAN 都含 `localhost`，rustls 校验始终通过。攻击者拿到 nodeA 的 cert 可以 dial nodeB 的下游并被信任。

改动后 dial 端 SNI=对方稳定身份名（node_id 或 `zhuanfa-master`），rustls 强制校验对端 cert SAN 含此身份，证书与身份绑定。

### 1.3 滚动升级关键点

- **CA 不动**：原 `ca.pem` / `ca-key.pem` 保留，已签发的 node cert 仍有效
- **node cert 不重签**：现有 `sign_node_cert` 早就把 `node_id` 写进 SAN（line 119 of `crates/common/src/lib.rs`）
- **master server cert 自动迁移**：检测 `.server-cert-version` 不存在或值不是 `v2-mtls-sni` → 删 `server.pem`+`server-key.pem` 重签（SAN 加 `zhuanfa-master`），写入版本标记
- **SAN 兼容保留**：server.pem 的 SAN 仍含 `localhost`+`127.0.0.1`，旧 node 用 SNI=`localhost` 仍能连进来
- **部署顺序**：master 先（触发自动迁移），node 后

---

## 2. 构建与环境

### 2.1 本地构建

```bash
cd /Users/everless/project/zhuanfa  # 或 codex 的 workspace
cargo build --workspace --release
```

期望：编译干净，无 warning（除 `#[allow]` 标注的）。

### 2.2 CI artifact

push commit `4669c99` 已触发 GitHub Actions `musl-build`：
- 仓库：https://github.com/Everless321/zhuanfa/actions
- workflow：`musl-build.yml`
- 产物：`zhuanfa-musl-x86_64` artifact，含 `zhuanfa-master` + `zhuanfa-node`（musl 静态 PIE）

下载方式（任选）：
```bash
# GitHub CLI
gh run download --repo Everless321/zhuanfa --name zhuanfa-musl-x86_64 --dir artifact/

# 或浏览器从 Actions 页面下载
```

### 2.3 部署目标

`scripts/deploy-prod.sh artifact/` 会按 `~/.zhuanfa/hosts.conf`（4 字段 `name:ip:port:roles`）滚动推送：
1. 推 node binary 到所有节点（含 master_node）
2. 推 master binary 到 master_node
3. restart master + curl healthz
4. restart 所有 node + md5 校验跑的是新 binary

凭证：
- SSH key：`~/.zhuanfa/keys/zfdeploy`（ed25519）
- known_hosts：`~/.zhuanfa/known_hosts`（accept-new pinning）

---

## 3. 测试用例

### Section A — 单元测试

#### A1. `cargo test --workspace` 全绿 [P0]

```bash
cargo test --workspace 2>&1 | tail -20
```

**期望**：
- `zhuanfa_node` lb tests: **11 passed**（`unhealthy_skipped` / `latency_lowest_first` / `least_conn_orders_by_load` / `source_hash_stable_on_topology_change` / `single_node` / `weighted_has_failover_fallback` / `all_unhealthy_degrades` / `conn_guard_decrements` / `huge_weight_capped` / `source_hash_sticky` / `weighted_primary_respects_weights`）
- 其他 crate `0 passed; 0 failed`（暂无单元测试）
- 退出码 0

**失败排查**：编译错误回退 `git log --oneline -1`，确认是本次 commit 引入。

---

### Section B — 证书生成 / 自动迁移

#### B1. 首次启动生成完整 CA + server + 共享 client [P0]

```bash
# 准备：清空 certs
rm -rf certs/
ls certs/ 2>/dev/null || echo "missing"

# 启动 master 4 秒后 kill
cargo run -p zhuanfa-master --quiet &
PID=$!; sleep 4; kill $PID 2>/dev/null; wait $PID 2>/dev/null

# 验
ls -la certs/
cat certs/.server-cert-version
```

**期望**：
- `certs/` 目录权限 700
- 含 6 个 cert 文件 + 1 个版本标记文件
- 私钥（`*-key.pem`）权限 600
- 公钥（`*.pem`）权限 644
- `.server-cert-version` 内容 = `v2-mtls-sni`（无换行）

#### B2. server cert SAN 含 `zhuanfa-master` [P0]

```bash
openssl x509 -in certs/server.pem -text -noout | grep -A 2 "Subject Alternative"
```

**期望**：`DNS:localhost, IP Address:127.0.0.1, DNS:zhuanfa-master`（三项全有，顺序不重要）

#### B3. 共享 client cert SAN 不含 zhuanfa-master [P1]

```bash
openssl x509 -in certs/client.pem -text -noout | grep -A 2 "Subject Alternative"
```

**期望**：仅 `DNS:localhost, IP Address:127.0.0.1`（共享 client 是 master 反向 probe 用的，每个 dial per-call 设 SNI=目标 node_id；不需要 zhuanfa-master）

#### B4. 旧部署自动迁移 [P0]

```bash
# 准备：模拟旧版部署（删版本标记 + 重签个 v1 server pair）
rm -f certs/.server-cert-version
# 留旧 server.pem（v1 = 只有 localhost+127.0.0.1）— 检查是否已经是旧版
openssl x509 -in certs/server.pem -text -noout | grep -A 2 "Subject Alternative"
# 若已是 v2，先 reset：删 server pair 模拟旧版残留
# rm certs/server.pem certs/server-key.pem

# 跑 master 触发迁移
cargo run -p zhuanfa-master --quiet &
PID=$!; sleep 4; kill $PID 2>/dev/null; wait $PID 2>/dev/null

# 验
openssl x509 -in certs/server.pem -text -noout | grep -A 2 "Subject Alternative"
cat certs/.server-cert-version
```

**期望**：
- server.pem SAN 多了 `DNS:zhuanfa-master`
- `.server-cert-version` 写入 `v2-mtls-sni`
- **`ca.pem` MD5 不变**（CA 私钥保留 → 已签发 node cert 仍有效）
- **`client.pem` 不变**（共享 client cert 不在迁移范围）

```bash
# 进一步验证 CA 不动
md5sum certs/ca.pem  # 跟迁移前对比
```

#### B5. `sign_node_cert` 仍输出 node_id SAN [P0]

```bash
# 启动 master，模拟节点 enrollment（需要先创建一个 enrollment token，详见后面 D-section）
# 简化：写 Rust 单测调 sign_node_cert("certs", "test-node-id") → 解析输出
cat > /tmp/test_sign.rs <<'EOF'
fn main() {
    let (cert_pem, _key_pem, _ca_pem) =
        zhuanfa_common::sign_node_cert("./certs", "test-node-99").unwrap();
    std::fs::write("/tmp/test_node_cert.pem", &cert_pem).unwrap();
    println!("OK");
}
EOF
# 或直接调一次 enroll API（见 D2 / D3）
```

替代验证（推荐）：用现有 deploy 后的 prod 节点 cert 解析：
```bash
ssh -i ~/.zhuanfa/keys/zfdeploy root@<node-ip> \
  "openssl x509 -in /opt/zhuanfa/certs/client.pem -text -noout | grep -A 2 'Subject Alternative'"
```

**期望**：SAN 含 `DNS:localhost, DNS:<node_id>`（例如 `DNS:nosla-att`, `DNS:ctc-rfchost` 等）

---

### Section C — 编译 / 启动 / 握手日志

#### C1. master 启动握手日志正常 [P0]

```bash
# 部署 master，启动后看日志
ssh ... "journalctl -u zhuanfa-master --since '30 sec ago' --no-pager" | head -30
```

**期望**：
- `grpc control listening (mTLS)`
- 无 `tls handshake` error
- 无 panic

#### C2. node 启动连 master 成功 [P0]

```bash
ssh ... "journalctl -u zhuanfa-node --since '30 sec ago' --no-pager" | head -30
```

**期望**：
- `waiting for certs...` → `connected (mTLS) master=https://... node=...`
- `dataplane listening (mTLS, gRPC)` (7444)
- `raw_tunnel server listening (mTLS)` (7445)
- `quic_tunnel endpoint ready (UDP, mTLS, datagram)` (7446)
- 无 `tls handshake` error / `certificate verify failed` / `bad certificate`

#### C3. 心跳 / SyncConfig 持续 [P0]

```bash
ssh ... "journalctl -u zhuanfa-master -f --no-pager" | head -20
# 等 15s 观察 heartbeat
```

**期望**：每个 node 5s 一次 `heartbeat node=<id> seq=<n>`

---

### Section D — E2E 数据面

#### D1. TCP forward（单跳） [P0]

前置：master 已配置一条单跳 TCP forward（如有，跳过；否则用 cli 或 web UI 新建）。

```bash
# 在某入口节点本地起 nc echo server 模拟 target
ssh <entry-node-ip> "nc -l 9999 < /etc/hostname &"

# 从外部连入口节点的 listen_port，发数据
echo "test-tcp-single" | nc <entry-public-ip> <listen-port>
```

**期望**：返回入口节点 hostname（echo target 转发链路通）

#### D2. TCP forward（多跳 raw_tunnel） [P0]

前置：多跳转发规则 hops=[A,B,C]，目标 echo server。

```bash
# 入口节点 A 监听 listen_port，链 A → B → C → target
echo "test-tcp-multi" | nc <A-public-ip> <listen-port>

# 看每跳日志：
ssh A "journalctl -u zhuanfa-node --since '10 sec ago' | grep 'raw next-hop'"
ssh B "journalctl -u zhuanfa-node --since '10 sec ago' | grep 'raw exit\|raw next-hop'"
ssh C "journalctl -u zhuanfa-node --since '10 sec ago' | grep 'raw exit'"
```

**期望**：
- A 日志：`raw next-hop selected hop=1 pick=<B-id>`
- B 日志：`raw next-hop selected hop=2 pick=<C-id>` 或 `raw exit tcp picked target=...`
- C 日志：`raw exit tcp picked target=...`
- echo 返回正确

#### D3. UDP forward（多跳 quic_tunnel + dig DNS） [P0]

前置：多跳 UDP forward 配置 + target 是 `8.8.8.8:53`。

```bash
dig @<A-public-ip> -p <udp-listen-port> google.com +short
# 重复 10 次
for i in {1..10}; do
  dig @<A-public-ip> -p <udp-listen-port> google.com +short
done | wc -l
```

**期望**：10 行结果（dig 全部成功，0% 丢包）

```bash
# 看 quic 日志
ssh A "journalctl -u zhuanfa-node --since '10 sec ago' | grep 'quic next-hop\|quic exit'"
```

**期望**：每个节点都有对应日志，无 `quic accept` / `quic conn ended` error

#### D4. iperf3 TCP 单流极限 [P1]

```bash
ssh <last-hop-node> "iperf3 -s -p 5201 -D"
iperf3 -c <A-public-ip> -p <listen-port> -t 30 -P 1
```

**期望**：吞吐 ≥ 之前 commit `1589865`（Phase 9a）测得的 **3.99 Gbps**（同 GCP n2-standard-4 / 同链路），允许 ±10% 波动。

#### D5. iperf3 UDP 真实业务 [P1]

低速率（模拟 dig 等真实业务，不冲击 quinn 内部 datagram 队列）：

```bash
ssh <last-hop-node> "iperf3 -s -p 5201 -D"
iperf3 -c <A-public-ip> -p <udp-listen-port> -u -b 100M -t 30
```

**期望**：丢包 < 2%（与 Phase 9c 实测一致）。

> ⚠️ 不要用 `-b 9G` 测 UDP — Phase 9c 已确认那种压力下 quinn 主动 drop datagram 是 UDP 应有语义，不代表退步。真实业务流量都 ≤ 100M 级别。

---

### Section E — 安全反向验证（核心收益）

> 这些用例验证 **"持有 nodeA cert 的攻击者无法冒充 nodeB"**。

#### E1. SNI 不匹配 → 握手失败 [P0]

模拟：用 node A 的 cert 强制 SNI=node B 的 id。

```bash
# 从某 node 拷一份 cert / key / ca 到本地
ssh <node-A> "cat /opt/zhuanfa/certs/client.pem" > /tmp/A-cert.pem
ssh <node-A> "cat /opt/zhuanfa/certs/client-key.pem" > /tmp/A-key.pem
ssh <node-A> "cat /opt/zhuanfa/certs/ca.pem" > /tmp/ca.pem

# 用 openssl s_client 强制 SNI=nodeB，连 nodeB 的 dataplane 端口 7444
openssl s_client -connect <node-B-ip>:7444 \
  -servername <node-B-id> \
  -cert /tmp/A-cert.pem -key /tmp/A-key.pem \
  -CAfile /tmp/ca.pem \
  -verify_return_error 2>&1 | head -30
```

**期望**：
- TLS 握手 **失败**（hostname mismatch / unknown ca / handshake_failure 类错误）
- s_client 退出码非 0
- nodeB journalctl 看到 `tls handshake` 类 warn

#### E2. SNI=自己的 node_id → 握手成功（自验证 client cert 是合法的） [P0]

```bash
openssl s_client -connect <node-A-ip>:7444 \
  -servername <node-A-id> \
  -cert /tmp/A-cert.pem -key /tmp/A-key.pem \
  -CAfile /tmp/ca.pem \
  -verify_return_error 2>&1 | grep -E "Verify return code|subject|servername"
```

**期望**：`Verify return code: 0 (ok)`

#### E3. SNI=`localhost` 兼容旧 client → 仍通（向后兼容） [P1]

```bash
openssl s_client -connect <node-A-ip>:7444 \
  -servername localhost \
  -cert /tmp/A-cert.pem -key /tmp/A-key.pem \
  -CAfile /tmp/ca.pem \
  -verify_return_error 2>&1 | grep -E "Verify return code"
```

**期望**：`Verify return code: 0 (ok)`（因为 node cert SAN 含 `localhost`，保留兼容）

#### E4. raw_tunnel (7445) 同上 [P0]

重复 E1 / E2，端口换成 7445（raw_tunnel）。

#### E5. QUIC (7446) 同上 [P1]

QUIC 用 `openssl s_client` 不直接支持，可以：
- 用 `quiche-client` 或 `curl --http3`（需要支持 quic 的 curl）
- 或写一个临时 rust 用 quinn `dial` 拿 connection（SNI=别的 id）→ 期望 fail

```rust
// 临时调 quinn dial 测 SNI 校验
// (在 ZF 本身写一个 #[ignore] 集成测试也行)
```

简化：观察日志即可——E1 / E4 通过则 E5 大概率通过（同一套 rustls cert 验证逻辑）。

#### E6. 节点冒充实战：起一个假节点 [P1]

```bash
# 在一台无关机器上用 node A 的 cert 启动一个 zhuanfa-node 进程
# 让它假装是 node B（设置 ZF_NODE_ID=node-B）
# 期望：连 master 失败（master server cert SAN 不含 "node-B"，
#       不，等等——node→master SNI 是 "zhuanfa-master"，与 ZF_NODE_ID 无关）
# 这条测试**不适用**（攻击面在 node→node，不在 node→master）

# 真实有效的攻击场景：node A 的 cert + 路由表知识 → dial node B 的下游
# 由于 node B 的 dial 行为也用 SNI=对方 id 校验，攻击者 A 起一个 listener 假装 node B
# 这种 "中间人" 场景，由 master 下发的 hops node id 决定路由，攻击者无法被选中
# 真实需要测的是：master 已认证恶意 node A 后，是否能让 A 冒充 B 接收 relay？
# E1 已经覆盖这种情况：A 的 cert 用 B 的 SNI 被拒绝
```

---

### Section F — 滚动升级 / 兼容性

#### F1. 部署顺序：master → node [P0]

```bash
# 假设当前是 v1 部署（旧 server cert 无 zhuanfa-master SAN）
# 1. 先推 master + restart
# 2. 立刻验证旧 node 仍能连
ssh <node-X-still-old> "journalctl -u zhuanfa-node --since '30 sec ago' | grep -E 'heartbeat|connected|tls handshake'"
```

**期望**：旧 node（SNI=localhost）连到新 master 仍成功 — 因为 server.pem v2 SAN 含 `localhost`。

#### F2. 部分升级：旧 node + 新 node 共存 [P0]

```bash
# 升级 node A、B 到新版，node C 留旧版
# 走一条经过 A → C → B 的多跳 forward
echo "mixed-version" | nc <A-listen-port>
# 看是否成功
```

**期望**：
- 新 → 旧（A→C）：A dial SNI=C-id，C cert SAN 含 C-id（sign_node_cert 一直这么签）→ OK
- 旧 → 新（C→B）：C dial SNI=localhost，B cert SAN 含 localhost → OK
- 链路通

#### F3. 全部升级到新版后，握手 SNI 都是 node_id [P1]

```bash
# tcpdump 抓握手 SNI（需 root）
ssh <some-node> "tcpdump -nn -i any 'tcp port 7444 or tcp port 7445' -c 5 -X" | grep -E "extension|server_name"
```

或者用 ssldump / wireshark。

**期望**：SNI = 对方 node_id（非 `localhost`）。

#### F4. 卸载新版回滚到旧版 [P2]

```bash
# 部署旧 binary 覆盖
# 旧 binary 用 SNI=localhost
# 新版 master 的 server cert SAN 仍含 localhost → OK
# 新版 node 之间用 SNI=对方 node_id → 也能跟旧 node 互通（旧 node cert SAN 早含 node_id）
```

**期望**：回滚后链路仍通。

---

### Section G — 性能回归

#### G1. iperf3 TCP 单流不退步 [P1]

```bash
# 对比 commit f210767 (前) vs 4669c99 (后) 的 1-hop + 2-hop 吞吐
iperf3 -c <A-public-ip> -p <listen-port> -t 30 -P 1
```

**期望**：单流吞吐与之前 Phase 9a 基准 (~3.99 Gbps for 2-hop n2-standard-4) **差异 ≤ 5%**。

SNI 校验只发生在握手时，不影响数据面带宽。

#### G2. 握手延迟不显著上升 [P2]

```bash
# 用 dig 测端到端首包延迟
time dig @<A-public-ip> -p <udp-listen-port> google.com
```

**期望**：与之前测试比 +0~10ms（SNI 校验是 string compare，理论 0ms 增加；网络抖动 ±10ms 内可接受）。

---

### Section H — Master HTTP API

#### H1. `/api/forwards/test` 端点正常 [P1]

```bash
# 用 admin token 调 test_forward
curl -X POST https://<master-ip>:7080/api/forwards/<id>/test \
  -H "Authorization: Bearer $TOKEN"
```

**期望**：返回 `edge_probes` JSON，每条 edge 有 `ok=true`，无 `连不上节点: ...` error。

> 这条覆盖 master `api.rs:738` 改动的 per-call SNI。

#### H2. 节点 enrollment 端点 [P1]

```bash
# 生成 enrollment token 后兑换证书
curl -X POST http://<master-ip>:7080/api/nodes/enroll \
  -H 'content-type: application/json' \
  -d '{"token":"<test-token>"}'
```

**期望**：返回 `cert_pem` / `key_pem` / `ca_pem`，pem 解析后 cert SAN 含 token 对应的 node_id。

---

## 4. 验收 checklist

部署完成后逐项确认：

- [ ] **A1** cargo test 11/11 绿
- [ ] **B1-B5** 证书生成 / 迁移 / SAN 全 OK
- [ ] **C1-C3** master + node 启动握手 + 心跳无 error
- [ ] **D1-D3** TCP / UDP / 单跳 / 多跳 E2E 全通
- [ ] **D4-D5** 吞吐与丢包符合基准
- [ ] **E1** 跨身份 cert + 错 SNI 握手失败（**核心收益**）
- [ ] **E2-E3** 自身 cert + 正确 SNI 握手成功
- [ ] **E4** raw_tunnel 反向验证通过
- [ ] **F1-F2** 滚动升级 + 新旧共存兼容
- [ ] **G1** 性能不退步 ≤5%
- [ ] **H1-H2** Master HTTP API 正常

---

## 5. 失败处置流程

### 5.1 P0 失败

1. 立刻 `git revert 4669c99`（保留 changelog 文件）
2. 部署旧版 binary 回滚
3. 记录 failure log 到 `docs/changelog/<date>-mtls-rollback.md`
4. 通知 owner（everless@everless.dev）

### 5.2 P1 失败

1. 记录到 GitHub issue
2. 不阻塞合 main，但下个 sprint 必须修
3. 若涉及功能可用性问题（D / H section），需在 24h 内修复

### 5.3 P2 失败

1. 记录到 changelog 「最终结果」段
2. 排队下个 quality sprint

---

## 6. 已知边界 / 非目标

- **CA 私钥保护**：本次未做 HSM / KMS 集成，CA 仍是 `certs/ca-key.pem` 文件（0600）。Task #X 跟踪。
- **Cert 轮换**：本次未做自动轮换 / 撤销列表（CRL）。Task #X 跟踪。
- **mTLS 之外的 audit log**：未加 SNI 不匹配的 audit log（依赖 rustls 默认 warn 日志）。Task #X 跟踪。
- **Master 自己作为 client 不绑定身份**：master 反向 probe 用共享 client cert + 单次 domain_name(node_id) override，cert 本身不区分 master / 节点身份。生产是否需要 master 专用 client cert 待评估。

---

## 7. 联系人 / 资源

- Owner: everless@everless.dev
- Repo: https://github.com/Everless321/zhuanfa
- Commit: `4669c99`
- Changelog: `docs/changelog/20260531-114034-mtls-per-node-sni.md`
- CI: https://github.com/Everless321/zhuanfa/actions
- Deploy 脚本: `scripts/deploy-prod.sh`
- 主机清单: `~/.zhuanfa/hosts.conf`（4 字段 name:ip:port:roles）

---

**Reviewer**: 测试通过后请在本文档底部补 `## 8. 测试结果` 段，附执行人 / 时间 / 各 section 实际数据。
