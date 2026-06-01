use anyhow::Result;
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::sync::RwLock;

use crate::dataplane::{NodeCtx, TargetRouter, TrafficCounter, UDP_BUF};
use crate::lb::LoadBalancer;
use crate::quic_tunnel;
use crate::sock;
use iris_proto::control::{Hop, TargetEndpoint};

const SESSION_IDLE_MS: i64 = 60_000;
const GC_INTERVAL: Duration = Duration::from_secs(10);

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

// ============================== 单跳 UDP ==============================
// 入口节点 == 出口节点。每个 src 维护一个 connected 出口 UdpSocket，
// 反向 recv task 把回包发回原始 src。

struct SingleSession {
    out: Arc<UdpSocket>,
    last_seen: AtomicI64,
}

type SingleMap = Arc<RwLock<HashMap<SocketAddr, Arc<SingleSession>>>>;

fn spawn_single_gc(map: SingleMap) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(GC_INTERVAL).await;
            let cutoff = now_ms() - SESSION_IDLE_MS;
            let dead: Vec<SocketAddr> = {
                let g = map.read().await;
                g.iter()
                    .filter(|(_, s)| s.last_seen.load(Ordering::Relaxed) < cutoff)
                    .map(|(k, _)| *k)
                    .collect()
            };
            if !dead.is_empty() {
                let mut g = map.write().await;
                for k in &dead {
                    g.remove(k);
                }
                tracing::debug!(dropped = dead.len(), "udp single sessions gc");
            }
        }
    });
}

pub async fn run_udp_single_hop(
    listen_port: u16,
    forward_id: i64,
    targets: Vec<TargetEndpoint>,
    target_strategy: String,
    target_router: Arc<TargetRouter>,
    traffic: Arc<TrafficCounter>,
    bind_result: tokio::sync::oneshot::Sender<Result<(), String>>,
) -> Result<()> {
    let bind_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), listen_port);
    let sock = match sock::udp_bind(bind_addr) {
        Ok(s) => {
            let _ = bind_result.send(Ok(()));
            Arc::new(s)
        }
        Err(e) => {
            let msg = format!("udp bind {listen_port}: {e}");
            let _ = bind_result.send(Err(msg.clone()));
            return Err(anyhow::anyhow!(msg));
        }
    };
    tracing::info!(
        listen_port,
        targets = targets.len(),
        %target_strategy,
        "udp entry listening (single-hop)"
    );
    let map: SingleMap = Arc::default();
    spawn_single_gc(map.clone());
    let targets = Arc::new(targets);
    let strategy = Arc::new(target_strategy);

    let mut buf = vec![0u8; UDP_BUF];
    loop {
        let (n, src) = match sock.recv_from(&mut buf).await {
            Ok(x) => x,
            Err(e) => {
                tracing::warn!(error = %e, "udp recv_from");
                continue;
            }
        };
        traffic.add_in(n);
        let data = buf[..n].to_vec();

        // 命中复用
        if let Some(s) = map.read().await.get(&src).cloned() {
            s.last_seen.store(now_ms(), Ordering::Relaxed);
            let _ = s.out.send(&data).await;
            continue;
        }

        // 新会话：选 target → 建出口 socket → 入 map → 起反向 recv task → 发首包
        let ordered = target_router.order(&targets, &strategy, &src.ip().to_string(), forward_id);
        let pick = match ordered.first() {
            Some(p) => p.clone(),
            None => {
                tracing::warn!("no udp target");
                continue;
            }
        };
        let out_bind = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0);
        let out = match sock::udp_bind(out_bind) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, "udp out bind");
                continue;
            }
        };
        if let Err(e) = out.connect(&pick).await {
            tracing::warn!(target = %pick, error = %e, "udp out connect");
            continue;
        }
        let out = Arc::new(out);
        let session = Arc::new(SingleSession {
            out: out.clone(),
            last_seen: AtomicI64::new(now_ms()),
        });
        map.write().await.insert(src, session.clone());
        {
            let (sock_back, out_back, map_back, sess_back, src_back, traffic_back) = (
                sock.clone(), out.clone(), map.clone(), session.clone(), src, traffic.clone(),
            );
            tokio::spawn(async move {
                let mut buf = vec![0u8; UDP_BUF];
                loop {
                    match tokio::time::timeout(GC_INTERVAL, out_back.recv(&mut buf)).await {
                        Ok(Ok(n)) if n > 0 => {
                            sess_back.last_seen.store(now_ms(), Ordering::Relaxed);
                            if sock_back.send_to(&buf[..n], src_back).await.is_err() {
                                break;
                            }
                            traffic_back.add_out(n);
                        }
                        Ok(_) => break,
                        Err(_) => {
                            if !map_back.read().await.contains_key(&src_back) {
                                break;
                            }
                        }
                    }
                }
            });
        }
        let _ = out.send(&data).await;
    }
}

