mod api;
mod auth;
mod db;
mod models;
mod probe;
mod web_assets;

use anyhow::Result;
use sqlx::SqlitePool;
use std::time::{SystemTime, UNIX_EPOCH};
use tonic::transport::{Certificate, Identity, Server, ServerTlsConfig};
use tonic::{Request, Response, Status};
use zhuanfa_proto::control::control_server::{Control, ControlServer};
use zhuanfa_proto::control::{
    ForwardRule, HeartbeatReply, HeartbeatRequest, Hop as PbHop, HopNode as PbHopNode, NodeAddr,
    SyncReply, SyncRequest,
};

use models::ForwardRow;

fn grpc_addr() -> String {
    std::env::var("ZF_LISTEN").unwrap_or_else(|_| "0.0.0.0:7443".into())
}
fn http_addr() -> String {
    std::env::var("ZF_HTTP").unwrap_or_else(|_| "0.0.0.0:7080".into())
}
fn cert_dir() -> String {
    std::env::var("ZF_CERT_DIR").unwrap_or_else(|_| "certs".into())
}
fn db_url() -> String {
    std::env::var("ZF_DB").unwrap_or_else(|_| "sqlite://data/zhuanfa.db".into())
}
fn probe_interval() -> u64 {
    std::env::var("ZF_PROBE_INTERVAL").ok().and_then(|s| s.parse().ok()).unwrap_or(15)
}
fn fail_threshold() -> i64 {
    std::env::var("ZF_FAIL_THRESHOLD").ok().and_then(|s| s.parse().ok()).unwrap_or(2)
}
fn jwt_secret() -> Vec<u8> {
    // P4 简单方案：env 提供秘钥；缺失则警告并使用临时随机值（重启后旧 token 失效）
    if let Ok(s) = std::env::var("ZF_JWT_SECRET") {
        return s.into_bytes();
    }
    tracing::warn!("ZF_JWT_SECRET 未设置，使用进程内随机秘钥（重启后已有 token 失效）");
    uuid::Uuid::new_v4().as_bytes().to_vec()
}
fn admin_bootstrap() -> Option<(String, String)> {
    let u = std::env::var("ZF_ADMIN_USER").ok()?;
    let p = std::env::var("ZF_ADMIN_PASS").ok()?;
    Some((u, p))
}
fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64
}

struct ControlSvc {
    pool: SqlitePool,
}

#[tonic::async_trait]
impl Control for ControlSvc {
    async fn heartbeat(
        &self,
        req: Request<HeartbeatRequest>,
    ) -> Result<Response<HeartbeatReply>, Status> {
        let r = req.into_inner();
        tracing::info!(node = %r.node_id, seq = r.seq, load = r.load, "heartbeat");
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
                Some(ForwardRule {
                    id: r.id,
                    listen_port: r.listen_port as u32,
                    protocol: r.protocol,
                    hops: pb_hops,
                    target: r.target,
                    enabled: true,
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

    std::fs::create_dir_all("data").ok();
    let pool = db::init(&db_url()).await?;
    tracing::info!(url = %db_url(), "sqlite ready (migrated)");

    // 首次启动：根据 env 引导 admin 账号
    if let Some((u, p)) = admin_bootstrap() {
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

    // HTTP 控制 API
    let auth_state = auth::AuthState::new(&jwt_secret());
    let app = api::router(api::AppState { pool: pool.clone(), auth: auth_state });
    let http_listener = tokio::net::TcpListener::bind(http_addr()).await?;
    tracing::info!(addr = %http_addr(), "http api listening");
    let http_task = tokio::spawn(async move { axum::serve(http_listener, app).await });

    // gRPC 控制面（mTLS）
    let dir = cert_dir();
    let paths = zhuanfa_common::ensure_dev_certs(&dir)?;
    let identity = Identity::from_pem(
        std::fs::read(&paths.server)?,
        std::fs::read(&paths.server_key)?,
    );
    let client_ca = Certificate::from_pem(std::fs::read(&paths.ca)?);
    let tls = ServerTlsConfig::new()
        .identity(identity)
        .client_ca_root(client_ca);
    let svc = ControlSvc { pool };
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
