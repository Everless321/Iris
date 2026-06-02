mod dataplane;
mod forward;
mod lb;
mod quic_tunnel;
mod ratelimit;
mod raw_tunnel;
mod session;
mod sock;
mod udp_forward;

use anyhow::Result;
use dataplane::{DataPlaneSvc, NodeCtx, NodeInfo};
use lb::LoadBalancer;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::task::JoinHandle;
use tonic::transport::{
    Certificate, ClientTlsConfig, Endpoint, Identity, Server, ServerTlsConfig,
};
use iris_proto::control::control_client::ControlClient;
use iris_proto::control::data_plane_server::DataPlaneServer;
use iris_proto::control::{
    ForwardRule, HeartbeatRequest, ListenerState, NodeAddr, RenewCertRequest, SyncRequest,
    TrafficStat,
};
use std::sync::atomic::{AtomicI64, Ordering};

/// 一条已激活 forward 的句柄：rule 全量快照（用于 diff 判定是否需要重启），
/// handles 持 TCP/UDP listener task 的 JoinHandle（abort 即可关闭 listener + 释放端口）。
/// status 是 spawn 时预先 probe bind 的结果，heartbeat 时上报给 master。
/// traffic 是字节流计数器（仅入口节点累计），与 listener task 共享。
struct ActiveForward {
    rule: ForwardRule,
    handles: Vec<JoinHandle<()>>,
    status: ListenerState,
    traffic: Arc<dataplane::TrafficCounter>,
}

/// listener task 内 bind 完成后,通过 oneshot 把 bind 结果上报给 spawn_forward，
/// 这样 master/UI 看到的 status.ok 是真实 bind 结果（不再依赖前置 probe）。
/// 1s timeout 是保险阀（正常 bind 在 ms 级；超时基本意味着 task 没启或卡在前置 await）。
async fn await_bind(rx: tokio::sync::oneshot::Receiver<Result<(), String>>) -> Result<(), String> {
    match tokio::time::timeout(Duration::from_secs(1), rx).await {
        Ok(Ok(r)) => r,
        Ok(Err(_)) => Err("bind result channel closed (task panicked?)".to_string()),
        Err(_) => Err("bind timed out (>1s)".to_string()),
    }
}

