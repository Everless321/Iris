use axum::{
    extract::{ConnectInfo, FromRef, Path, State},
    http::StatusCode,
    routing::{get, post, put},
    Json, Router,
};
use serde_json::{json, Value};
use sqlx::SqlitePool;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::auth::{hash_password, issue_token, verify_password, AdminClaims, AuthState, Claims};
use crate::models::{
    AuthResponse, EnrollRequest, EnrollResponse, EnrollmentToken, Forward, ForwardCreate,
    ForwardRow, InviteCode, LoginRequest, Node, NodeCreate, RegisterRequest, UserDto, UserRow,
};
use crate::ratelimit::RateLimiter;

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub auth: AuthState,
    pub login_rl: Arc<RateLimiter>,
    pub register_rl: Arc<RateLimiter>,
    pub cert_dir: String, // master 证书目录，供节点 enrollment 时签发新证书
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
        .route(
            "/api/forwards/:id",
            put(update_forward).delete(delete_forward),
        )
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
    Ok(Json(rows.into_iter().map(Forward::from).collect()))
}

async fn create_forward(
    claims: Claims,
    State(s): State<AppState>,
    Json(f): Json<ForwardCreate>,
) -> Result<Json<Forward>, ApiErr> {
    let mut hops = f.normalized_hops();
    if hops.is_empty() || hops.iter().any(|h| h.nodes.is_empty()) {
        return Err(bad("hops 不能为空，且每跳至少一个节点"));
    }
    if f.listen_port < 1 || f.listen_port > 65535 {
        return Err(bad("listen_port 必须在 1-65535"));
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
    let hops_json = serde_json::to_string(&hops).map_err(err)?;
    let now = now_ms();
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO forwards (name,listen_port,protocol,path,target,enabled,created_at,owner_id) \
         VALUES (?,?,?,?,?,1,?,?) RETURNING id",
    )
    .bind(&f.name).bind(f.listen_port).bind(&f.protocol).bind(&hops_json)
    .bind(&f.target).bind(now).bind(claims.sub)
    .fetch_one(&s.pool).await
    .map_err(|e| {
        if e.to_string().contains("UNIQUE") {
            (StatusCode::CONFLICT, "该端口已被你的其它转发占用".into())
        } else {
            err(e)
        }
    })?;
    Ok(Json(Forward {
        id, name: f.name, listen_port: f.listen_port, protocol: f.protocol,
        hops, target: f.target, enabled: true, created_at: now,
        owner_id: claims.sub,
    }))
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
    let mut hops = f.normalized_hops();
    if hops.is_empty() || hops.iter().any(|h| h.nodes.is_empty()) {
        return Err(bad("hops 不能为空，且每跳至少一个节点"));
    }
    if f.listen_port < 1 || f.listen_port > 65535 {
        return Err(bad("listen_port 必须在 1-65535"));
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
    let hops_json = serde_json::to_string(&hops).map_err(err)?;
    sqlx::query(
        "UPDATE forwards SET name=?, listen_port=?, protocol=?, path=?, target=? WHERE id=?",
    )
    .bind(&f.name).bind(f.listen_port).bind(&f.protocol).bind(&hops_json)
    .bind(&f.target).bind(id)
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
    Json(r): Json<EnrollRequest>,
) -> Result<Json<EnrollResponse>, ApiErr> {
    let row = sqlx::query_as::<_, EnrollmentToken>(
        "SELECT * FROM node_enrollment_tokens WHERE token=?",
    )
    .bind(&r.token).fetch_optional(&s.pool).await.map_err(err)?
    .ok_or((StatusCode::UNAUTHORIZED, "令牌无效".into()))?;
    let now = now_ms();
    if row.used_at.is_some() {
        return Err((StatusCode::UNAUTHORIZED, "令牌已使用".into()));
    }
    if row.expires_at < now {
        return Err((StatusCode::UNAUTHORIZED, "令牌已过期".into()));
    }
    // 签发节点专属证书（CN=node_id），由 master CA 签发
    let (cert_pem, key_pem, ca_pem) =
        zhuanfa_common::sign_node_cert(&s.cert_dir, &row.node_id).map_err(err)?;
    sqlx::query("UPDATE node_enrollment_tokens SET used_at=? WHERE token=?")
        .bind(now).bind(&r.token).execute(&s.pool).await.map_err(err)?;
    // 拿出节点的注册地址作为 ZF_DATA_ADDR 提示（去掉 host，只保留端口）
    let addr: Option<String> = sqlx::query_scalar("SELECT addr FROM nodes WHERE id=?")
        .bind(&row.node_id).fetch_optional(&s.pool).await.map_err(err)?;
    let data_addr_hint = addr
        .as_deref()
        .and_then(|a| a.rsplit(':').next())
        .map(|port| format!("0.0.0.0:{port}"))
        .unwrap_or_else(|| "0.0.0.0:7444".into());
    Ok(Json(EnrollResponse {
        node_id: row.node_id,
        ca_pem, cert_pem, key_pem,
        master_grpc: std::env::var("ZF_PUBLIC_GRPC").unwrap_or_else(|_| "https://127.0.0.1:7443".into()),
        data_addr_hint,
    }))
}

/// 安装脚本（公开端点，从 master 镜像直接获取）。
async fn install_script() -> impl axum::response::IntoResponse {
    let body = include_str!("../assets/install-node.sh");
    (
        [(axum::http::header::CONTENT_TYPE, "text/x-shellscript; charset=utf-8")],
        body,
    )
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
    o.push_str("# HELP zhuanfa_node_up 节点是否健康(1/0)\n# TYPE zhuanfa_node_up gauge\n");
    for n in &nodes {
        o.push_str(&format!("zhuanfa_node_up{{node=\"{}\"}} {}\n",
            esc(&n.id), if n.health == "healthy" { 1 } else { 0 }));
    }
    o.push_str("# HELP zhuanfa_node_latency_ms 最近探测RTT\n# TYPE zhuanfa_node_latency_ms gauge\n");
    for n in &nodes {
        o.push_str(&format!("zhuanfa_node_latency_ms{{node=\"{}\"}} {}\n",
            esc(&n.id), n.latency_ms.unwrap_or(0)));
    }
    o.push_str("# HELP zhuanfa_node_uptime_ratio 探测可用率\n# TYPE zhuanfa_node_uptime_ratio gauge\n");
    for n in &nodes {
        o.push_str(&format!("zhuanfa_node_uptime_ratio{{node=\"{}\"}} {:.4}\n", esc(&n.id), uptime(n)));
    }
    o.push_str("# HELP zhuanfa_node_fail_events 故障事件次数\n# TYPE zhuanfa_node_fail_events counter\n");
    for n in &nodes {
        o.push_str(&format!("zhuanfa_node_fail_events{{node=\"{}\"}} {}\n", esc(&n.id), n.fail_events));
    }
    o.push_str("# HELP zhuanfa_node_downtime_ms 累计不可用时长\n# TYPE zhuanfa_node_downtime_ms counter\n");
    for n in &nodes {
        o.push_str(&format!("zhuanfa_node_downtime_ms{{node=\"{}\"}} {}\n", esc(&n.id), n.downtime_ms));
    }
    let online = nodes.iter().filter(|n| n.health == "healthy").count();
    o.push_str(&format!("# HELP zhuanfa_nodes_online 在线节点数\n# TYPE zhuanfa_nodes_online gauge\nzhuanfa_nodes_online {online}\n"));
    o.push_str(&format!("# HELP zhuanfa_nodes_total 节点总数\n# TYPE zhuanfa_nodes_total gauge\nzhuanfa_nodes_total {}\n", nodes.len()));
    Ok(o)
}
