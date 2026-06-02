use axum::{
    extract::{ConnectInfo, FromRef, Path, Query, State},
    http::StatusCode,
    response::sse::{Event, KeepAlive, Sse},
    routing::{get, post, put},
    Json, Router,
};
use futures::stream::Stream;
use tower_http::compression::CompressionLayer;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::broadcast;
use tokio_stream::{wrappers::BroadcastStream, StreamExt};
use tonic::transport::{ClientTlsConfig, Endpoint};
use iris_proto::control::data_plane_client::DataPlaneClient;
use iris_proto::control::ProbeReachRequest;

use crate::auth::{hash_password, issue_token, verify_password, AdminClaims, AuthState, Claims};
use std::sync::Mutex;
use uuid::Uuid;
use crate::models::{
    parse_protocol, AuthResponse, EnrollRequest, EnrollResponse, EnrollmentToken, Forward,
    ForwardCreate, ForwardRow, Hop, InviteCode, LoginRequest, Node, NodeCreate, RegisterRequest,
    TargetEndpoint, UserDto, UserRow,
};
use crate::ratelimit::RateLimiter;

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub auth: AuthState,
    pub login_rl: Arc<RateLimiter>,
    pub register_rl: Arc<RateLimiter>,
    pub cert_dir: String, // master 证书目录，供节点 enrollment 时签发新证书
    pub node_caller_tls: ClientTlsConfig, // master 反向调用节点 DataPlane 用的 mTLS
    pub listener_states: crate::ListenerStateView, // heartbeat 上报的 per-(node,forward) listener 状态
    /// SSE 实时通道：heartbeat 写完 session 后 send(forward_id)，SSE 订阅者 filter 后 push 给浏览器。
    /// 容量 256：突发 session 风暴时旧消息丢弃（订阅者收 Lagged）— UI 拿到任意 ping 就重拉，幂等。
    pub sessions_tx: broadcast::Sender<i64>,
    /// SSE 短期单用 ticket：避免在 EventSource URL 上裸传 JWT (会进 access log / browser history)。
    /// UI 先 POST /api/forwards/:id/sse-ticket（用 Authorization header）换 60s 一次性 ticket，
    /// 再用 ticket 开 EventSource。消费一次即从 map 移除，过期清理在每次发新 ticket 时顺手做。
    pub sse_tickets: Arc<Mutex<HashMap<String, SseTicketEntry>>>,
}

#[derive(Clone)]
pub struct SseTicketEntry {
    pub forward_id: i64,
    pub exp_ms: i64,
}

impl FromRef<AppState> for AuthState {
    fn from_ref(s: &AppState) -> AuthState {
        s.auth.clone()
    }
}
impl FromRef<AppState> for SqlitePool {
    fn from_ref(s: &AppState) -> SqlitePool {
        s.pool.clone()
    }
}

pub fn router(state: AppState) -> Router {
    Router::new()
        // 公共
        .route("/api/auth/register", post(register))
        .route("/api/auth/login", post(login))
        .route("/api/me", get(me))
        // 节点：admin only
        .route("/api/nodes", get(list_nodes).post(create_node))
        .route("/api/nodes/:id", axum::routing::delete(delete_node))
        .route("/api/nodes/:id/enrollment", post(create_enrollment))
        .route("/api/nodes/enroll", post(enroll_node))
        .route("/install.sh", get(install_script))
        // 转发：customer 仅看/改自己；admin 全权
        .route("/api/forwards", get(list_forwards).post(create_forward))
        .route("/api/forwards/test", post(test_forward))
        .route(
            "/api/forwards/:id",
            put(update_forward).delete(delete_forward),
        )
        // #36 会话级历史
        .route("/api/forwards/:id/sessions", get(list_forward_sessions))
        .route("/api/forwards/:id/sessions/active", get(list_active_sessions))
        .route("/api/forwards/:id/sse-ticket", post(issue_sse_ticket))
        .route("/api/forwards/:id/sessions/stream", get(sessions_stream))
        .route("/api/sessions/:id", get(get_session))
        // 邀请码 & 用户管理：admin only
        .route("/api/invites", get(list_invites).post(create_invite))
        .route("/api/users", get(list_users))
        // SLA / 监控
        .route("/api/sla", get(sla))
        .route("/api/sla/samples", get(sla_samples))
        .route("/metrics", get(metrics))
        .route("/healthz", get(|| async { "ok" }))
        // 前端静态资源：SPA 兜底，必须放在所有 /api/* 之后
        .route("/", get(|| crate::web_assets::handler(None)))
        .route("/*path", get(crate::web_assets::handler))
        .with_state(state)
        // gzip/br/deflate 压缩（按 Accept-Encoding 自动选）。1MB JS bundle 压成 ~250KB，
        // 跨大区/限速出口下 web UI 首次加载 3-5x 提速。layer 在 with_state 后，作用全 router。
        .layer(CompressionLayer::new())
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as i64
}

type ApiErr = (StatusCode, String);
/// 对外仅返回通用错误，详情写日志。避免泄露 SQL 错误、文件路径、内部栈。
fn err<E: std::fmt::Display>(e: E) -> ApiErr {
    tracing::error!(detail = %e, "internal error");
    (StatusCode::INTERNAL_SERVER_ERROR, "internal error".into())
}
fn bad(msg: &str) -> ApiErr {
    (StatusCode::BAD_REQUEST, msg.into())
}
fn client_ip(addr: SocketAddr) -> String {
    addr.ip().to_string()
}

// ---- 鉴权端点 ----

async fn register(
    State(s): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(r): Json<RegisterRequest>,
) -> Result<Json<AuthResponse>, ApiErr> {
    if !s.register_rl.check(&client_ip(addr)) {
        return Err((StatusCode::TOO_MANY_REQUESTS, "请求过于频繁，请稍后重试".into()));
    }
    if r.username.trim().len() < 3 || r.password.len() < 6 {
        return Err(bad("用户名 ≥3 字符，密码 ≥6 字符"));
    }
    // 校验邀请码并标记已用（事务）
    let mut tx = s.pool.begin().await.map_err(err)?;
    let code = sqlx::query_as::<_, InviteCode>("SELECT * FROM invite_codes WHERE code=?")
        .bind(&r.invite_code)
        .fetch_optional(&mut *tx)
        .await
        .map_err(err)?
        .ok_or((StatusCode::BAD_REQUEST, "邀请码无效".into()))?;
    if code.used_by.is_some() {
        return Err(bad("邀请码已被使用"));
    }
    let hash = hash_password(&r.password).map_err(err)?;
    let now = now_ms();
    let user_id: i64 = sqlx::query_scalar(
        "INSERT INTO users (username, password_hash, role, created_at, updated_at) \
         VALUES (?,?,'customer',?,?) RETURNING id",
    )
    .bind(&r.username)
    .bind(&hash)
    .bind(now)
    .bind(now)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| {
        if e.to_string().contains("UNIQUE") {
            (StatusCode::CONFLICT, "用户名已存在".into())
        } else {
            err(e)
        }
    })?;
    sqlx::query("UPDATE invite_codes SET used_by=?, used_at=? WHERE code=?")
        .bind(user_id)
        .bind(now)
        .bind(&r.invite_code)
        .execute(&mut *tx)
        .await
        .map_err(err)?;
    tx.commit().await.map_err(err)?;
    let token = issue_token(&s.auth, user_id, &r.username, "customer").map_err(err)?;
    Ok(Json(AuthResponse {
        token,
        user: UserDto { id: user_id, username: r.username, role: "customer".into(), created_at: now },
    }))
}

