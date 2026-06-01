use anyhow::Result;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use iris_proto::control::TargetEndpoint;

use crate::dataplane::{TargetRouter, TrafficCounter};
use crate::session::SessionTable;
use crate::sock;

/// 单跳入口：监听本地端口，按 target_strategy 在多个 target 之间挑一个并 failover。
/// `traffic` 由 main.rs::spawn_forward 创建并共享给 ActiveForward + 数据面 spawn task,
/// heartbeat 周期收集 AtomicU64 累计值上报 master。
/// `sessions` 用于 #36 会话级历史记录：每条 accept 的 TCP 连接建一个 SessionState。
pub async fn run_single_hop(
    listen_port: u16,
    forward_id: i64,
    targets: Vec<TargetEndpoint>,
    target_strategy: String,
    traffic: Arc<TrafficCounter>,
    sessions: Arc<SessionTable>,
    entry_node_id: Arc<String>,
) -> Result<()> {
    let bind_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), listen_port);
    let l = sock::tcp_listen(bind_addr)?;
    tracing::info!(
        listen_port,
        targets = targets.len(),
        %target_strategy,
        "entry listening (single-hop multi-target)"
    );
    let targets = Arc::new(targets);
    let strategy = Arc::new(target_strategy);
    let router = Arc::new(TargetRouter::new());
    loop {
        let (inbound, peer) = l.accept().await?;
        sock::tune_accepted(&inbound);
        let client_ip = peer.ip().to_string();
        let client_port = peer.port() as u32;
        let (targets, strategy, router, traffic, sessions, entry_node_id) = (
            targets.clone(),
            strategy.clone(),
            router.clone(),
            traffic.clone(),
            sessions.clone(),
            entry_node_id.clone(),
        );
        tokio::spawn(async move {
            let ordered = router.order(&targets, &strategy, &client_ip, forward_id);
            let mut connected: Option<(TcpStream, String)> = None;
            for addr in &ordered {
                match sock::tcp_connect(addr).await {
                    Ok(s) => {
                        connected = Some((s, addr.clone()));
                        break;
                    }
                    Err(e) => tracing::warn!(target = %addr, error = %e, "single-hop target failover"),
                }
            }
            let Some((outbound, target_addr)) = connected else {
                tracing::warn!("single-hop: all targets failed");
                return;
            };
            // 建会话：单跳路径 = 仅入口节点
            let session = sessions.create(
                forward_id,
                &entry_node_id,
                client_ip,
                client_port,
                target_addr,
                vec![(*entry_node_id).clone()],
                "tcp",
            );
            // split + 手动双向 loop 以便统计字节数。
            // inbound = 客户端连接,outbound = target 连接。
            // bytes_in  = 入口 inbound.read = 客户端发来的字节
            // bytes_out = 入口 inbound.write = 写回客户端的字节
            let (mut ir, mut iw) = inbound.into_split();
            let (mut tr, mut tw) = outbound.into_split();
            let traffic_up = traffic.clone();
            let sess_up = session.clone();
            let up = tokio::spawn(async move {
                let mut buf = vec![0u8; 64 * 1024];
                loop {
                    match ir.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            traffic_up.add_in(n);
                            sess_up.add_in(n);
                            if tw.write_all(&buf[..n]).await.is_err() {
                                break;
                            }
                        }
                    }
                }
            });
            let traffic_dn = traffic.clone();
            let sess_dn = session.clone();
            let down = tokio::spawn(async move {
                let mut buf = vec![0u8; 64 * 1024];
                loop {
                    match tr.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            if iw.write_all(&buf[..n]).await.is_err() {
                                break;
                            }
                            traffic_dn.add_out(n);
                            sess_dn.add_out(n);
                        }
                    }
                }
            });
            tokio::select! { _ = up => {}, _ = down => {} }
            session.close("normal");
        });
    }
}