/// 按 ForwardRule 启动 TCP/UDP listener task，返回 ActiveForward（含 abort handles）。
/// 仅当本节点是 hops[0]（入口）且 targets 非空时启动；否则返回 None。
///
/// async：spawn 后通过 oneshot 等真实 bind 结果回报，status.ok 反映实际 bind 成败。
async fn spawn_forward(
    f: &ForwardRule,
    node_id: &str,
    ctx: &Arc<NodeCtx>,
    lb: &Arc<LoadBalancer>,
    target_router: &Arc<dataplane::TargetRouter>,
    sessions: &Arc<session::SessionTable>,
    entry_node_id_arc: &Arc<String>,
) -> Option<ActiveForward> {
    let is_entry = f
        .hops
        .first()
        .map(|h| h.nodes.iter().any(|n| n.id == node_id))
        == Some(true);
    if !is_entry {
        return None;
    }

    let targets: Vec<iris_proto::control::TargetEndpoint> = if !f.targets.is_empty() {
        f.targets.clone()
    } else {
        #[allow(deprecated)]
        let t = f.target.trim().to_string();
        if t.is_empty() {
            return None;
        }
        vec![iris_proto::control::TargetEndpoint { addr: t, weight: 1 }]
    };
    if targets.is_empty() {
        tracing::warn!(forward_id = f.id, "forward 没有 target，跳过 entry 启动");
        return None;
    }

    let target_strategy = if f.target_strategy.is_empty() {
        "weighted".to_string()
    } else {
        f.target_strategy.clone()
    };
    let parts: Vec<&str> = f
        .protocol
        .split('+')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    let has_tcp = parts.is_empty() || parts.iter().any(|p| *p == "tcp");
    let has_udp = parts.iter().any(|p| *p == "udp");

    let port = f.listen_port as u16;
    let fid = f.id;
    let mut handles: Vec<JoinHandle<()>> = Vec::new();
    let mut bind_rxs: Vec<tokio::sync::oneshot::Receiver<Result<(), String>>> = Vec::new();
    let traffic = Arc::new(dataplane::TrafficCounter::default());
    // #39 per-forward 速率限制（TCP 和 UDP 共享同一份 bucket；0 = 该方向不限）
    let rate = Arc::new(ratelimit::RateLimit::new(f.rate_in_bps, f.rate_out_bps));

    if has_tcp {
        let (tx, rx) = tokio::sync::oneshot::channel();
        bind_rxs.push(rx);
        if f.hops.len() == 1 {
            let (t, s, tc, ss, ent, rl) = (
                targets.clone(),
                target_strategy.clone(),
                traffic.clone(),
                sessions.clone(),
                entry_node_id_arc.clone(),
                rate.clone(),
            );
            handles.push(tokio::spawn(async move {
                if let Err(e) = forward::run_single_hop(port, fid, t, s, tc, ss, ent, rl, tx).await {
                    tracing::error!(error = %e, "tcp single-hop entry exited");
                }
            }));
        } else {
            let (hops, t, s, ctx2, lb2, tc, ss, ent, rl) = (
                f.hops.clone(),
                targets.clone(),
                target_strategy.clone(),
                ctx.clone(),
                lb.clone(),
                traffic.clone(),
                sessions.clone(),
                entry_node_id_arc.clone(),
                rate.clone(),
            );
            handles.push(tokio::spawn(async move {
                if let Err(e) =
                    dataplane::run_multi_hop_entry(port, fid, hops, t, s, ctx2, lb2, tc, ss, ent, rl, tx).await
                {
                    tracing::error!(error = %e, "tcp multi-hop entry exited");
                }
            }));
        }
    }
    if has_udp {
        let (tx, rx) = tokio::sync::oneshot::channel();
        bind_rxs.push(rx);
        if f.hops.len() == 1 {
            let (t, s, tr, tc, rl) = (
                targets.clone(),
                target_strategy.clone(),
                target_router.clone(),
                traffic.clone(),
                rate.clone(),
            );
            handles.push(tokio::spawn(async move {
                if let Err(e) = udp_forward::run_udp_single_hop(port, fid, t, s, tr, tc, rl, tx).await {
                    tracing::error!(error = %e, "udp single-hop entry exited");
                }
            }));
        } else {
            let (hops, t, s, ctx2, lb2, tc, rl) = (
                f.hops.clone(),
                targets.clone(),
                target_strategy.clone(),
                ctx.clone(),
                lb.clone(),
                traffic.clone(),
                rate.clone(),
            );
            handles.push(tokio::spawn(async move {
                if let Err(e) =
                    udp_forward::run_udp_multi_hop(port, fid, hops, t, s, ctx2, lb2, tc, rl, tx).await
                {
                    tracing::error!(error = %e, "udp multi-hop entry exited");
                }
            }));
        }
    }

    // 收齐所有 listener 的真实 bind 结果。任意一个失败 → status.ok=false +
    // abort 已起的 task 释放占用的端口，避免 zombie listener。
    let mut all_ok = true;
    let mut first_err = String::new();
    for rx in bind_rxs {
        match await_bind(rx).await {
            Ok(()) => {}
            Err(e) => {
                if first_err.is_empty() {
                    first_err = e;
                }
                all_ok = false;
            }
        }
    }
    if !all_ok {
        tracing::warn!(forward_id = fid, port, reason = %first_err, "listener bind failed; aborting");
        for h in &handles {
            h.abort();
        }
        return Some(ActiveForward {
            rule: f.clone(),
            handles: Vec::new(),
            status: ListenerState {
                forward_id: fid,
                port: port as u32,
                protocol: f.protocol.clone(),
                ok: false,
                error: first_err,
            },
            traffic,
        });
    }

    Some(ActiveForward {
        rule: f.clone(),
        handles,
        status: ListenerState {
            forward_id: fid,
            port: port as u32,
            protocol: f.protocol.clone(),
            ok: true,
            error: String::new(),
        },
        traffic,
    })
}

