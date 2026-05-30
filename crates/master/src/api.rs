use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::get,
    Json, Router,
};
use sqlx::SqlitePool;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::models::{Forward, ForwardCreate, ForwardRow, Node, NodeCreate};
use serde_json::{json, Value};

pub fn router(pool: SqlitePool) -> Router {
    Router::new()
        .route("/api/nodes", get(list_nodes).post(create_node))
        .route("/api/nodes/:id", axum::routing::delete(delete_node))
        .route("/api/forwards", get(list_forwards).post(create_forward))
        .route("/api/forwards/:id", axum::routing::delete(delete_forward))
        .route("/api/sla", get(sla))
        .route("/metrics", get(metrics))
        .route("/healthz", get(|| async { "ok" }))
        .with_state(pool)
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as i64
}

type ApiErr = (StatusCode, String);
fn err<E: std::fmt::Display>(e: E) -> ApiErr {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

async fn list_nodes(State(pool): State<SqlitePool>) -> Result<Json<Vec<Node>>, ApiErr> {
    let rows = sqlx::query_as::<_, Node>("SELECT * FROM nodes ORDER BY created_at")
        .fetch_all(&pool)
        .await
        .map_err(err)?;
    Ok(Json(rows))
}

async fn create_node(
    State(pool): State<SqlitePool>,
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
    .execute(&pool)
    .await
    .map_err(err)?;
    Ok(Json(Node {
        id: n.id,
        name: n.name,
        addr: n.addr,
        status: "offline".into(),
        last_seen: None,
        created_at: now,
        weight: n.weight,
        health: "unknown".into(),
        latency_ms: None,
        fail_count: 0,
        probe_total: 0,
        probe_ok: 0,
        fail_events: 0,
        down_since: None,
        downtime_ms: 0,
    }))
}

fn uptime(n: &Node) -> f64 {
    if n.probe_total > 0 {
        n.probe_ok as f64 / n.probe_total as f64
    } else {
        0.0
    }
}

/// SLA 报告（JSON，给客户/管理端展示）。
async fn sla(State(pool): State<SqlitePool>) -> Result<Json<Value>, ApiErr> {
    let nodes = sqlx::query_as::<_, Node>("SELECT * FROM nodes ORDER BY id")
        .fetch_all(&pool)
        .await
        .map_err(err)?;
    let online = nodes.iter().filter(|n| n.health == "healthy").count();
    let items: Vec<Value> = nodes
        .iter()
        .map(|n| {
            json!({
                "id": n.id,
                "name": n.name,
                "health": n.health,
                "latency_ms": n.latency_ms,
                "uptime": uptime(n),
                "fail_events": n.fail_events,
                "downtime_ms": n.downtime_ms,
            })
        })
        .collect();
    Ok(Json(json!({
        "online": online,
        "total": nodes.len(),
        "nodes": items,
    })))
}

/// Prometheus label 值转义（反斜杠、双引号、换行）。
fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n")
}