async fn login(
    State(s): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(r): Json<LoginRequest>,
) -> Result<Json<AuthResponse>, ApiErr> {
    if !s.login_rl.check(&client_ip(addr)) {
        return Err((StatusCode::TOO_MANY_REQUESTS, "登录过于频繁，请稍后重试".into()));
    }
    let u = sqlx::query_as::<_, UserRow>("SELECT * FROM users WHERE username=?")
        .bind(&r.username)
        .fetch_optional(&s.pool)
        .await
        .map_err(err)?
        .ok_or((StatusCode::UNAUTHORIZED, "用户名或密码错误".into()))?;
    if !verify_password(&r.password, &u.password_hash) {
        return Err((StatusCode::UNAUTHORIZED, "用户名或密码错误".into()));
    }
    let token = issue_token(&s.auth, u.id, &u.username, &u.role).map_err(err)?;
    Ok(Json(AuthResponse { token, user: u.into() }))
}

async fn me(claims: Claims) -> Json<Value> {
    Json(json!({
        "id": claims.sub,
        "username": claims.username,
        "role": claims.role,
    }))
}

// ---- 节点：admin only ----

async fn list_nodes(_: AdminClaims, State(s): State<AppState>) -> Result<Json<Vec<Node>>, ApiErr> {
    let rows = sqlx::query_as::<_, Node>("SELECT * FROM nodes ORDER BY created_at")
        .fetch_all(&s.pool)
        .await
        .map_err(err)?;
    Ok(Json(rows))
}

async fn create_node(
    _: AdminClaims,
    State(s): State<AppState>,
    Json(n): Json<NodeCreate>,
) -> Result<Json<Node>, ApiErr> {
    let now = now_ms();
    sqlx::query(
        "INSERT INTO nodes (id,name,addr,status,weight,created_at) VALUES (?,?,?,'offline',?,?)",
    )
    .bind(&n.id)
    .bind(&n.name)
    .bind(&n.addr)
    .bind(n.weight)
    .bind(now)
    .execute(&s.pool)
    .await
    .map_err(err)?;
    Ok(Json(Node {
        id: n.id, name: n.name, addr: n.addr, status: "offline".into(), last_seen: None,
        created_at: now, weight: n.weight, health: "unknown".into(), latency_ms: None,
        fail_count: 0, probe_total: 0, probe_ok: 0, fail_events: 0, down_since: None, downtime_ms: 0,
        cert_not_after_ms: 0,
    }))
}

