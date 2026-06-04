use anyhow::Result;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
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
    rate: Arc<crate::ratelimit::RateLimit>,
    bind_result: tokio::sync::oneshot::Sender<Result<(), String>>,
) -> Result<()> {
    let bind_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), listen_port);
    let l = match sock::tcp_listen(bind_addr) {
        Ok(l) => {
            let _ = bind_result.send(Ok(()));
            l
        }
        Err(e) => {
            let msg = format!("tcp bind {listen_port}: {e}");
            let _ = bind_result.send(Err(msg.clone()));
            return Err(anyhow::anyhow!(msg));
        }
    };
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
        let (targets, strategy, router, traffic, sessions, entry_node_id, rate) = (
            targets.clone(),
            strategy.clone(),
            router.clone(),
            traffic.clone(),
            sessions.clone(),
            entry_node_id.clone(),
            rate.clone(),
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
            // bytes_in  = 客户端 → target（入口视角"上传"）
            // bytes_out = target → 客户端（"下载"）
            let traffic_up = traffic.clone();
            let sess_up = session.clone();
            let traffic_dn = traffic.clone();
            let sess_dn = session.clone();
            let on_up = move |n: usize| {
                traffic_up.add_in(n);
                sess_up.add_in(n);
            };
            let on_down = move |n: usize| {
                traffic_dn.add_out(n);
                sess_dn.add_out(n);
            };
            forward_bidirectional(
                inbound,
                outbound,
                rate.up.clone(),
                rate.down.clone(),
                on_up,
                on_down,
            )
            .await;
            session.close("normal");
        });
    }
}

/// 单跳双向转发分发：
/// - Linux: `splice(2)` 零拷贝，单流 TCP 接近裸金属带宽
/// - 其他平台: tokio `copy_bidirectional`，buf 内核管理，比手动 read/write 略快
///
/// 两路径都通过回调把搬运字节数喂给 traffic/session 计数器 + ratelimit。
#[cfg(target_os = "linux")]
async fn forward_bidirectional<U, D>(
    inbound: TcpStream,
    outbound: TcpStream,
    rate_up: Option<Arc<crate::ratelimit::Limiter>>,
    rate_down: Option<Arc<crate::ratelimit::Limiter>>,
    on_up: U,
    on_down: D,
) where
    U: FnMut(usize) + Send + 'static,
    D: FnMut(usize) + Send + 'static,
{
    crate::zero_copy::splice_bidirectional(inbound, outbound, rate_up, rate_down, on_up, on_down)
        .await;
}

/// 非 Linux fallback：copy_bidirectional 不支持 per-byte 回调，
/// 只能在结束时一次性结算（精度足够，因为统计是累计量）。限速在该路径下退化为不生效，
/// 等同于 buf-based 实现里 None limiter 的 noop 路径（macOS/Windows 通常仅开发环境用）。
#[cfg(not(target_os = "linux"))]
async fn forward_bidirectional<U, D>(
    mut inbound: TcpStream,
    mut outbound: TcpStream,
    _rate_up: Option<Arc<crate::ratelimit::Limiter>>,
    _rate_down: Option<Arc<crate::ratelimit::Limiter>>,
    mut on_up: U,
    mut on_down: D,
) where
    U: FnMut(usize) + Send + 'static,
    D: FnMut(usize) + Send + 'static,
{
    match tokio::io::copy_bidirectional(&mut inbound, &mut outbound).await {
        Ok((up, down)) => {
            on_up(up as usize);
            on_down(down as usize);
        }
        Err(e) => tracing::debug!(error = %e, "copy_bidirectional ended"),
    }
}
