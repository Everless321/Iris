use anyhow::Result;
use sqlx::SqlitePool;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::net::TcpStream;

const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as i64
}

/// 后台探测调度器：周期对每个节点 TCP 探测，更新健康/延迟/SLA 统计。
pub fn spawn(pool: SqlitePool, interval_secs: u64, fail_threshold: i64) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(interval_secs.max(1)));
        tracing::info!(interval_secs, fail_threshold, "probe scheduler started");
        loop {
            tick.tick().await;
            if let Err(e) = probe_round(&pool, fail_threshold).await {
                tracing::error!(error = %e, "probe round failed");
            }
        }
    });
}

async fn probe_round(pool: &SqlitePool, fail_threshold: i64) -> Result<()> {
    let nodes: Vec<(String, String)> =
        sqlx::query_as("SELECT id, addr FROM nodes").fetch_all(pool).await?;
    let mut tasks = Vec::new();
    for (id, addr) in nodes {
        let pool = pool.clone();
        tasks.push(tokio::spawn(async move {
            let start = Instant::now();
            let ok = matches!(
                tokio::time::timeout(PROBE_TIMEOUT, TcpStream::connect(&addr)).await,
                Ok(Ok(_))
            );
            let rtt = start.elapsed().as_millis() as i64;
            if let Err(e) = update_probe(&pool, &id, ok, rtt, fail_threshold).await {
                tracing::warn!(node = %id, error = %e, "update probe failed");
            }
        }));
    }
    for t in tasks {
        let _ = t.await;
    }
    Ok(())
}

async fn update_probe(
    pool: &SqlitePool,
    id: &str,
    ok: bool,
    rtt: i64,
    fail_threshold: i64,
) -> Result<()> {
    let now = now_ms();
    // 写入探测样本（每节点保留最近 240 条 = 近 1 小时 @ 15s 间隔）
    sqlx::query("INSERT INTO probe_samples (node_id, ts, ok, latency_ms) VALUES (?,?,?,?)")
        .bind(id)
        .bind(now)
        .bind(if ok { 1 } else { 0 })
        .bind(if ok { Some(rtt) } else { None })
        .execute(pool)
        .await?;
    // 滚动保留每节点最近 240 条：用游标拿到第 240 条的 ts 做边界，比 NOT IN 子查询更快
    sqlx::query(
        "DELETE FROM probe_samples WHERE node_id=? AND ts < \
         COALESCE((SELECT ts FROM probe_samples WHERE node_id=? ORDER BY ts DESC LIMIT 1 OFFSET 239), 0)",
    )
    .bind(id)
    .bind(id)
    .execute(pool)
    .await?;
    let (health, fail_count, down_since): (String, i64, Option<i64>) =
        sqlx::query_as("SELECT health, fail_count, down_since FROM nodes WHERE id=?")
            .bind(id)
            .fetch_one(pool)
            .await?;

    if ok {
        // 从故障恢复：累加本次故障时长
        let downtime_add = if health == "unhealthy" {
            down_since.map(|ds| (now - ds).max(0)).unwrap_or(0)
        } else {
            0
        };
        if health == "unhealthy" {
            tracing::info!(node = %id, downtime_ms = downtime_add, "node recovered");
        }
        sqlx::query(
            "UPDATE nodes SET health='healthy', latency_ms=?, fail_count=0, \
             probe_total=probe_total+1, probe_ok=probe_ok+1, \
             down_since=NULL, downtime_ms=downtime_ms+? WHERE id=?",
        )
        .bind(rtt)
        .bind(downtime_add)
        .bind(id)
        .execute(pool)
        .await?;
    } else {
        let new_fail = fail_count + 1;
        let newly_down = new_fail >= fail_threshold && health != "unhealthy";
        if newly_down {
            tracing::warn!(node = %id, fail_count = new_fail, "node marked unhealthy");
            sqlx::query(
                "UPDATE nodes SET health='unhealthy', latency_ms=NULL, fail_count=?, \
                 probe_total=probe_total+1, fail_events=fail_events+1, down_since=? WHERE id=?",
            )
            .bind(new_fail)
            .bind(now)
            .bind(id)
            .execute(pool)
            .await?;
        } else {
            // 探测失败本次无延迟数据，清空避免显示过期值
            sqlx::query(
                "UPDATE nodes SET fail_count=?, latency_ms=NULL, probe_total=probe_total+1 WHERE id=?",
            )
            .bind(new_fail)
            .bind(id)
            .execute(pool)
            .await?;
        }
    }
    Ok(())
}