async fn delete_node(
    _: AdminClaims,
    State(s): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiErr> {
    // 检测是否有转发引用该节点（防止 forwards.path JSON 残留幽灵节点）
    let refs: Vec<(i64, String)> =
        sqlx::query_as("SELECT id, name FROM forwards WHERE path LIKE ?")
            .bind(format!("%\"{}\"%", id))
            .fetch_all(&s.pool).await.map_err(err)?;
    if !refs.is_empty() {
        let names: Vec<String> = refs.iter().map(|(id, n)| format!("#{id} {n}")).collect();
        return Err((
            StatusCode::CONFLICT,
            format!("还有 {} 条转发引用该节点：{}", refs.len(), names.join(", ")),
        ));
    }
    sqlx::query("DELETE FROM nodes WHERE id=?").bind(id).execute(&s.pool).await.map_err(err)?;
    Ok(StatusCode::NO_CONTENT)
}

// ---- 转发：归属权限 ----

async fn list_forwards(claims: Claims, State(s): State<AppState>) -> Result<Json<Vec<Forward>>, ApiErr> {
    let rows = if claims.is_admin() {
        sqlx::query_as::<_, ForwardRow>("SELECT * FROM forwards ORDER BY id")
            .fetch_all(&s.pool).await
    } else {
        sqlx::query_as::<_, ForwardRow>("SELECT * FROM forwards WHERE owner_id=? ORDER BY id")
            .bind(claims.sub).fetch_all(&s.pool).await
    }.map_err(err)?;
    let states = s.listener_states.read().unwrap().clone();
    let mut out: Vec<Forward> = rows.into_iter().map(Forward::from).collect();
    for f in &mut out {
        // 入口节点 = hops[0].nodes — UI 只关心入口的 bind 状态
        let entry_ids: Vec<String> = f
            .hops
            .first()
            .map(|h| h.nodes.iter().map(|n| n.id.clone()).collect())
            .unwrap_or_default();
        for nid in entry_ids {
            if let Some(st) = states.get(&(nid.clone(), f.id)) {
                f.listener_status.push(crate::models::ListenerNodeStatus {
                    node_id: nid,
                    ok: st.ok,
                    error: st.error.clone(),
                    updated_at: st.updated_at,
                });
            }
        }
    }
    Ok(Json(out))
}

/// 校验 forward 入口 + 端口 + 协议是否与已 enabled 的其它 forward 冲突。
/// 冲突条件：listen_port 相同 && 协议交集非空 && hops[0] 节点交集非空。
/// 跨 owner 同样阻断 — node 端是单点 bind，跟 owner 无关。
/// `exclude_id` 用于 update 时排除自己。
async fn check_entry_port_conflict(
    pool: &sqlx::SqlitePool,
    listen_port: i64,
    protocol: &str,
    entry_node_ids: &[String],
    exclude_id: Option<i64>,
) -> Result<(), ApiErr> {
    use std::collections::HashSet;
    let new_protos: HashSet<&str> = protocol
        .split('+').map(str::trim).filter(|s| !s.is_empty()).collect();
    let entry_set: HashSet<&str> = entry_node_ids.iter().map(|s| s.as_str()).collect();
    if new_protos.is_empty() || entry_set.is_empty() {
        return Ok(());
    }
    let rows = sqlx::query_as::<_, ForwardRow>(
        "SELECT * FROM forwards WHERE enabled=1 AND listen_port=?",
    )
    .bind(listen_port)
    .fetch_all(pool)
    .await
    .map_err(err)?;
    for r in rows {
        if Some(r.id) == exclude_id {
            continue;
        }
        let r_protos: HashSet<&str> = r
            .protocol
            .split('+').map(str::trim).filter(|s| !s.is_empty()).collect();
        if new_protos.is_disjoint(&r_protos) {
            continue;
        }
        let hops = r.hops();
        let r_entry: HashSet<&str> = hops
            .first()
            .map(|h| h.nodes.iter().map(|n| n.id.as_str()).collect())
            .unwrap_or_default();
        let overlap: Vec<&str> = entry_set.intersection(&r_entry).copied().collect();
        if !overlap.is_empty() {
            let proto_overlap: Vec<&str> = new_protos.intersection(&r_protos).copied().collect();
            return Err((
                StatusCode::CONFLICT,
                format!(
                    "端口 {} 冲突：转发 '{}' (id={}) 已在节点 [{}] 使用 {} 协议",
                    listen_port,
                    r.name,
                    r.id,
                    overlap.join(","),
                    proto_overlap.join("+")
                ),
            ));
        }
    }
    Ok(())
}

async fn create_forward(
    claims: Claims,
    State(s): State<AppState>,
    Json(f): Json<ForwardCreate>,
) -> Result<Json<Forward>, ApiErr> {
    let protocol = parse_protocol(&f.protocol)
        .ok_or_else(|| bad("protocol 必须为 tcp / udp / tcp+udp 之一"))?;
    let mut hops = f.normalized_hops();
    if hops.is_empty() || hops.iter().any(|h| h.nodes.is_empty()) {
        return Err(bad("hops 不能为空，且每跳至少一个节点"));
    }
    if f.listen_port < 1 || f.listen_port > 65535 {
        return Err(bad("listen_port 必须在 1-65535"));
    }
    let mut targets = f.normalized_targets();
    if targets.is_empty() {
        return Err(bad("至少需要 1 个目标地址"));
    }
    for t in &mut targets {
        if t.addr.trim().is_empty() {
            return Err(bad("目标地址不能为空"));
        }
        t.weight = t.weight.clamp(1, 1000);
    }
    let mut seen = std::collections::HashSet::new();
    for h in &mut hops {
        for n in &mut h.nodes {
            n.weight = n.weight.clamp(1, 1000);
            if !seen.insert(n.id.clone()) {
                return Err(bad(&format!("节点 {} 在路径中重复（循环路径）", n.id)));
            }
        }
    }
    // 入口节点 + 端口 + 协议冲突校验（避免 node 端 bind 失败）
    let entry_ids: Vec<String> = hops
        .first()
        .map(|h| h.nodes.iter().map(|n| n.id.clone()).collect())
        .unwrap_or_default();
    check_entry_port_conflict(&s.pool, f.listen_port, &protocol, &entry_ids, None).await?;
    let hops_json = serde_json::to_string(&hops).map_err(err)?;
    let targets_json = serde_json::to_string(&targets).map_err(err)?;
    let now = now_ms();
    // #39 quota / rate / 重置策略：仅 admin 可设置。customer 创建时强制 None（之后 admin 来加）。
    // #27 link_encryption：admin 可设，customer 强制 'tls'。
    let (eff_qin, eff_qout, eff_rin, eff_rout, eff_qreset, eff_qreset_at, eff_link_enc) = if claims.is_admin() {
        let qr = normalize_quota_reset(f.quota_reset.as_deref());
        let qra = crate::compute_next_reset_at_ms(qr.as_deref(), now);
        (
            nz_opt(f.quota_in_bytes), nz_opt(f.quota_out_bytes),
            nz_opt(f.rate_in_bps), nz_opt(f.rate_out_bps),
            qr, qra,
            normalize_link_encryption(f.link_encryption.as_deref()),
        )
    } else {
        (None, None, None, None, None, None, "tls".into())
    };
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO forwards \
         (name,listen_port,protocol,path,target,target_strategy,enabled,created_at,owner_id, \
          quota_in_bytes,quota_out_bytes,rate_in_bps,rate_out_bps,quota_reset,quota_reset_at_ms, \
          link_encryption) \
         VALUES (?,?,?,?,?,?,1,?,?, ?,?,?,?,?,?, ?) RETURNING id",
    )
    .bind(&f.name).bind(f.listen_port).bind(&protocol).bind(&hops_json)
    .bind(&targets_json).bind(&f.target_strategy).bind(now).bind(claims.sub)
    .bind(eff_qin).bind(eff_qout)
    .bind(eff_rin).bind(eff_rout)
    .bind(&eff_qreset).bind(eff_qreset_at)
    .bind(&eff_link_enc)
    .fetch_one(&s.pool).await
    .map_err(|e| {
        if e.to_string().contains("UNIQUE") {
            (StatusCode::CONFLICT, "该端口已被你的其它转发占用".into())
        } else {
            err(e)
        }
    })?;
    Ok(Json(Forward {
        id, name: f.name, listen_port: f.listen_port, protocol,
        hops, targets, target_strategy: f.target_strategy,
        enabled: true, created_at: now, owner_id: claims.sub,
        listener_status: Vec::new(),
        bytes_in: 0, bytes_out: 0,
        quota_in_bytes: eff_qin,
        quota_out_bytes: eff_qout,
        rate_in_bps: eff_rin,
        rate_out_bps: eff_rout,
        quota_reset: eff_qreset,
        quota_reset_at_ms: eff_qreset_at,
        quota_exhausted_at_ms: None,
        link_encryption: eff_link_enc,
    }))
}

/// 把 0 / 负数视为"未启用该字段"，None 也视为未启用。统一规约成 NULL。
fn nz_opt(v: Option<i64>) -> Option<i64> {
    v.filter(|&x| x > 0)
}

/// 规范化 link_encryption：'plain' 透传，其余（含 None / 空 / 未知）均收敛到 'tls'。
fn normalize_link_encryption(v: Option<&str>) -> String {
    match v.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
        Some("plain") => "plain".into(),
        _ => "tls".into(),
    }
}

/// 规范化 quota_reset 字符串为合法枚举或 None。
fn normalize_quota_reset(v: Option<&str>) -> Option<String> {
    match v.map(str::trim).filter(|s| !s.is_empty()) {
        Some("daily") => Some("daily".into()),
        Some("monthly") => Some("monthly".into()),
        Some("none") | None => None, // 'none' 视为无重置策略（quota_reset_at_ms = NULL）
        Some(other) => {
            tracing::warn!(value = %other, "未知 quota_reset，按 none 处理");
            None
        }
    }
}

