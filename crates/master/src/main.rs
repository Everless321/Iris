mod api;
mod auth;
mod db;
mod models;
mod probe;
mod ratelimit;
mod web_assets;

use anyhow::Result;
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};
use tonic::transport::{Certificate, ClientTlsConfig, Identity, Server, ServerTlsConfig};
use tonic::{Request, Response, Status};
use iris_proto::control::control_server::{Control, ControlServer};
use iris_proto::control::{
    ForwardRule, HeartbeatReply, HeartbeatRequest, Hop as PbHop, HopNode as PbHopNode, NodeAddr,
    RenewCertReply, RenewCertRequest, SyncReply, SyncRequest, TargetEndpoint as PbTargetEndpoint,
};

use models::ForwardRow;

/// 每节点上报的 listener 状态视图。Heartbeat 写、API 读。
/// key = (node_id, forward_id) 让 list_forwards O(1) 查单 forward 在某 node 的状态。
/// 不持久化 — master 重启后第一轮心跳（≤5s）会重建。
pub type ListenerStateView = Arc<RwLock<HashMap<(String, i64), ListenerStateEntry>>>;

#[derive(Clone, Debug)]
pub struct ListenerStateEntry {
    pub ok: bool,
    pub error: String,
    pub port: u32,
    pub protocol: String,
    pub updated_at: i64,
}

/// 跟踪每 (node_id, forward_id) 上次心跳上报的 traffic 累计值。
/// 用于计算 delta = current - last（node 重启时 current<last 则 delta=current）。
/// master 重启会清空，第一轮心跳起始值即为 last，下一轮才开始算 delta（少计一轮，可接受）。
pub type TrafficLastView = Arc<RwLock<HashMap<(String, i64), (u64, u64)>>>;