/// 收集 active_forwards 状态用于 heartbeat 上报。
fn collect_listener_states(active: &HashMap<i64, ActiveForward>) -> Vec<ListenerState> {
    active.values().map(|af| af.status.clone()).collect()
}

/// 收集 active_forwards 流量计数器快照。bytes_in/out 是自 node 启动以来累计；
/// node 重启会归零，master 检测到 current < last 时把 delta 设为 current（视作新 epoch）。
fn collect_traffic_stats(active: &HashMap<i64, ActiveForward>) -> Vec<TrafficStat> {
    use std::sync::atomic::Ordering;
    active
        .iter()
        .map(|(fid, af)| TrafficStat {
            forward_id: *fid,
            bytes_in: af.traffic.bytes_in.load(Ordering::Relaxed),
            bytes_out: af.traffic.bytes_out.load(Ordering::Relaxed),
        })
        .collect()
}

/// 心跳循环每次 sync_config 后调用：对比新 forwards vs 当前 active，启停 listener。
/// - 删除：abort 旧 handles + 从 map 移除
/// - 新增 / 不再 / 重新成为入口节点 / 关键字段变化：abort 旧的（若有）+ spawn 新的
/// - rule 完全相同：保持不动
/// abort 一个 ActiveForward 的所有 listener task,并 await 它们 unwind（含 TcpListener/UdpSocket Drop
/// 释放 fd）。tokio::JoinHandle::abort() 只发 cancellation flag,fd 真正释放要等 task await 退出 +
/// 局部变量 Drop —— 直接接着 bind 同一端口 100% EADDRINUSE。timeout 2s 防止卡死 reconcile。
async fn shutdown_active(af: ActiveForward) {
    for h in &af.handles {
        h.abort();
    }
    for h in af.handles {
        let _ = tokio::time::timeout(Duration::from_secs(2), h).await;
    }
}

async fn reconcile_forwards(
    new_forwards: &[ForwardRule],
    active: &mut HashMap<i64, ActiveForward>,
    node_id: &str,
    ctx: &Arc<NodeCtx>,
    lb: &Arc<LoadBalancer>,
    target_router: &Arc<dataplane::TargetRouter>,
    sessions: &Arc<session::SessionTable>,
    entry_node_id_arc: &Arc<String>,
) {
    let new_ids: std::collections::HashSet<i64> = new_forwards.iter().map(|f| f.id).collect();

    // 1. 停掉消失的 forward
    let to_remove: Vec<i64> = active
        .keys()
        .filter(|id| !new_ids.contains(id))
        .copied()
        .collect();
    for id in to_remove {
        if let Some(af) = active.remove(&id) {
            let port = af.rule.listen_port;
            shutdown_active(af).await;
            tracing::info!(forward_id = id, port, "forward removed: listener stopped");
        }
    }

    // 2. 新增 / 改动
    for f in new_forwards {
        let fid = f.id;
        let still_entry = f
            .hops
            .first()
            .map(|h| h.nodes.iter().any(|n| n.id == node_id))
            == Some(true);

        if !still_entry {
            if let Some(af) = active.remove(&fid) {
                shutdown_active(af).await;
                tracing::info!(forward_id = fid, "no longer entry: listener stopped");
            }
            continue;
        }

        // 已激活 + rule 完全相同 + 上次 spawn 成功 → 复用。
        // bind 失败时 spawn_forward 返回 status.ok=false + handles=empty，
        // 即使 rule 不变也要重试（外部进程释放后下轮自动恢复）。
        if let Some(existing) = active.get(&fid) {
            if existing.rule == *f && existing.status.ok {
                continue;
            }
            // rule 变化 OR 上次 bind 失败 → 必须先 await 旧 task 完全退出，否则新 bind = EADDRINUSE
            if let Some(af) = active.remove(&fid) {
                let (old_port, prev_ok) = (af.rule.listen_port, af.status.ok);
                shutdown_active(af).await;
                tracing::info!(
                    forward_id = fid,
                    old_port,
                    new_port = f.listen_port,
                    prev_ok,
                    "forward changed or prev spawn failed: restarting listener"
                );
            }
        }

        // spawn 新 listener（spawn_forward 内部 await 真实 bind 结果回报）
        if let Some(af) =
            spawn_forward(f, node_id, ctx, lb, target_router, sessions, entry_node_id_arc).await
        {
            tracing::info!(
                forward_id = fid,
                port = af.rule.listen_port,
                proto = %af.rule.protocol,
                hops = af.rule.hops.len(),
                ok = af.status.ok,
                "forward listener spawned"
            );
            active.insert(fid, af);
        }
    }
}