/// 编辑转发：admin 可改任意，customer 只能改自己。
async fn update_forward(
    claims: Claims,
    State(s): State<AppState>,
    Path(id): Path<i64>,
    Json(f): Json<ForwardCreate>,
) -> Result<Json<Forward>, ApiErr> {
    let owner: Option<i64> = sqlx::query_scalar("SELECT owner_id FROM forwards WHERE id=?")
        .bind(id).fetch_optional(&s.pool).await.map_err(err)?;
    let owner = owner.ok_or((StatusCode::NOT_FOUND, "转发不存在".into()))?;
    if !claims.is_admin() && owner != claims.sub {
        return Err((StatusCode::FORBIDDEN, "无权编辑他人的转发".into()));
    }
    let protocol = parse_protocol(&f.protocol)
        .ok_or_else(|| bad("protocol 必须为 tcp / udp / tcp+udp 之一"))?;
    let mut hops = f.normalized_hops();
    if hops.is_empty() || hops.iter().any(|h| h.nodes.is_empty()) {
        return Err(bad("hops 不能为空，且每跳至少一个节点"));
    }
    if f.listen_port < 1 || f.listen_port > 65535 {
        return Err(bad("listen_port 必须在 1-65535"));
    }
    let mut targets = f.normalized_targets();
    if targets.is_empty() {
        return Err(bad("至少需要 1 个目标地址"));
    }
    for t in &mut targets {
        if t.addr.trim().is_empty() {
            return Err(bad("目标地址不能为空"));
        }
        t.weight = t.weight.clamp(1, 1000);
    }
    let mut seen = std::collections::HashSet::new();
    for h in &mut hops {
        for n in &mut h.nodes {
            n.weight = n.weight.clamp(1, 1000);
            if !seen.insert(n.id.clone()) {
                return Err(bad(&format!("节点 {} 在路径中重复（循环路径）", n.id)));
            }
        }
    }
    // 入口节点 + 端口 + 协议冲突校验（排除自己）
    let entry_ids: Vec<String> = hops
        .first()
        .map(|h| h.nodes.iter().map(|n| n.id.clone()).collect())
        .unwrap_or_default();
    check_entry_port_conflict(&s.pool, f.listen_port, &protocol, &entry_ids, Some(id)).await?;
    let hops_json = serde_json::to_string(&hops).map_err(err)?;
    let targets_json = serde_json::to_string(&targets).map_err(err)?;
    // #39 quota / rate / 重置策略：admin 可调，customer 不可（否则可绕过自己被管理员设置的限额）。
    // #27 link_encryption 同样 admin-only。
    // customer 改其它字段时，quota_* / link_encryption 全部用 DB 旧值覆盖入参。
    let (eff_qin, eff_qout, eff_rin, eff_rout, eff_qreset, eff_qreset_at, eff_link_enc) = if claims.is_admin() {
        let qr = normalize_quota_reset(f.quota_reset.as_deref());
        let qra = crate::compute_next_reset_at_ms(qr.as_deref(), now_ms());
        (
            nz_opt(f.quota_in_bytes), nz_opt(f.quota_out_bytes),
            nz_opt(f.rate_in_bps), nz_opt(f.rate_out_bps),
            qr, qra,
            normalize_link_encryption(f.link_encryption.as_deref()),
        )
    } else {
        let row: (Option<i64>, Option<i64>, Option<i64>, Option<i64>, Option<String>, Option<i64>, String) =
            sqlx::query_as(
                "SELECT quota_in_bytes, quota_out_bytes, rate_in_bps, rate_out_bps, \
                 quota_reset, quota_reset_at_ms, link_encryption FROM forwards WHERE id=?",
            )
            .bind(id).fetch_one(&s.pool).await.map_err(err)?;
        row
    };
    sqlx::query(
        "UPDATE forwards SET name=?, listen_port=?, protocol=?, path=?, target=?, target_strategy=?, \
         quota_in_bytes=?, quota_out_bytes=?, rate_in_bps=?, rate_out_bps=?, \
         quota_reset=?, quota_reset_at_ms=?, link_encryption=? \
         WHERE id=?",
    )
    .bind(&f.name).bind(f.listen_port).bind(&protocol).bind(&hops_json)
    .bind(&targets_json).bind(&f.target_strategy)
    .bind(eff_qin).bind(eff_qout)
    .bind(eff_rin).bind(eff_rout)
    .bind(&eff_qreset).bind(eff_qreset_at)
    .bind(&eff_link_enc)
    .bind(id)
    .execute(&s.pool).await
    .map_err(|e| {
        if e.to_string().contains("UNIQUE") {
            (StatusCode::CONFLICT, "该端口已被同 owner 的其它转发占用".into())
        } else {
            err(e)
        }
    })?;
    let row = sqlx::query_as::<_, ForwardRow>("SELECT * FROM forwards WHERE id=?")
        .bind(id).fetch_one(&s.pool).await.map_err(err)?;
    Ok(Json(row.into()))
}