fn grpc_addr() -> String {
    std::env::var("IRIS_LISTEN").unwrap_or_else(|_| "0.0.0.0:7443".into())
}
fn http_addr() -> String {
    std::env::var("IRIS_HTTP").unwrap_or_else(|_| "0.0.0.0:7080".into())
}
fn cert_dir() -> String {
    std::env::var("IRIS_CERT_DIR").unwrap_or_else(|_| "certs".into())
}
fn db_url() -> String {
    std::env::var("IRIS_DB").unwrap_or_else(|_| "sqlite://data/iris.db".into())
}
fn probe_interval() -> u64 {
    std::env::var("IRIS_PROBE_INTERVAL").ok().and_then(|s| s.parse().ok()).unwrap_or(15)
}
fn fail_threshold() -> i64 {
    std::env::var("IRIS_FAIL_THRESHOLD").ok().and_then(|s| s.parse().ok()).unwrap_or(2)
}
fn jwt_secret() -> Vec<u8> {
    // 生产建议设置 IRIS_JWT_SECRET（≥32 字节高熵）；缺失时用 OsRng 生成 32 字节临时密钥
    if let Ok(s) = std::env::var("IRIS_JWT_SECRET") {
        if s.len() < 16 {
            tracing::warn!(len = s.len(), "IRIS_JWT_SECRET 过短，建议 ≥32 字节");
        }
        return s.into_bytes();
    }
    use argon2::password_hash::rand_core::{OsRng, RngCore};
    let mut k = [0u8; 32];
    OsRng.fill_bytes(&mut k);
    tracing::warn!("IRIS_JWT_SECRET 未设置，使用进程内 32 字节随机秘钥（重启后已有 token 失效）");
    k.to_vec()
}
fn admin_bootstrap() -> Option<(String, String)> {
    let u = std::env::var("IRIS_ADMIN_USER").ok()?;
    let p = std::env::var("IRIS_ADMIN_PASS").ok()?;
    Some((u, p))
}
/// #36 session 明细保留天数。0 = 永久全量（默认）。>0 = 超期明细聚合到 hourly 后 DELETE。
fn session_retain_days() -> i64 {
    std::env::var("IRIS_SESSION_RETAIN_DAYS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}
/// #36 session hourly 聚合保留天数。0 = 永久（默认）。
fn session_hourly_retain_days() -> i64 {
    std::env::var("IRIS_SESSION_HOURLY_RETAIN_DAYS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

/// #36 retention 一次 pass：明细聚合到 hourly + DELETE 过期；hourly 表过期 DELETE。
/// 两个开关独立，0 = 跳过该层。
async fn session_retention_pass(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    let now = now_ms() as i64;
    let retain_d = session_retain_days();
    if retain_d > 0 {
        let cutoff = now - retain_d * 24 * 3600 * 1000;
        sqlx::query(
            "INSERT INTO forward_sessions_hourly \
                (forward_id, hour_start_ms, session_count, total_bytes_in, total_bytes_out, unique_clients) \
             SELECT forward_id, \
                    (opened_at_ms / 3600000) * 3600000 AS hour_start_ms, \
                    COUNT(*), SUM(bytes_in), SUM(bytes_out), COUNT(DISTINCT client_ip) \
             FROM forward_sessions \
             WHERE closed_at_ms IS NOT NULL AND closed_at_ms < ? \
             GROUP BY forward_id, hour_start_ms \
             ON CONFLICT(forward_id, hour_start_ms) DO UPDATE SET \
                session_count = session_count + excluded.session_count, \
                total_bytes_in = total_bytes_in + excluded.total_bytes_in, \
                total_bytes_out = total_bytes_out + excluded.total_bytes_out, \
                unique_clients = unique_clients + excluded.unique_clients",
        )
        .bind(cutoff)
        .execute(pool)
        .await?;
        sqlx::query(
            "DELETE FROM forward_sessions WHERE closed_at_ms IS NOT NULL AND closed_at_ms < ?",
        )
        .bind(cutoff)
        .execute(pool)
        .await?;
    }
    let hourly_d = session_hourly_retain_days();
    if hourly_d > 0 {
        let cutoff = now - hourly_d * 24 * 3600 * 1000;
        sqlx::query("DELETE FROM forward_sessions_hourly WHERE hour_start_ms < ?")
            .bind(cutoff)
            .execute(pool)
            .await?;
    }
    Ok(())
}
fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64
}

struct ControlSvc {
    pool: SqlitePool,
    listener_states: ListenerStateView,
    traffic_last: TrafficLastView,
    /// CA 目录，RenewCert 时调 sign_node_cert 用。
    cert_dir: String,
    /// SSE 实时通知通道：heartbeat 写完 session 后 send(forward_id) 让浏览器订阅者重拉。
    sessions_tx: tokio::sync::broadcast::Sender<i64>,
}

/// 从 tonic Request 的 mTLS peer cert 链中解析首张证书的 Subject CN。
/// 节点 cert CN 格式 `iris-node-<id>`（参见 common::sign_node_cert）；
/// rebrand #25 之前签发的 legacy cert CN 是 `zhuanfa-node-<id>`，
/// 容忍其作为过渡 — 让 legacy 节点也能调 RenewCert 自助升级到新 CN。
/// 解析失败或无 peer cert 返回 None。
fn peer_cn_node_id<T>(req: &Request<T>) -> Option<String> {
    let certs = req.peer_certs()?;
    let leaf = certs.first()?;
    let (_, c) = x509_parser::parse_x509_certificate(leaf.as_ref()).ok()?;
    for rdn in c.subject().iter_common_name() {
        if let Ok(cn) = rdn.as_str() {
            return cn
                .strip_prefix("iris-node-")
                .or_else(|| cn.strip_prefix("zhuanfa-node-"))
                .map(|s| s.to_string());
        }
    }
    None
}

#[tonic::async_trait]
impl Control for ControlSvc {
    async fn heartbeat(
        &self,
        req: Request<HeartbeatRequest>,
    ) -> Result<Response<HeartbeatReply>, Status> {
        let r = req.into_inner();
        tracing::info!(node = %r.node_id, seq = r.seq, load = r.load,
            listeners = r.listener_states.len(), "heartbeat");
        // 同时更新 cert_not_after_ms（节点上报）。0 = 未上报，保留旧值避免覆盖。
        let cna = r.cert_not_after_ms;
        let upd = if cna > 0 {
            sqlx::query("UPDATE nodes SET status='online', last_seen=?, cert_not_after_ms=? WHERE id=?")
                .bind(now_ms() as i64).bind(cna).bind(&r.node_id)
                .execute(&self.pool).await
        } else {
            sqlx::query("UPDATE nodes SET status='online', last_seen=? WHERE id=?")
                .bind(now_ms() as i64).bind(&r.node_id)
                .execute(&self.pool).await
        };
        match upd {
            Ok(res) if res.rows_affected() == 0 => {
                tracing::warn!(node = %r.node_id, "心跳来自未注册节点")
            }
            Ok(_) => {}
            Err(e) => tracing::error!(node = %r.node_id, error = %e, "更新节点状态失败"),
        }
        // 收集本节点上报的所有 listener_states 写入共享内存视图。
        // 同时清掉该 node 不再上报的旧条目（forward 被删/不再入口）。
        if !r.listener_states.is_empty() || !r.node_id.is_empty() {
            let now = now_ms() as i64;
            let reported: std::collections::HashSet<i64> =
                r.listener_states.iter().map(|s| s.forward_id).collect();
            let mut g = self.listener_states.write().unwrap();
            // remove stale entries for this node
            g.retain(|(nid, fid), _| nid != &r.node_id || reported.contains(fid));
            // upsert reported
            for s in &r.listener_states {
                g.insert(
                    (r.node_id.clone(), s.forward_id),
                    ListenerStateEntry {
                        ok: s.ok,
                        error: s.error.clone(),
                        port: s.port,
                        protocol: s.protocol.clone(),
                        updated_at: now,
                    },
                );
            }
        }
        // #36 会话事件：每条 TCP 连接的生命周期记录。upsert by id，closed_at_ms=0 → NULL（active）。
        // 同时收集本批 forward_id 集合用于 SSE broadcast（单次心跳同 forward 多条事件 → 一次通知）。
        if !r.session_events.is_empty() {
            let mut touched_forwards: std::collections::HashSet<i64> =
                std::collections::HashSet::new();
            for ev in &r.session_events {
                let hops_json = serde_json::to_string(&ev.hops_path).unwrap_or_else(|_| "[]".into());
                let closed: Option<i64> =
                    if ev.closed_at_ms > 0 { Some(ev.closed_at_ms) } else { None };
                let reason: Option<&str> =
                    if ev.close_reason.is_empty() { None } else { Some(ev.close_reason.as_str()) };
                if let Err(e) = sqlx::query(
                    "INSERT INTO forward_sessions (id, forward_id, entry_node_id, client_ip, \
                     client_port, target_addr, hops_path, protocol, opened_at_ms, \
                     closed_at_ms, bytes_in, bytes_out, close_reason) \
                     VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?) \
                     ON CONFLICT(id) DO UPDATE SET \
                       closed_at_ms = excluded.closed_at_ms, \
                       bytes_in = excluded.bytes_in, \
                       bytes_out = excluded.bytes_out, \
                       close_reason = excluded.close_reason",
                )
                .bind(&ev.id)
                .bind(ev.forward_id)
                .bind(&ev.entry_node_id)
                .bind(&ev.client_ip)
                .bind(ev.client_port as i64)
                .bind(&ev.target_addr)
                .bind(&hops_json)
                .bind(&ev.protocol)
                .bind(ev.opened_at_ms)
                .bind(closed)
                .bind(ev.bytes_in as i64)
                .bind(ev.bytes_out as i64)
                .bind(reason)
                .execute(&self.pool)
                .await
                {
                    tracing::warn!(session_id = %ev.id, error = %e, "session upsert failed");
                } else {
                    touched_forwards.insert(ev.forward_id);
                }
            }
            // 仅 DB 写入成功的 forward_id 才广播；无订阅者时 send 返回 Err 也忽略。
            for fid in touched_forwards {
                let _ = self.sessions_tx.send(fid);
            }
        }
        // 流量 delta 累加到 forwards 表。current<last 视作 node 重启 → delta=current。
        // node 多入口 (LB 同一 forward 多节点 entry) → 各节点独立 last，加和累计无重复。
        if !r.traffic_stats.is_empty() {
            let mut deltas: Vec<(i64, u64, u64)> = Vec::with_capacity(r.traffic_stats.len());
            {
                let mut g = self.traffic_last.write().unwrap();
                for s in &r.traffic_stats {
                    let key = (r.node_id.clone(), s.forward_id);
                    let (last_in, last_out) = g.get(&key).copied().unwrap_or((0, 0));
                    let din = if s.bytes_in >= last_in { s.bytes_in - last_in } else { s.bytes_in };
                    let dout = if s.bytes_out >= last_out { s.bytes_out - last_out } else { s.bytes_out };
                    g.insert(key, (s.bytes_in, s.bytes_out));
                    if din > 0 || dout > 0 {
                        deltas.push((s.forward_id, din, dout));
                    }
                }
            }
            for (fid, din, dout) in deltas {
                if let Err(e) = sqlx::query(
                    "UPDATE forwards SET bytes_in = bytes_in + ?, bytes_out = bytes_out + ? WHERE id = ?",
                )
                .bind(din as i64).bind(dout as i64).bind(fid)
                .execute(&self.pool).await {
                    tracing::warn!(forward_id = fid, error = %e, "traffic delta UPDATE failed");
                }
            }
        }
        Ok(Response::new(HeartbeatReply {
            server_time: now_ms(),
            message: format!("ack seq={}", r.seq),
        }))
    }

    async fn sync_config(
        &self,
        req: Request<SyncRequest>,
    ) -> Result<Response<SyncReply>, Status> {
        let nid = req.into_inner().node_id;
        let rows = sqlx::query_as::<_, ForwardRow>("SELECT * FROM forwards WHERE enabled=1")
            .fetch_all(&self.pool)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;
        let forwards = rows
            .into_iter()
            .filter_map(|r| {
                let fid = r.id;
                let hops = r.hops();
                if hops.is_empty() {
                    tracing::warn!(forward_id = fid, "转发规则 hops 为空（可能 JSON 损坏），跳过下发");
                    return None;
                }
                let is_entry = hops
                    .first()
                    .map(|h| h.nodes.iter().any(|n| n.id == nid))
                    .unwrap_or(false);
                if !is_entry {
                    return None;
                }
                let pb_hops = hops
                    .into_iter()
                    .map(|h| PbHop {
                        strategy: h.strategy,
                        nodes: h
                            .nodes
                            .into_iter()
                            .map(|n| PbHopNode { id: n.id, weight: n.weight })
                            .collect(),
                    })
                    .collect();
                let targets = r.targets();
                let pb_targets: Vec<PbTargetEndpoint> = targets
                    .iter()
                    .map(|t| PbTargetEndpoint { addr: t.addr.clone(), weight: t.weight })
                    .collect();
                // 兼容字段：把第一个 target 也填到旧 string，便于过渡（节点优先用 targets）
                let legacy_target = targets.first().map(|t| t.addr.clone()).unwrap_or_default();
                let strategy = if r.target_strategy.is_empty() { "weighted".into() } else { r.target_strategy.clone() };
                #[allow(deprecated)]
                Some(ForwardRule {
                    id: r.id,
                    listen_port: r.listen_port as u32,
                    protocol: r.protocol,
                    hops: pb_hops,
                    target: legacy_target,
                    enabled: true,
                    targets: pb_targets,
                    target_strategy: strategy,
                })
            })
            .collect();

        let nodes = sqlx::query_as::<_, (String, String, String, Option<i64>)>(
            "SELECT id, addr, health, latency_ms FROM nodes",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| Status::internal(e.to_string()))?
        .into_iter()
        .map(|(id, addr, health, latency)| NodeAddr {
            id,
            addr,
            health,
            latency_ms: latency.unwrap_or(0) as u32,
        })
        .collect();

        Ok(Response::new(SyncReply { forwards, nodes }))
    }

    /// 节点 cert 临近过期时主动调用：用现有 mTLS cert 认证，
    /// master 校验 peer cert CN == request.node_id 后用同一 CA 签发新 cert。
    /// 失败统一返回 PermissionDenied，避免泄露内部状态。
    async fn renew_cert(
        &self,
        req: Request<RenewCertRequest>,
    ) -> Result<Response<RenewCertReply>, Status> {
        let cn_node = peer_cn_node_id(&req)
            .ok_or_else(|| Status::permission_denied("missing or invalid peer cert"))?;
        let r = req.into_inner();
        if r.node_id.is_empty() || r.node_id != cn_node {
            tracing::warn!(claimed = %r.node_id, peer_cn = %cn_node, "renew_cert: node_id 与 peer cert CN 不匹配");
            return Err(Status::permission_denied("node_id mismatch with peer cert"));
        }
        let exists: i64 = sqlx::query_scalar("SELECT count(*) FROM nodes WHERE id=?")
            .bind(&r.node_id).fetch_one(&self.pool).await
            .map_err(|e| Status::internal(e.to_string()))?;
        if exists == 0 {
            return Err(Status::permission_denied("node not registered"));
        }
        let (cert_pem, key_pem, _ca_pem) =
            iris_common::sign_node_cert(&self.cert_dir, &r.node_id)
                .map_err(|e| Status::internal(format!("sign: {e}")))?;
        let valid_until_ms = iris_common::cert_not_after_ms(cert_pem.as_bytes())
            .map_err(|e| Status::internal(format!("parse new cert: {e}")))?;
        tracing::info!(node = %r.node_id, valid_until_ms, "renew_cert: issued new cert");
        Ok(Response::new(RenewCertReply { cert_pem, key_pem, valid_until_ms }))
    }
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

    // feature unification 后 rustls 同时启用了 ring + aws_lc_rs（来自 tonic 间接 + node 切换），
    // 不显式 install 会 panic。选 aws_lc_rs（AES-NI 加速，与 node 一致）。
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    std::fs::create_dir_all("data").ok();
    let pool = db::init(&db_url()).await?;
    tracing::info!(url = %db_url(), "sqlite ready (migrated)");

    // 首次启动：根据 env 引导 admin 账号（与 register 端点同样强制 ≥6 字符密码）
    if let Some((u, p)) = admin_bootstrap() {
        if p.len() < 6 {
            anyhow::bail!("IRIS_ADMIN_PASS 长度 < 6，拒绝引导 admin");
        }
        let exists: i64 = sqlx::query_scalar("SELECT count(*) FROM users WHERE username=?")
            .bind(&u).fetch_one(&pool).await?;
        if exists == 0 {
            let hash = auth::hash_password(&p)?;
            let now = now_ms() as i64;
            sqlx::query(
                "INSERT INTO users (username, password_hash, role, created_at, updated_at) \
                 VALUES (?,?,'admin',?,?)",
            )
            .bind(&u).bind(&hash).bind(now).bind(now)
            .execute(&pool).await?;
            tracing::info!(user = %u, "bootstrap admin created");
        }
    }

    // 健康探测调度器
    probe::spawn(pool.clone(), probe_interval(), fail_threshold());

    // #36 session retention cron：每 1h 跑一次。两个 env 默认 0 = 永久不归档。
    {
        let pool = pool.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(3600));
            // 首次 tick 立刻触发；防止启动时立即跑（让 master 稳一会儿）
            tick.tick().await;
            tracing::info!(
                retain_days = session_retain_days(),
                hourly_retain_days = session_hourly_retain_days(),
                "session retention cron started"
            );
            loop {
                tick.tick().await;
                if let Err(e) = session_retention_pass(&pool).await {
                    tracing::warn!(error = %e, "session retention pass failed");
                }
            }
        });
    }

    // 预先准备 mTLS client config，让 master 能反向调用节点 DataPlane（用于链路测试 ProbeReach）
    let dir = cert_dir();
    let paths = iris_common::ensure_dev_certs(&dir)?;
    let node_caller_tls = ClientTlsConfig::new()
        .ca_certificate(Certificate::from_pem(std::fs::read(&paths.ca)?))
        .identity(Identity::from_pem(
            std::fs::read(&paths.client)?,
            std::fs::read(&paths.client_key)?,
        ))
        .domain_name("localhost");

    // listener 状态共享视图：heartbeat 写 / API 读
    let listener_states: ListenerStateView = Arc::new(RwLock::new(HashMap::new()));
    // 流量 last_reported 视图：仅 heartbeat 内部使用（不需要 API 暴露）
    let traffic_last: TrafficLastView = Arc::new(RwLock::new(HashMap::new()));
    // SSE 通知通道：heartbeat 端 send，HTTP SSE 端 subscribe。容量 256 足够吸收
    // 4 节点突发心跳；超过时旧消息丢弃（订阅者 Lagged → 下次 send 仍能触发 refresh）。
    let (sessions_tx, _) = tokio::sync::broadcast::channel::<i64>(256);

    // HTTP 控制 API
    let auth_state = auth::AuthState::new(&jwt_secret());
    let app = api::router(api::AppState {
        pool: pool.clone(),
        auth: auth_state,
        // login: 同 IP 5次/分钟；register: 同 IP 3次/小时
        login_rl: std::sync::Arc::new(ratelimit::RateLimiter::new(
            std::time::Duration::from_secs(60), 5,
        )),
        register_rl: std::sync::Arc::new(ratelimit::RateLimiter::new(
            std::time::Duration::from_secs(3600), 3,
        )),
        cert_dir: cert_dir(),
        node_caller_tls,
        listener_states: listener_states.clone(),
        sessions_tx: sessions_tx.clone(),
    });
    let http_listener = tokio::net::TcpListener::bind(http_addr()).await?;
    tracing::info!(addr = %http_addr(), "http api listening");
    // 注入 ConnectInfo 以便限速器能取到客户端 IP
    let http_task = tokio::spawn(async move {
        axum::serve(
            http_listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
    });

    // gRPC 控制面（mTLS）— 复用前面已加载的 paths
    let identity = Identity::from_pem(
        std::fs::read(&paths.server)?,
        std::fs::read(&paths.server_key)?,
    );
    let client_ca = Certificate::from_pem(std::fs::read(&paths.ca)?);
    let tls = ServerTlsConfig::new()
        .identity(identity)
        .client_ca_root(client_ca);
    let svc = ControlSvc {
        pool,
        listener_states: listener_states.clone(),
        traffic_last,
        cert_dir: cert_dir(),
        sessions_tx,
    };
    let addr = grpc_addr().parse()?;
    tracing::info!(%addr, "grpc control listening (mTLS)");

    let grpc = Server::builder()
        .tls_config(tls)?
        .add_service(ControlServer::new(svc))
        .serve(addr);

    tokio::select! {
        r = grpc => r?,
        r = http_task => { r??; }
    }
    Ok(())
}