fn build_nodes(ns: &[NodeAddr]) -> HashMap<String, NodeInfo> {
    ns.iter()
        .map(|n| {
            (
                n.id.clone(),
                NodeInfo {
                    addr: n.addr.clone(),
                    health: n.health.clone(),
                    latency_ms: n.latency_ms,
                },
            )
        })
        .collect()
}

fn env(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.into())
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr()))
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let master = env("IRIS_MASTER", "https://127.0.0.1:7443");
    let cert_dir = env("IRIS_CERT_DIR", "certs");
    let node_id = env("IRIS_NODE_ID", "node-dev-1");
    let data_addr = env("IRIS_DATA_ADDR", "0.0.0.0:7444");

    // 等待证书就绪（容器编排下启动顺序不保证）
    let p = |f: &str| format!("{cert_dir}/{f}");
    while !Path::new(&p("client.pem")).exists() {
        tracing::info!("waiting for certs...");
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    let ca_pem = std::fs::read(p("ca.pem"))?;
    let cert_pem = std::fs::read(p("client.pem"))?;
    let key_pem = std::fs::read(p("client-key.pem"))?;
    // 解析 cert NotAfter 给 heartbeat 上报 + renew task 触发判断。
    let cert_not_after = Arc::new(AtomicI64::new(
        iris_common::cert_not_after_ms(&cert_pem).unwrap_or(0),
    ));
    tracing::info!(
        cert_not_after_ms = cert_not_after.load(Ordering::Relaxed),
        "cert loaded"
    );
    // tonic 用（保持 gRPC 数据面 + 控制面兼容）
    let ca = Certificate::from_pem(&ca_pem);
    let identity = Identity::from_pem(&cert_pem, &key_pem);
    // SNI 用 master 稳定身份名（master server.pem SAN v2 含 iris-master + localhost 兼容）。
    // node → node gRPC 的 SNI 在 dataplane::connect_dataplane per-call override 为对方 node_id。
    let tls_client = ClientTlsConfig::new()
        .ca_certificate(ca.clone())
        .identity(identity.clone())
        .domain_name("iris-master");
    let dp_tls = ServerTlsConfig::new()
        .identity(identity)
        .client_ca_root(ca);
    // raw_tunnel 用（rustls 原生，bypass HTTP/2 framing）
    let (raw_server_cfg, raw_client_cfg) =
        raw_tunnel::build_configs(&ca_pem, &cert_pem, &key_pem)?;
    let raw_acceptor = tokio_rustls::TlsAcceptor::from(raw_server_cfg.clone());
    let raw_connector = tokio_rustls::TlsConnector::from(raw_client_cfg.clone());

    // 连 master（重试直到就绪）
    let channel = loop {
        match Endpoint::from_shared(master.clone())
            .and_then(|e| e.tls_config(tls_client.clone()))
        {
            Ok(ep) => match ep.connect().await {
                Ok(c) => break c,
                Err(e) => tracing::warn!(error = %e, "connect failed, retry 2s"),
            },
            Err(e) => tracing::warn!(error = %e, "endpoint config error, retry 2s"),
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    };
    let mut client = ControlClient::new(channel);
    tracing::info!(%master, %node_id, "connected (mTLS)");

    // 首次拉配置：得转发规则 + 全量节点地址表
    let reply = client
        .sync_config(SyncRequest { node_id: node_id.clone() })
        .await?
        .into_inner();
    tracing::info!(forwards = reply.forwards.len(), nodes = reply.nodes.len(), "config synced");
    // 解析 data_addr 用于 QUIC bind（QUIC 端口 = TCP 端口 + 2）
    let dp_addr_early: SocketAddr = data_addr.parse()?;
    let quic_bind = SocketAddr::new(dp_addr_early.ip(), dp_addr_early.port() + 2);
    let (quic_endpoint, quic_client_cfg) =
        quic_tunnel::make_endpoints(quic_bind, raw_server_cfg, raw_client_cfg)?;
    tracing::info!(%quic_bind, "quic_tunnel endpoint ready (UDP, mTLS, datagram)");

    let ctx = Arc::new(NodeCtx {
        nodes: RwLock::new(build_nodes(&reply.nodes)),
        tls_client,
        raw_connector: raw_connector.clone(),
        quic_endpoint: quic_endpoint.clone(),
        quic_client_cfg: Arc::new(quic_client_cfg),
    });

    // LB 由 DataPlane server 与入口共享（中转节点也需 LB 选下一跳）
    let lb = Arc::new(LoadBalancer::new());

    // 出口 target 路由（按 forward_id 维护加权 RR 游标）
    let target_router = Arc::new(dataplane::TargetRouter::new());

    // gRPC 数据面服务（端口 7444，被 master ProbeReach 调用 + raw 不可用时的 fallback）
    let dp_addr: SocketAddr = data_addr.parse()?;
    {
        let (ctx, lb, target_router) = (ctx.clone(), lb.clone(), target_router.clone());
        tokio::spawn(async move {
            let svc = DataPlaneSvc { ctx, lb, target_router };
            if let Err(e) = Server::builder()
                .tls_config(dp_tls)
                .expect("dp tls")
                .add_service(DataPlaneServer::new(svc))
                .serve(dp_addr)
                .await
            {
                tracing::error!(error = %e, "dataplane server exited");
            }
        });
        tracing::info!(%data_addr, "dataplane listening (mTLS, gRPC)");
    }

    // raw_tunnel 数据面服务（端口 = grpc + 1，TCP forward；Phase 9a）
    let raw_addr = SocketAddr::new(dp_addr.ip(), dp_addr.port() + 1);
    {
        let (ctx, lb, target_router) = (ctx.clone(), lb.clone(), target_router.clone());
        let acceptor = raw_acceptor.clone();
        let connector = raw_connector.clone();
        tokio::spawn(async move {
            if let Err(e) =
                raw_tunnel::serve(raw_addr, acceptor, connector, ctx, lb, target_router).await
            {
                tracing::error!(error = %e, "raw_tunnel server exited");
            }
        });
    }

    // QUIC 数据面服务（端口 = grpc + 2，UDP forward；Phase 9c — datagram extension 避免 TCP backpressure）
    {
        let (ctx, lb, target_router) = (ctx.clone(), lb.clone(), target_router.clone());
        let ep = quic_endpoint.clone();
        let cc = ctx.quic_client_cfg.clone();
        tokio::spawn(async move {
            if let Err(e) = quic_tunnel::serve(ep, cc, ctx, lb, target_router).await {
                tracing::error!(error = %e, "quic_tunnel server exited");
            }
        });
    }

    // Cert 自动续签 background task：每 ~1h 检查 cert NotAfter，剩 ≤30 天触发 RenewCert RPC，
    // 成功后原子替换 client.pem + client-key.pem → process exit(0) 让 systemd 重启加载新 cert。
    // 节点自身哈希派生 ±60s 抖动避开羊群效应。IRIS_AUTO_RENEW=0 关闭。
    if env("IRIS_AUTO_RENEW", "1") != "0" {
        let mut renew_client = client.clone();
        let cert_not_after = cert_not_after.clone();
        let cert_dir = cert_dir.clone();
        let node_id_t = node_id.clone();
        let jitter_secs: u64 = {
            let h: u64 = node_id_t.bytes().fold(0u64, |a, b| a.wrapping_add(b as u64));
            3600 + (h % 120).saturating_sub(60)
        };
        tokio::spawn(async move {
            // 启动后等 60s 让首轮 heartbeat + sync_config 走完
            tokio::time::sleep(Duration::from_secs(60)).await;
            loop {
                tokio::time::sleep(Duration::from_secs(jitter_secs)).await;
                let not_after = cert_not_after.load(Ordering::Relaxed);
                if not_after <= 0 {
                    continue;
                }
                let now = iris_common::now_ms();
                let remaining_days = (not_after - now) / (24 * 3600 * 1000);
                if remaining_days > 30 {
                    tracing::debug!(remaining_days, "cert 充足，跳过续签");
                    continue;
                }
                tracing::info!(remaining_days, "cert 临近过期，触发 RenewCert");
                match renew_client
                    .renew_cert(RenewCertRequest { node_id: node_id_t.clone() })
                    .await
                {
                    Ok(resp) => {
                        let r = resp.into_inner();
                        let cert_path = format!("{cert_dir}/client.pem");
                        let key_path = format!("{cert_dir}/client-key.pem");
                        let cert_tmp = format!("{cert_path}.new");
                        let key_tmp = format!("{key_path}.new");
                        if let Err(e) = std::fs::write(&cert_tmp, &r.cert_pem)
                            .and_then(|_| std::fs::write(&key_tmp, &r.key_pem))
                            .and_then(|_| std::fs::rename(&cert_tmp, &cert_path))
                            .and_then(|_| std::fs::rename(&key_tmp, &key_path))
                        {
                            tracing::error!(error = %e, "renew_cert: 写新 cert/key 失败，下轮重试");
                            continue;
                        }
                        tracing::warn!(
                            valid_until_ms = r.valid_until_ms,
                            "cert 续签成功，进程退出由 systemd 重启加载新 cert"
                        );
                        std::process::exit(0);
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "renew_cert RPC 失败，1h 后重试");
                    }
                }
            }
        });
    }

    // #36 会话级历史记录：节点全局 session table，每条 TCP 入口连接对应一个 SessionState。
    // heartbeat 时 snapshot_and_gc 上报 master；master 端 upsert by id 累计/标记关闭。
    let session_table = session::SessionTable::new();
    let entry_node_id_arc = Arc::new(node_id.clone());

    // 启动入口监听器 + 后续 sync_config 时 reconcile（热加载，无需 restart node）
    let mut active_forwards: HashMap<i64, ActiveForward> = HashMap::new();
    reconcile_forwards(
        &reply.forwards,
        &mut active_forwards,
        &node_id,
        &ctx,
        &lb,
        &target_router,
        &session_table,
        &entry_node_id_arc,
    )
    .await;

    // 心跳循环：2s 一次（含 session_events 上报，节奏决定历史/活跃数据的实时性下限）。
    // 同时刷新节点视图 + 同步 forward listener 状态。
    let mut seq = 0u64;
    let mut tick = tokio::time::interval(Duration::from_secs(2));
    loop {
        tick.tick().await;
        seq += 1;
        let listener_states = collect_listener_states(&active_forwards);
        let traffic_stats = collect_traffic_stats(&active_forwards);
        if let Err(e) = client
            .heartbeat(HeartbeatRequest {
                node_id: node_id.clone(),
                seq,
                load: 0.0,
                listener_states,
                traffic_stats,
                cert_not_after_ms: cert_not_after.load(Ordering::Relaxed),
                session_events: session_table.snapshot_and_gc(),
                advertised_addr: data_addr.clone(),
            })
            .await
        {
            tracing::warn!(seq, error = %e, "heartbeat failed");
        }
        if let Ok(r) = client
            .sync_config(SyncRequest { node_id: node_id.clone() })
            .await
        {
            let reply = r.into_inner();
            *ctx.nodes.write().unwrap() = build_nodes(&reply.nodes);
            reconcile_forwards(
                &reply.forwards,
                &mut active_forwards,
                &node_id,
                &ctx,
                &lb,
                &target_router,
                &session_table,
                &entry_node_id_arc,
            )
            .await;
        }
    }
}