// ============================== 多跳 UDP ==============================
// 每个 src 一条 mTLS gRPC tunnel。隧道首帧 header.udp_src_addr = src.to_string()，
// 出口节点据此识别 UDP 路径。

struct MultiSession {
    /// Phase 9c：QUIC connection（每个 UDP src 一条）。Connection 是 cheap clone（内部 Arc）。
    conn: quinn::Connection,
    last_seen: Arc<AtomicI64>,
}

type MultiMap = Arc<RwLock<HashMap<SocketAddr, Arc<MultiSession>>>>;

fn spawn_multi_gc(map: MultiMap) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(GC_INTERVAL).await;
            let cutoff = now_ms() - SESSION_IDLE_MS;
            let dead: Vec<SocketAddr> = {
                let g = map.read().await;
                g.iter()
                    .filter(|(_, s)| s.last_seen.load(Ordering::Relaxed) < cutoff)
                    .map(|(k, _)| *k)
                    .collect()
            };
            if !dead.is_empty() {
                let mut g = map.write().await;
                for k in &dead {
                    g.remove(k);
                }
                tracing::debug!(dropped = dead.len(), "udp multi sessions gc");
            }
        }
    });
}

pub async fn run_udp_multi_hop(
    listen_port: u16,
    forward_id: i64,
    hops: Vec<Hop>,
    targets: Vec<TargetEndpoint>,
    target_strategy: String,
    ctx: Arc<NodeCtx>,
    lb: Arc<LoadBalancer>,
    traffic: Arc<TrafficCounter>,
    bind_result: tokio::sync::oneshot::Sender<Result<(), String>>,
) -> Result<()> {
    let bind_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), listen_port);
    let sock = match sock::udp_bind(bind_addr) {
        Ok(s) => {
            let _ = bind_result.send(Ok(()));
            Arc::new(s)
        }
        Err(e) => {
            let msg = format!("udp bind {listen_port}: {e}");
            let _ = bind_result.send(Err(msg.clone()));
            return Err(anyhow::anyhow!(msg));
        }
    };
    tracing::info!(
        listen_port,
        hops = hops.len(),
        targets = targets.len(),
        "udp entry listening (multi-hop)"
    );
    let hops_rest: Vec<Hop> = hops[1..].to_vec();
    let map: MultiMap = Arc::default();
    spawn_multi_gc(map.clone());

    let mut buf = vec![0u8; UDP_BUF];
    loop {
        let (n, src) = match sock.recv_from(&mut buf).await {
            Ok(x) => x,
            Err(e) => {
                tracing::warn!(error = %e, "udp recv_from");
                continue;
            }
        };
        traffic.add_in(n);
        let data = buf[..n].to_vec();

        // 命中复用
        if let Some(s) = map.read().await.get(&src).cloned() {
            s.last_seen.store(now_ms(), Ordering::Relaxed);
            if quic_tunnel::udp_send_packet(&s.conn, &data).is_err() {
                map.write().await.remove(&src);
            }
            continue;
        }

        // 新会话：起 QUIC 连接（datagram 模式，无 backpressure）
        let conn = match quic_tunnel::open_next_hop(
            &ctx.quic_endpoint,
            &ctx.quic_client_cfg,
            &ctx,
            &lb,
            &hops_rest,
            &targets,
            &target_strategy,
            &src.ip().to_string(),
            forward_id,
            &src.to_string(),
        )
        .await
        {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(error = %e, %src, "udp quic tunnel open failed");
                continue;
            }
        };
        let last_seen = Arc::new(AtomicI64::new(now_ms()));
        let session = Arc::new(MultiSession {
            conn: conn.clone(),
            last_seen: last_seen.clone(),
        });
        map.write().await.insert(src, session);
        {
            let (sock_back, map_back, src_back, ls, c, traffic_back) =
                (sock.clone(), map.clone(), src, last_seen.clone(), conn.clone(), traffic.clone());
            tokio::spawn(async move {
                quic_tunnel::udp_recv_loop(c, sock_back, src_back, ls, traffic_back).await;
                map_back.write().await.remove(&src_back);
            });
        }
        let _ = quic_tunnel::udp_send_packet(&conn, &data);
    }
}
