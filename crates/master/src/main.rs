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
    SyncReply, SyncRequest, TargetEndpoint as PbTargetEndpoint,
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
fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64
}

struct ControlSvc {
    pool: SqlitePool,
    listener_states: ListenerStateView,
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
        match sqlx::query("UPDATE nodes SET status='online', last_seen=? WHERE id=?")
            .bind(now_ms() as i64)
            .bind(&r.node_id)
            .execute(&self.pool)
            .await
        {
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
    let svc = ControlSvc { pool, listener_states: listener_states.clone() };
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