async fn delete_forward(
    claims: Claims,
    State(s): State<AppState>,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiErr> {
    let owner: Option<i64> =
        sqlx::query_scalar("SELECT owner_id FROM forwards WHERE id=?")
            .bind(id).fetch_optional(&s.pool).await.map_err(err)?;
    let owner = owner.ok_or((StatusCode::NOT_FOUND, "转发不存在".into()))?;
    if !claims.is_admin() && owner != claims.sub {
        return Err((StatusCode::FORBIDDEN, "无权操作他人的转发".into()));
    }
    sqlx::query("DELETE FROM forwards WHERE id=?").bind(id).execute(&s.pool).await.map_err(err)?;
    Ok(StatusCode::NO_CONTENT)
}

// ---- 邀请码 / 用户：admin only ----

async fn list_invites(_: AdminClaims, State(s): State<AppState>) -> Result<Json<Vec<InviteCode>>, ApiErr> {
    Ok(Json(
        sqlx::query_as::<_, InviteCode>("SELECT * FROM invite_codes ORDER BY created_at DESC")
            .fetch_all(&s.pool).await.map_err(err)?
    ))
}

async fn create_invite(admin: AdminClaims, State(s): State<AppState>) -> Result<Json<InviteCode>, ApiErr> {
    let code = uuid::Uuid::new_v4().simple().to_string();
    let now = now_ms();
    sqlx::query("INSERT INTO invite_codes (code, created_by, created_at) VALUES (?,?,?)")
        .bind(&code).bind(admin.0.sub).bind(now)
        .execute(&s.pool).await.map_err(err)?;
    Ok(Json(InviteCode {
        code, created_by: admin.0.sub, used_by: None, used_at: None, created_at: now,
    }))
}

// ---- 节点注册令牌 ----

const ENROLL_TTL_MS: i64 = 24 * 3600 * 1000;

/// admin 为指定节点生成一次性注册令牌（24h 有效）。
async fn create_enrollment(
    _: AdminClaims,
    State(s): State<AppState>,
    Path(node_id): Path<String>,
) -> Result<Json<EnrollmentToken>, ApiErr> {
    // 节点必须先存在（admin 先在 UI 加节点 → 再生成令牌）
    let exists: i64 = sqlx::query_scalar("SELECT count(*) FROM nodes WHERE id=?")
        .bind(&node_id).fetch_one(&s.pool).await.map_err(err)?;
    if exists == 0 {
        return Err((StatusCode::NOT_FOUND, "节点不存在".into()));
    }
    // 旧令牌作废（每个节点同时只保留一个有效令牌）
    sqlx::query("DELETE FROM node_enrollment_tokens WHERE node_id=? AND used_at IS NULL")
        .bind(&node_id).execute(&s.pool).await.map_err(err)?;
    let token = uuid::Uuid::new_v4().simple().to_string();
    let now = now_ms();
    let expires = now + ENROLL_TTL_MS;
    sqlx::query(
        "INSERT INTO node_enrollment_tokens (token, node_id, expires_at, created_at) VALUES (?,?,?,?)",
    )
    .bind(&token).bind(&node_id).bind(expires).bind(now)
    .execute(&s.pool).await.map_err(err)?;
    Ok(Json(EnrollmentToken {
        token, node_id, expires_at: expires, used_at: None, created_at: now,
    }))
}

/// 节点用令牌兑换专属 mTLS 证书（一次性、公开端点、不需要 JWT）。
async fn enroll_node(
    State(s): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(r): Json<EnrollRequest>,
) -> Result<Json<EnrollResponse>, ApiErr> {
    // 安全：生产部署应强制 HTTPS（CA 私钥不应走明文链路）。
    // IRIS_REQUIRE_TLS=1 开启后，非 https 请求一律拒绝。
    if std::env::var("IRIS_REQUIRE_TLS").as_deref() == Ok("1") {
        let xfp = headers.get("x-forwarded-proto").and_then(|v| v.to_str().ok());
        if xfp != Some("https") {
            return Err((
                StatusCode::FORBIDDEN,
                "enrollment 必须经 HTTPS（X-Forwarded-Proto=https），拒绝明文传输证书私钥".into(),
            ));
        }
    }
    // 原子消费：UPDATE 并校验 rows_affected。失败 = 不存在 / 已用 / 已过期，统一返回 401，防 TOCTOU
    let now = now_ms();
    let res = sqlx::query(
        "UPDATE node_enrollment_tokens SET used_at=? \
         WHERE token=? AND used_at IS NULL AND expires_at >= ?",
    )
    .bind(now).bind(&r.token).bind(now)
    .execute(&s.pool).await.map_err(err)?;
    if res.rows_affected() != 1 {
        return Err((StatusCode::UNAUTHORIZED, "令牌无效、已使用或已过期".into()));
    }
    // 取节点 id（消费已确认令牌唯一）
    let node_id: String = sqlx::query_scalar("SELECT node_id FROM node_enrollment_tokens WHERE token=?")
        .bind(&r.token).fetch_one(&s.pool).await.map_err(err)?;
    // 签发节点专属证书
    let (cert_pem, key_pem, ca_pem) =
        iris_common::sign_node_cert(&s.cert_dir, &node_id).map_err(err)?;
    // 数据面端口提示
    let addr: Option<String> = sqlx::query_scalar("SELECT addr FROM nodes WHERE id=?")
        .bind(&node_id).fetch_optional(&s.pool).await.map_err(err)?;
    let data_addr_hint = addr
        .as_deref()
        .and_then(|a| a.rsplit(':').next())
        .map(|port| format!("0.0.0.0:{port}"))
        .unwrap_or_else(|| "0.0.0.0:7444".into());
    Ok(Json(EnrollResponse {
        node_id,
        ca_pem, cert_pem, key_pem,
        master_grpc: std::env::var("IRIS_PUBLIC_GRPC").unwrap_or_else(|_| "https://127.0.0.1:7443".into()),
        data_addr_hint,
    }))
}

/// 安装脚本（公开端点）。脚本本体托管在 GitHub raw（主仓库 main 分支或 IRIS_INSTALL_SCRIPT_URL
/// 覆盖），此端点仅做 302 redirect：脚本改动不再需要 rebuild master，且 master 离线也不影响新装节点。
/// 生产强制 HTTPS 同 enroll 端点。
async fn install_script(headers: axum::http::HeaderMap) -> axum::response::Response {
    use axum::response::{IntoResponse, Redirect};
    if std::env::var("IRIS_REQUIRE_TLS").as_deref() == Ok("1") {
        let xfp = headers.get("x-forwarded-proto").and_then(|v| v.to_str().ok());
        if xfp != Some("https") {
            return (StatusCode::FORBIDDEN, "请使用 HTTPS 访问 /install.sh").into_response();
        }
    }
    let url = std::env::var("IRIS_INSTALL_SCRIPT_URL").unwrap_or_else(|_| {
        "https://raw.githubusercontent.com/Everless321/Iris/main/install.sh".to_string()
    });
    Redirect::temporary(&url).into_response()
}

// ---- #36 sessions ----

#[derive(Debug, sqlx::FromRow)]
struct SessionRow {
    id: String,
    forward_id: i64,
    entry_node_id: String,
    client_ip: String,
    client_port: i64,
    target_addr: String,
    hops_path: String,
    protocol: String,
    opened_at_ms: i64,
    closed_at_ms: Option<i64>,
    bytes_in: i64,
    bytes_out: i64,
    close_reason: Option<String>,
}

#[derive(Debug, serde::Serialize)]
struct Session {
    id: String,
    forward_id: i64,
    entry_node_id: String,
    client_ip: String,
    client_port: i64,
    target_addr: String,
    hops_path: Vec<String>,
    protocol: String,
    opened_at_ms: i64,
    closed_at_ms: Option<i64>,
    bytes_in: i64,
    bytes_out: i64,
    close_reason: Option<String>,
}

impl From<SessionRow> for Session {
    fn from(r: SessionRow) -> Self {
        Session {
            id: r.id,
            forward_id: r.forward_id,
            entry_node_id: r.entry_node_id,
            client_ip: r.client_ip,
            client_port: r.client_port,
            target_addr: r.target_addr,
            hops_path: serde_json::from_str(&r.hops_path).unwrap_or_default(),
            protocol: r.protocol,
            opened_at_ms: r.opened_at_ms,
            closed_at_ms: r.closed_at_ms,
            bytes_in: r.bytes_in,
            bytes_out: r.bytes_out,
            close_reason: r.close_reason,
        }
    }
}

#[derive(Debug, serde::Deserialize)]
struct ListSessionsQuery {
    #[serde(default)]
    page: Option<i64>,
    #[serde(default)]
    page_size: Option<i64>,
    #[serde(default)]
    from: Option<i64>,
    #[serde(default)]
    to: Option<i64>,
    #[serde(default)]
    client_ip: Option<String>,
}

#[derive(Debug, serde::Serialize)]
struct SessionsResp {
    sessions: Vec<Session>,
    total: i64,
    page: i64,
    page_size: i64,
}

async fn list_forward_sessions(
    _: AdminClaims,
    State(s): State<AppState>,
    Path(forward_id): Path<i64>,
    axum::extract::Query(q): axum::extract::Query<ListSessionsQuery>,
) -> Result<Json<SessionsResp>, ApiErr> {
    let page = q.page.unwrap_or(1).max(1);
    let page_size = q.page_size.unwrap_or(50).clamp(1, 500);
    let offset = (page - 1) * page_size;

    // 动态拼 where + bind 参数（顺序必须一致）
    let mut where_clauses: Vec<&str> = vec!["forward_id = ?"];
    if q.from.is_some() { where_clauses.push("opened_at_ms >= ?"); }
    if q.to.is_some() { where_clauses.push("opened_at_ms <= ?"); }
    if q.client_ip.is_some() { where_clauses.push("client_ip = ?"); }
    let where_sql = where_clauses.join(" AND ");

    let count_sql = format!("SELECT COUNT(*) FROM forward_sessions WHERE {where_sql}");
    let mut count_q = sqlx::query_scalar::<_, i64>(&count_sql).bind(forward_id);
    if let Some(f) = q.from { count_q = count_q.bind(f); }
    if let Some(t) = q.to { count_q = count_q.bind(t); }
    if let Some(ref ip) = q.client_ip { count_q = count_q.bind(ip.clone()); }
    let total: i64 = count_q.fetch_one(&s.pool).await.map_err(err)?;

    let list_sql = format!(
        "SELECT * FROM forward_sessions WHERE {where_sql} ORDER BY opened_at_ms DESC LIMIT ? OFFSET ?"
    );
    let mut list_q = sqlx::query_as::<_, SessionRow>(&list_sql).bind(forward_id);
    if let Some(f) = q.from { list_q = list_q.bind(f); }
    if let Some(t) = q.to { list_q = list_q.bind(t); }
    if let Some(ref ip) = q.client_ip { list_q = list_q.bind(ip.clone()); }
    let rows = list_q.bind(page_size).bind(offset).fetch_all(&s.pool).await.map_err(err)?;
    let sessions: Vec<Session> = rows.into_iter().map(Session::from).collect();
    Ok(Json(SessionsResp { sessions, total, page, page_size }))
}

async fn list_active_sessions(
    _: AdminClaims,
    State(s): State<AppState>,
    Path(forward_id): Path<i64>,
) -> Result<Json<Vec<Session>>, ApiErr> {
    let rows: Vec<SessionRow> = sqlx::query_as(
        "SELECT * FROM forward_sessions WHERE forward_id = ? AND closed_at_ms IS NULL \
         ORDER BY opened_at_ms DESC LIMIT 500",
    )
    .bind(forward_id)
    .fetch_all(&s.pool)
    .await
    .map_err(err)?;
    Ok(Json(rows.into_iter().map(Session::from).collect()))
}

async fn get_session(
    _: AdminClaims,
    State(s): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Session>, ApiErr> {
    let row: Option<SessionRow> =
        sqlx::query_as("SELECT * FROM forward_sessions WHERE id = ?")
            .bind(&id)
            .fetch_optional(&s.pool)
            .await
            .map_err(err)?;
    row.map(Session::from)
        .map(Json)
        .ok_or_else(|| (StatusCode::NOT_FOUND, "session not found".into()))
}

#[derive(Deserialize)]
pub struct SseTicketQuery {
    ticket: String,
}

#[derive(Serialize)]
pub struct SseTicketResp {
    pub ticket: String,
    pub expires_in: u64,
}

/// 发放 SSE 单用 ticket，避免把 JWT 直接放进 EventSource URL（会落进 access log / 浏览器历史）。
/// 用 Authorization header 走标准 admin auth，返回 60s opaque ticket，UI 紧接着开 EventSource。
async fn issue_sse_ticket(
    _: AdminClaims,
    State(s): State<AppState>,
    Path(forward_id): Path<i64>,
) -> Result<Json<SseTicketResp>, ApiErr> {
    let ticket = Uuid::new_v4().to_string();
    let now = now_ms();
    let exp_ms = now + 60_000;
    {
        let mut g = s.sse_tickets.lock().unwrap();
        g.retain(|_, e| e.exp_ms > now); // 顺手清过期，O(n) 但 ticket 量极小
        g.insert(ticket.clone(), SseTicketEntry { forward_id, exp_ms });
    }
    Ok(Json(SseTicketResp { ticket, expires_in: 60 }))
}

/// SSE 实时推送 session 变化通知。每条事件是空 ping（"refresh"），UI 收到后重拉
/// /sessions/active + /sessions（debounce 300ms），靠后端 SQL 保证一致性。
/// 鉴权：ticket 从 query string 取，consumed 一次后从 map 移除（single-use）。
async fn sessions_stream(
    State(s): State<AppState>,
    Path(forward_id): Path<i64>,
    Query(q): Query<SseTicketQuery>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiErr> {
    let now = now_ms();
    let entry = {
        let mut g = s.sse_tickets.lock().unwrap();
        g.remove(&q.ticket) // single-use
    };
    let entry = entry.ok_or((StatusCode::UNAUTHORIZED, "invalid or used ticket".into()))?;
    if entry.exp_ms < now {
        return Err((StatusCode::UNAUTHORIZED, "ticket expired".into()));
    }
    if entry.forward_id != forward_id {
        return Err((StatusCode::FORBIDDEN, "ticket forward mismatch".into()));
    }

    let rx = s.sessions_tx.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(move |msg| match msg {
        Ok(fid) if fid == forward_id => {
            Some(Ok(Event::default().event("refresh").data("ping")))
        }
        Ok(_) => None,        // 其它 forward_id 不发
        Err(_) => None,       // Lagged 丢弃 — 下一个 send 还能触发 refresh
    });
    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15))))
}

