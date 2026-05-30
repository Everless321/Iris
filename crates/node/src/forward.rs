use anyhow::Result;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use tokio::io::copy_bidirectional;
use tokio::net::TcpStream;
use zhuanfa_proto::control::TargetEndpoint;

use crate::dataplane::TargetRouter;
use crate::sock;

/// 单跳入口：监听本地端口，按 target_strategy 在多个 target 之间挑一个并 failover。
pub async fn run_single_hop(
    listen_port: u16,
    forward_id: i64,
    targets: Vec<TargetEndpoint>,
    target_strategy: String,
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
        let (mut inbound, peer) = l.accept().await?;
        sock::tune_accepted(&inbound);
        let client_ip = peer.ip().to_string();
        let (targets, strategy, router) = (targets.clone(), strategy.clone(), router.clone());
        tokio::spawn(async move {
            let ordered = router.order(&targets, &strategy, &client_ip, forward_id);
            let mut connected: Option<TcpStream> = None;
            for addr in &ordered {
                match sock::tcp_connect(addr).await {
                    Ok(s) => {
                        connected = Some(s);
                        break;
                    }
                    Err(e) => tracing::warn!(target = %addr, error = %e, "single-hop target failover"),
                }
            }
            match connected {
                Some(mut outbound) => {
                    if let Err(e) = copy_bidirectional(&mut inbound, &mut outbound).await {
                        tracing::debug!(error = %e, "tcp copy ended");
                    }
                }
                None => tracing::warn!("single-hop: all targets failed"),
            }
        });
    }
}
