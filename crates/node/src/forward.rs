use anyhow::Result;
use tokio::io::copy_bidirectional;
use tokio::net::{TcpListener, TcpStream};

/// 单跳入口：监听本地端口，每个客户端连接直接透传到 target。
/// 多跳（经下游节点）的入口逻辑在 1.5 用 DataPlane 隧道实现。
pub async fn run_single_hop(listen_port: u16, target: String) -> Result<()> {
    let l = TcpListener::bind(("0.0.0.0", listen_port)).await?;
    tracing::info!(listen_port, %target, "entry listening (single-hop)");
    loop {
        let (mut inbound, _peer) = l.accept().await?;
        let target = target.clone();
        tokio::spawn(async move {
            match TcpStream::connect(&target).await {
                Ok(mut outbound) => {
                    if let Err(e) = copy_bidirectional(&mut inbound, &mut outbound).await {
                        tracing::debug!(error = %e, "tcp copy ended");
                    }
                }
                Err(e) => tracing::warn!(%target, error = %e, "connect target failed"),
            }
        });
    }
}