async fn list_users(_: AdminClaims, State(s): State<AppState>) -> Result<Json<Vec<UserDto>>, ApiErr> {
    let rows = sqlx::query_as::<_, UserRow>("SELECT * FROM users ORDER BY created_at")
        .fetch_all(&s.pool).await.map_err(err)?;
    Ok(Json(rows.into_iter().map(UserDto::from).collect()))
}

// ---- SLA / metrics ----

fn uptime(n: &Node) -> f64 {
    if n.probe_total > 0 { n.probe_ok as f64 / n.probe_total as f64 } else { 0.0 }
}

async fn sla(State(s): State<AppState>) -> Result<Json<Value>, ApiErr> {
    let nodes = sqlx::query_as::<_, Node>("SELECT * FROM nodes ORDER BY id")
        .fetch_all(&s.pool).await.map_err(err)?;
    let online = nodes.iter().filter(|n| n.health == "healthy").count();
    let items: Vec<Value> = nodes.iter().map(|n| json!({
        "id": n.id, "name": n.name, "health": n.health, "latency_ms": n.latency_ms,
        "uptime": uptime(n), "fail_events": n.fail_events, "downtime_ms": n.downtime_ms,
    })).collect();
    Ok(Json(json!({ "online": online, "total": nodes.len(), "nodes": items })))
}