/// Prometheus 指标导出。
async fn metrics(State(pool): State<SqlitePool>) -> Result<String, ApiErr> {
    let nodes = sqlx::query_as::<_, Node>("SELECT * FROM nodes ORDER BY id")
        .fetch_all(&pool)
        .await
        .map_err(err)?;
    let mut o = String::new();
    o.push_str("# HELP zhuanfa_node_up 节点是否健康(1/0)\n# TYPE zhuanfa_node_up gauge\n");
    for n in &nodes {
        o.push_str(&format!(
            "zhuanfa_node_up{{node=\"{}\"}} {}\n",
            esc(&n.id),
            if n.health == "healthy" { 1 } else { 0 }
        ));
    }
    o.push_str("# HELP zhuanfa_node_latency_ms 最近探测RTT\n# TYPE zhuanfa_node_latency_ms gauge\n");
    for n in &nodes {
        o.push_str(&format!(
            "zhuanfa_node_latency_ms{{node=\"{}\"}} {}\n",
            esc(&n.id),
            n.latency_ms.unwrap_or(0)
        ));
    }
    o.push_str("# HELP zhuanfa_node_uptime_ratio 探测可用率\n# TYPE zhuanfa_node_uptime_ratio gauge\n");
    for n in &nodes {
        o.push_str(&format!(
            "zhuanfa_node_uptime_ratio{{node=\"{}\"}} {:.4}\n",
            esc(&n.id),
            uptime(n)
        ));
    }
    o.push_str("# HELP zhuanfa_node_fail_events 故障事件次数\n# TYPE zhuanfa_node_fail_events counter\n");
    for n in &nodes {
        o.push_str(&format!(
            "zhuanfa_node_fail_events{{node=\"{}\"}} {}\n",
            esc(&n.id), n.fail_events
        ));
    }
    o.push_str("# HELP zhuanfa_node_downtime_ms 累计不可用时长\n# TYPE zhuanfa_node_downtime_ms counter\n");
    for n in &nodes {
        o.push_str(&format!(
            "zhuanfa_node_downtime_ms{{node=\"{}\"}} {}\n",
            esc(&n.id), n.downtime_ms
        ));
    }
    let online = nodes.iter().filter(|n| n.health == "healthy").count();
    o.push_str("# HELP zhuanfa_nodes_online 在线节点数\n# TYPE zhuanfa_nodes_online gauge\n");
    o.push_str(&format!("zhuanfa_nodes_online {online}\n"));
    o.push_str("# HELP zhuanfa_nodes_total 节点总数\n# TYPE zhuanfa_nodes_total gauge\n");
    o.push_str(&format!("zhuanfa_nodes_total {}\n", nodes.len()));
    Ok(o)
}

async fn delete_node(
    State(pool): State<SqlitePool>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiErr> {
    sqlx::query("DELETE FROM nodes WHERE id=?")
        .bind(id)
        .execute(&pool)
        .await
        .map_err(err)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_forwards(State(pool): State<SqlitePool>) -> Result<Json<Vec<Forward>>, ApiErr> {
    let rows = sqlx::query_as::<_, ForwardRow>("SELECT * FROM forwards ORDER BY id")
        .fetch_all(&pool)
        .await
        .map_err(err)?;
    Ok(Json(rows.into_iter().map(Forward::from).collect()))
}

async fn create_forward(
    State(pool): State<SqlitePool>,
    Json(f): Json<ForwardCreate>,
) -> Result<Json<Forward>, ApiErr> {
    let mut hops = f.normalized_hops();
    if hops.is_empty() || hops.iter().any(|h| h.nodes.is_empty()) {
        return Err((StatusCode::BAD_REQUEST, "hops 不能为空，且每跳至少一个节点".into()));
    }
    if f.listen_port < 1 || f.listen_port > 65535 {
        return Err((StatusCode::BAD_REQUEST, "listen_port 必须在 1-65535".into()));
    }
    // 权重上界保护（防 expand 内存爆炸），并做循环路径检测
    let mut seen = std::collections::HashSet::new();
    for h in &mut hops {
        for n in &mut h.nodes {
            n.weight = n.weight.clamp(1, 1000);
            if !seen.insert(n.id.clone()) {
                return Err((
                    StatusCode::BAD_REQUEST,
                    format!("节点 {} 在路径中重复出现（循环路径）", n.id),
                ));
            }
        }
    }
    let hops_json = serde_json::to_string(&hops).map_err(err)?;
    let now = now_ms();
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO forwards (name,listen_port,protocol,path,target,enabled,created_at) \
         VALUES (?,?,?,?,?,1,?) RETURNING id",
    )
    .bind(&f.name)
    .bind(f.listen_port)
    .bind(&f.protocol)
    .bind(&hops_json)
    .bind(&f.target)
    .bind(now)
    .fetch_one(&pool)
    .await
    .map_err(err)?;
    Ok(Json(Forward {
        id,
        name: f.name,
        listen_port: f.listen_port,
        protocol: f.protocol,
        hops,
        target: f.target,
        enabled: true,
        created_at: now,
    }))
}

async fn delete_forward(
    State(pool): State<SqlitePool>,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiErr> {
    sqlx::query("DELETE FROM forwards WHERE id=?")
        .bind(id)
        .execute(&pool)
        .await
        .map_err(err)?;
    Ok(StatusCode::NO_CONTENT)
}
