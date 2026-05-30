use axum::{
    extract::{FromRef, Path, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};
use sqlx::SqlitePool;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::auth::{hash_password, issue_token, verify_password, AdminClaims, AuthState, Claims};
use crate::models::{
    AuthResponse, Forward, ForwardCreate, ForwardRow, InviteCode, LoginRequest, Node, NodeCreate,
    RegisterRequest, UserDto, UserRow,
};

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub auth: AuthState,
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
        // 转发：customer 仅看/改自己；admin 全权
        .route("/api/forwards", get(list_forwards).post(create_forward))
        .route("/api/forwards/:id", axum::routing::delete(delete_forward))
        // 邀请码 & 用户管理：admin only
        .route("/api/invites", get(list_invites).post(create_invite))
        .route("/api/users", get(list_users))
        // SLA / 监控
        .route("/api/sla", get(sla))
        .route("/metrics", get(metrics))
        .route("/healthz", get(|| async { "ok" }))
        .with_state(state)
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as i64
}

type ApiErr = (StatusCode, String);
fn err<E: std::fmt::Display>(e: E) -> ApiErr {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}
fn bad(msg: &str) -> ApiErr {
    (StatusCode::BAD_REQUEST, msg.into())
}

// ---- 鉴权端点 ----

async fn register(
    State(s): State<AppState>,
    Json(r): Json<RegisterRequest>,
) -> Result<Json<AuthResponse>, ApiErr> {
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
    Json(r): Json<LoginRequest>,
) -> Result<Json<AuthResponse>, ApiErr> {
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
    .fetch_one(&s.pool).await.map_err(err)?;
    Ok(Json(Forward {
        id, name: f.name, listen_port: f.listen_port, protocol: f.protocol,
        hops, target: f.target, enabled: true, created_at: now,
    }))
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