/// 近 1 小时的探测样本，按 node_id 分组。
async fn sla_samples(
    _: AdminClaims,
    State(s): State<AppState>,
) -> Result<Json<Value>, ApiErr> {
    let cutoff = now_ms() - 3600 * 1000;
    let rows: Vec<(String, i64, i64, Option<i64>)> = sqlx::query_as(
        "SELECT node_id, ts, ok, latency_ms FROM probe_samples WHERE ts >= ? ORDER BY ts ASC",
    )
    .bind(cutoff)
    .fetch_all(&s.pool)
    .await
    .map_err(err)?;
    let mut grouped: std::collections::BTreeMap<String, Vec<Value>> = Default::default();
    for (node_id, ts, ok, latency) in rows {
        grouped
            .entry(node_id)
            .or_default()
            .push(json!({ "ts": ts, "ok": ok, "latency_ms": latency }));
    }
    Ok(Json(json!(grouped)))
}

fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n")
}

async fn metrics(State(s): State<AppState>) -> Result<String, ApiErr> {
    let nodes = sqlx::query_as::<_, Node>("SELECT * FROM nodes ORDER BY id")
        .fetch_all(&s.pool).await.map_err(err)?;
    let mut o = String::new();
    o.push_str("# HELP iris_node_up 节点是否健康(1/0)\n# TYPE iris_node_up gauge\n");
    for n in &nodes {
        o.push_str(&format!("iris_node_up{{node=\"{}\"}} {}\n",
            esc(&n.id), if n.health == "healthy" { 1 } else { 0 }));
    }
    o.push_str("# HELP iris_node_latency_ms 最近探测RTT\n# TYPE iris_node_latency_ms gauge\n");
    for n in &nodes {
        o.push_str(&format!("iris_node_latency_ms{{node=\"{}\"}} {}\n",
            esc(&n.id), n.latency_ms.unwrap_or(0)));
    }
    o.push_str("# HELP iris_node_uptime_ratio 探测可用率\n# TYPE iris_node_uptime_ratio gauge\n");
    for n in &nodes {
        o.push_str(&format!("iris_node_uptime_ratio{{node=\"{}\"}} {:.4}\n", esc(&n.id), uptime(n)));
    }
    o.push_str("# HELP iris_node_fail_events 故障事件次数\n# TYPE iris_node_fail_events counter\n");
    for n in &nodes {
        o.push_str(&format!("iris_node_fail_events{{node=\"{}\"}} {}\n", esc(&n.id), n.fail_events));
    }
    o.push_str("# HELP iris_node_downtime_ms 累计不可用时长\n# TYPE iris_node_downtime_ms counter\n");
    for n in &nodes {
        o.push_str(&format!("iris_node_downtime_ms{{node=\"{}\"}} {}\n", esc(&n.id), n.downtime_ms));
    }
    let online = nodes.iter().filter(|n| n.health == "healthy").count();
    o.push_str(&format!("# HELP iris_nodes_online 在线节点数\n# TYPE iris_nodes_online gauge\niris_nodes_online {online}\n"));
    o.push_str(&format!("# HELP iris_nodes_total 节点总数\n# TYPE iris_nodes_total gauge\niris_nodes_total {}\n", nodes.len()));
    // forward 累计流量（counter，重启不归零 = 持久化在 DB）
    let fwds = sqlx::query_as::<_, ForwardRow>("SELECT * FROM forwards")
        .fetch_all(&s.pool).await.map_err(err)?;
    o.push_str("# HELP iris_forward_bytes_in_total 转发上行累计字节（客户端→入口）\n# TYPE iris_forward_bytes_in_total counter\n");
    for f in &fwds {
        o.push_str(&format!("iris_forward_bytes_in_total{{id=\"{}\",name=\"{}\",port=\"{}\"}} {}\n",
            f.id, esc(&f.name), f.listen_port, f.bytes_in));
    }
    o.push_str("# HELP iris_forward_bytes_out_total 转发下行累计字节（入口→客户端）\n# TYPE iris_forward_bytes_out_total counter\n");
    for f in &fwds {
        o.push_str(&format!("iris_forward_bytes_out_total{{id=\"{}\",name=\"{}\",port=\"{}\"}} {}\n",
            f.id, esc(&f.name), f.listen_port, f.bytes_out));
    }
    Ok(o)
}

// ─── 链路测试：让每条边的 from_node 通过 gRPC ProbeReach 探测 to_addr ───
#[derive(Debug, Deserialize)]
pub struct TestRequest {
    pub hops: Vec<Hop>,
    // 新格式
    #[serde(default)]
    pub targets: Option<Vec<TargetEndpoint>>,
    // 旧格式：单字符串，自动包成单元素数组
    #[serde(default)]
    pub target: Option<String>,
}
#[derive(Debug, Clone, Serialize)]
pub struct EdgeProbe {
    pub from_node: String,
    pub to_node: Option<String>, // None 表示 to=target
    pub to_addr: String,
    pub ok: bool,
    pub latency_ms: u64,
    pub error: String,
}
#[derive(Debug, Serialize)]
pub struct TestResponse {
    pub results: Vec<EdgeProbe>,
}

/// 安全校验：拒绝 target 解析到回环/内网/链路本地/多播/广播/未指定 等地址，
/// 防止认证用户用本端点把节点群当端口扫描器去扫主控/节点本机或它们所处的内网。
/// 设置 `IRIS_ALLOW_PRIVATE_TARGETS=1` 可在开发环境放行。
///
/// 返回 pinned 的 `ip:port` 字符串：调用方应把这个值发给 node，避免 node 端二次
/// DNS 解析时被 DNS rebinding 切到内网地址（TOCTOU）。
async fn check_external_target(addr: &str) -> Result<String, ApiErr> {
    let resolved: Vec<SocketAddr> = match tokio::net::lookup_host(addr).await {
        Ok(it) => it.collect(),
        Err(e) => return Err(bad(&format!("无法解析目标地址: {e}"))),
    };
    if resolved.is_empty() {
        return Err(bad("无法解析目标地址"));
    }
    let allow_private = std::env::var("IRIS_ALLOW_PRIVATE_TARGETS").as_deref() == Ok("1");
    if !allow_private {
        for sa in &resolved {
            if is_disallowed_ip(&sa.ip()) {
                return Err(bad(&format!(
                    "禁止指向回环/内网/链路本地地址: {} (解析自 {addr})",
                    sa.ip()
                )));
            }
        }
    }
    // 返回第一条解析结果的 ip:port 形式（v6 自动 [::1]:port）。
    // node 端 TcpStream::connect 看到字面 IP 就不会再走 DNS，杜绝 rebinding。
    Ok(resolved[0].to_string())
}

fn is_disallowed_ip(ip: &std::net::IpAddr) -> bool {
    use std::net::IpAddr;
    // IPv4-mapped IPv6（::ffff:127.0.0.1 之类）折回 v4 复用规则，防绕过
    let ip = match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(IpAddr::V6(*v6)),
        v => *v,
    };
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_multicast()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || o[0] == 0              // 0.0.0.0/8 "this network"
                || (o[0] & 0xf0) == 0xe0  // 224.0.0.0/4 multicast (冗余兜底)
        }
        IpAddr::V6(v6) => {
            let s = v6.segments();
            v6.is_loopback()
                || v6.is_multicast()
                || v6.is_unspecified()
                || (s[0] & 0xfe00) == 0xfc00 // unique local fc00::/7
                || (s[0] & 0xffc0) == 0xfe80 // link-local fe80::/10
        }
    }
}

async fn test_forward(
    _claims: Claims,
    State(s): State<AppState>,
    Json(req): Json<TestRequest>,
) -> Result<Json<TestResponse>, ApiErr> {
    // 校验
    if req.hops.is_empty() || req.hops.iter().any(|h| h.nodes.is_empty()) {
        return Err(bad("hops 不能为空，且每跳至少一个节点"));
    }
    // 归一化目标：新字段优先，旧 target 单串包成单元素
    let targets: Vec<TargetEndpoint> = if let Some(ts) = req.targets {
        ts
    } else if let Some(t) = req.target.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        vec![TargetEndpoint { addr: t.into(), weight: 1 }]
    } else {
        Vec::new()
    };
    if targets.is_empty() {
        return Err(bad("targets 不能为空"));
    }
    // SSRF 防护：解析后用 pinned ip:port 替换原始 addr，避免 node 二次 DNS 被 rebinding
    let mut targets = targets;
    for t in &mut targets {
        if t.addr.trim().is_empty() {
            return Err(bad("目标地址不能为空"));
        }
        t.addr = check_external_target(&t.addr).await?;
    }

    // 加载所有相关节点的 addr 一次
    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT id, addr FROM nodes")
            .fetch_all(&s.pool).await.map_err(err)?;
    let addr_map: HashMap<String, String> = rows.into_iter().collect();

    // 收集要探测的边：(from_node_id, Option<to_node_id>, to_addr)
    let mut tasks: Vec<(String, Option<String>, String)> = Vec::new();
    for i in 0..req.hops.len().saturating_sub(1) {
        for from in &req.hops[i].nodes {
            for to in &req.hops[i + 1].nodes {
                let Some(addr) = addr_map.get(&to.id).cloned() else { continue };
                tasks.push((from.id.clone(), Some(to.id.clone()), addr));
            }
        }
    }
    // 最后一跳 → 每个 target（笛卡尔积）
    if let Some(last) = req.hops.last() {
        for from in &last.nodes {
            for tgt in &targets {
                tasks.push((from.id.clone(), None, tgt.addr.clone()));
            }
        }
    }

    // 并发执行（按 from_node 分桶共享 channel，单 channel 串行调用避免重复建链）
    let mut by_from: HashMap<String, Vec<(Option<String>, String, usize)>> = HashMap::new();
    for (i, (from, to, addr)) in tasks.iter().enumerate() {
        by_from.entry(from.clone()).or_default().push((to.clone(), addr.clone(), i));
    }

    let mut results: Vec<EdgeProbe> = vec![
        EdgeProbe {
            from_node: String::new(), to_node: None, to_addr: String::new(),
            ok: false, latency_ms: 0, error: String::new(),
        }; tasks.len()
    ];
    let tls = s.node_caller_tls.clone();

    // 整体超时 8s
    let overall = tokio::time::timeout(Duration::from_secs(8), async {
        let mut set = tokio::task::JoinSet::new();
        for (from, items) in by_from.into_iter() {
            let Some(from_addr) = addr_map.get(&from).cloned() else {
                for (to, addr, idx) in items {
                    results[idx] = EdgeProbe {
                        from_node: from.clone(), to_node: to, to_addr: addr,
                        ok: false, latency_ms: 0,
                        error: "节点未注册或无地址".into(),
                    };
                }
                continue;
            };
            let tls = tls.clone();
            let from_id = from.clone();
            set.spawn(async move {
                // SNI 用目标 node_id：rustls 校验对方 cert SAN 含此 node_id，绑定 cert 到具体节点。
                let tls = tls.domain_name(from_id.clone());
                // 建一次 channel，串行 probe 所有目标
                let ep_res = Endpoint::from_shared(format!("https://{from_addr}"))
                    .and_then(|e| e.tls_config(tls));
                let channel_res = match ep_res {
                    Ok(e) => e.connect_timeout(Duration::from_millis(1500)).connect().await,
                    Err(e) => Err(e),
                };
                let mut probes: Vec<(Option<String>, String, usize, EdgeProbe)> =
                    Vec::with_capacity(items.len());
                match channel_res {
                    Err(e) => {
                        let detail = format!("{e:?}");
                        tracing::warn!(node = %from_id, addr = %from_addr, error = %detail, "probe: 连不上节点 gRPC");
                        let emsg = format!("连不上节点: {e}");
                        for (to, addr, idx) in items {
                            probes.push((to.clone(), addr.clone(), idx, EdgeProbe {
                                from_node: from_id.clone(), to_node: to, to_addr: addr,
                                ok: false, latency_ms: 0, error: emsg.clone(),
                            }));
                        }
                    }
                    Ok(channel) => {
                        let mut client = DataPlaneClient::new(channel);
                        for (to, addr, idx) in items {
                            let req = ProbeReachRequest { addr: addr.clone(), timeout_ms: 2000 };
                            let edge = match client.probe_reach(req).await {
                                Ok(resp) => {
                                    let p = resp.into_inner();
                                    EdgeProbe {
                                        from_node: from_id.clone(), to_node: to.clone(),
                                        to_addr: addr.clone(),
                                        ok: p.ok, latency_ms: p.latency_ms, error: p.error,
                                    }
                                }
                                Err(st) => EdgeProbe {
                                    from_node: from_id.clone(), to_node: to.clone(),
                                    to_addr: addr.clone(),
                                    ok: false, latency_ms: 0,
                                    error: format!("rpc: {st}"),
                                },
                            };
                            probes.push((to, addr, idx, edge));
                        }
                    }
                }
                probes
            });
        }
        while let Some(joined) = set.join_next().await {
            let Ok(items) = joined else { continue };
            for (_to, _addr, idx, edge) in items {
                results[idx] = edge;
            }
        }
        results
    }).await;

    match overall {
        Ok(r) => Ok(Json(TestResponse { results: r })),
        Err(_) => Err((StatusCode::REQUEST_TIMEOUT, "测试超时（8s）".into())),
    }
}
