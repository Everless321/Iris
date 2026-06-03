// Phase 9c: 节点间 UDP 数据面，QUIC datagram extension。
// 取代 raw_tunnel 的 UDP-over-TCP 路径，杜绝 TCP backpressure 引起的 UDP 丢包。
//
// 端口规划：7446 UDP（= gRPC port + 2）
// 协议：
//   - 每条 UDP forward session = 一个 QUIC connection（1:1，简化 P1）
//   - 首帧用 bidi stream 发 protobuf TunnelHeader
//   - 后续 UDP packet 用 QUIC datagram (unreliable/unordered) 传递
//   - exit 节点 unmarshal datagram → UdpSocket::send 给 target；反向同理
//
// ALPN: "iris-quic-1"（避免和其它 QUIC 服务混淆）

use anyhow::{anyhow, Context, Result};
use prost::Message;
use quinn::crypto::rustls::{QuicClientConfig, QuicServerConfig};
use quinn::{ClientConfig, Endpoint, MtuDiscoveryConfig, ServerConfig, TransportConfig};
use rustls::{ClientConfig as RustlsClientConfig, ServerConfig as RustlsServerConfig};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::UdpSocket;
use iris_proto::control::TunnelHeader;

const ALPN: &[&[u8]] = &[b"iris-quic-1"];

// M6 调参（2026-06-03）
// - DATAGRAM_BUF 64MB → 16MB：单 QUIC connection 收发缓冲，64MB 太大且 forward 多时内存翻倍
// - IDLE_TIMEOUT 30s → 120s：避免 NAT 中间设备清流导致重连
// - KEEP_ALIVE 15s：保活心跳，比 idle_timeout 短，确保 NAT 不超时
// - INITIAL_MTU 1200 → 1350：GCE intra-zone MTU 1460，跨太平洋 1280，取折中
// - MTU DISCOVERY 启用：上限 1452（IPv4-safe），让 quinn 自动探测最优
// - RECV_WINDOW 8MB：跨太平洋 RTT 100ms × 1Gbps BDP ≈ 12.5MB，给 8MB 已能跑 ~640Mbps 单流
// - STREAM_WINDOW 同 RECV_WINDOW，单 stream 内不阻塞
// - MAX_BIDI / MAX_UNI streams 256：高并发拨号场景
const DATAGRAM_BUF: usize = 16 * 1024 * 1024;
const IDLE_TIMEOUT_SECS: u64 = 120;
const KEEP_ALIVE_SECS: u64 = 15;
const INITIAL_MTU: u16 = 1350;
const MTU_UPPER: u16 = 1452;
const RECV_WINDOW: u32 = 8 * 1024 * 1024;
const STREAM_WINDOW: u32 = 8 * 1024 * 1024;
const MAX_BIDI_STREAMS: u32 = 256;
const MAX_UNI_STREAMS: u32 = 256;
const MAX_HEADER: usize = 64 * 1024;

/// 在 quinn 用的 rustls config 上 patch ALPN + datagram buffer 等参数，
/// 同时复用 raw_tunnel::build_configs 加载的 CA / identity（同信任域）。
pub fn make_endpoints(
    bind: SocketAddr,
    rustls_server: Arc<RustlsServerConfig>,
    rustls_client: Arc<RustlsClientConfig>,
) -> Result<(Endpoint, ClientConfig)> {
    let mut server_rustls = (*rustls_server).clone();
    server_rustls.alpn_protocols = ALPN.iter().map(|p| p.to_vec()).collect();
    let mut client_rustls = (*rustls_client).clone();
    client_rustls.alpn_protocols = ALPN.iter().map(|p| p.to_vec()).collect();

    let quic_server_crypto = Arc::new(
        QuicServerConfig::try_from(server_rustls)
            .context("rustls -> QuicServerConfig")?,
    );
    let mut server_cfg = ServerConfig::with_crypto(quic_server_crypto);
    server_cfg.transport_config(Arc::new(transport_config()));

    let quic_client_crypto = Arc::new(
        QuicClientConfig::try_from(client_rustls)
            .context("rustls -> QuicClientConfig")?,
    );
    let mut client_cfg = ClientConfig::new(quic_client_crypto);
    client_cfg.transport_config(Arc::new(transport_config()));

    let endpoint = Endpoint::server(server_cfg, bind)
        .with_context(|| format!("bind quinn endpoint @ {bind}"))?;
    Ok((endpoint, client_cfg))
}

fn transport_config() -> TransportConfig {
    let mut mtu = MtuDiscoveryConfig::default();
    mtu.upper_bound(MTU_UPPER);

    let mut t = TransportConfig::default();
    t.datagram_receive_buffer_size(Some(DATAGRAM_BUF))
        .datagram_send_buffer_size(DATAGRAM_BUF)
        .initial_mtu(INITIAL_MTU)
        .mtu_discovery_config(Some(mtu))
        .receive_window((RECV_WINDOW as u64).try_into().expect("recv window in range"))
        .stream_receive_window((STREAM_WINDOW as u64).try_into().expect("stream window in range"))
        .max_concurrent_bidi_streams(MAX_BIDI_STREAMS.into())
        .max_concurrent_uni_streams(MAX_UNI_STREAMS.into())
        .keep_alive_interval(Some(Duration::from_secs(KEEP_ALIVE_SECS)))
        .max_idle_timeout(Some(
            Duration::from_secs(IDLE_TIMEOUT_SECS)
                .try_into()
                .expect("idle timeout in range"),
        ));
    t
}

/// "host:7444" → "host:7446"（gRPC → QUIC）。-2 路径用于 raw → grpc 反向不需要。
pub fn grpc_to_quic_addr(addr: &str) -> Option<String> {
    let (host, port_s) = addr.rsplit_once(':')?;
    let port: u16 = port_s.parse().ok()?;
    Some(format!("{host}:{}", port.checked_add(2)?))
}

/// Server 端：dial-back client endpoint 建一条额外的对外 client（暂保留接口）。
/// M2/M3 实际由调用方按需 dial。
pub async fn dial(
    endpoint: &Endpoint,
    client_cfg: &ClientConfig,
    addr: SocketAddr,
    server_name: &str,
) -> Result<quinn::Connection> {
    let connecting = endpoint
        .connect_with(client_cfg.clone(), addr, server_name)
        .with_context(|| format!("quinn connect_with {addr}"))?;
    let conn = connecting.await.context("quinn connecting await")?;
    Ok(conn)
}

/// 在新建的 QUIC connection 上打开一条 bidi stream 发 TunnelHeader 首帧，
/// 返回 connection（datagram 用）+ stream 双向（保活/读 EOF）。
pub async fn send_header(
    conn: &quinn::Connection,
    header: &TunnelHeader,
) -> Result<(quinn::SendStream, quinn::RecvStream)> {
    let (mut send, recv) = conn.open_bi().await.context("open_bi")?;
    let mut buf = header.encode_to_vec();
    let len = (buf.len() as u32).to_be_bytes();
    let mut framed = Vec::with_capacity(4 + buf.len());
    framed.extend_from_slice(&len);
    framed.append(&mut buf);
    send.write_all(&framed).await.context("write header")?;
    Ok((send, recv))
}

/// 服务端接收新 QUIC connection，读首帧 TunnelHeader 解析。
pub async fn accept_header(conn: &quinn::Connection) -> Result<(TunnelHeader, quinn::SendStream, quinn::RecvStream)> {
    let (send, mut recv) = conn.accept_bi().await.context("accept_bi")?;
    let mut len_buf = [0u8; 4];
    recv.read_exact(&mut len_buf).await.context("read header len")?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len == 0 || len > MAX_HEADER {
        return Err(anyhow!("invalid header len: {len}"));
    }
    let mut hb = vec![0u8; len];
    recv.read_exact(&mut hb).await.context("read header body")?;
    let header = TunnelHeader::decode(&*hb).context("decode TunnelHeader")?;
    Ok((header, send, recv))
}

// ============================== Server ==============================

use crate::dataplane::{effective_targets, NodeCtx, TargetRouter, UDP_BUF};
use crate::lb::LoadBalancer;
use crate::sock;
use std::net::{IpAddr, Ipv4Addr};
use iris_proto::control::{Hop, TargetEndpoint};

pub async fn serve(
    endpoint: Endpoint,
    client_cfg: Arc<ClientConfig>,
    ctx: Arc<NodeCtx>,
    lb: Arc<LoadBalancer>,
    target_router: Arc<TargetRouter>,
) -> Result<()> {
    tracing::info!(addr = ?endpoint.local_addr(), "quic_tunnel server listening (mTLS, datagram)");
    loop {
        let incoming = match endpoint.accept().await {
            Some(c) => c,
            None => return Err(anyhow!("quinn endpoint closed")),
        };
        let endpoint = endpoint.clone();
        let client_cfg = client_cfg.clone();
        let ctx = ctx.clone();
        let lb = lb.clone();
        let tr = target_router.clone();
        tokio::spawn(async move {
            let conn = match incoming.await {
                Ok(c) => c,
                Err(e) => {
                    tracing::debug!(error = %e, "quic accept");
                    return;
                }
            };
            if let Err(e) = handle_conn(conn, endpoint, client_cfg, ctx, lb, tr).await {
                tracing::debug!(error = %e, "quic conn ended");
            }
        });
    }
}

async fn handle_conn(
    conn: quinn::Connection,
    endpoint: Endpoint,
    client_cfg: Arc<ClientConfig>,
    ctx: Arc<NodeCtx>,
    lb: Arc<LoadBalancer>,
    tr: Arc<TargetRouter>,
) -> Result<()> {
    let (header, _send, _recv) = accept_header(&conn).await?;
    if header.remaining_hops.is_empty() {
        exit_udp(&conn, &header, &tr).await
    } else {
        relay(conn, header, endpoint, client_cfg, ctx, lb).await
    }
}

async fn exit_udp(conn: &quinn::Connection, header: &TunnelHeader, tr: &TargetRouter) -> Result<()> {
    let targets = effective_targets(header);
    if targets.is_empty() {
        return Err(anyhow!("no udp targets"));
    }
    let ordered = tr.order(
        &targets,
        &header.target_strategy,
        &header.client_ip,
        header.forward_id,
    );
    let pick = ordered.first().cloned().ok_or_else(|| anyhow!("no udp target"))?;
    let usock = sock::udp_bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0))?;
    usock.connect(&pick).await?;
    let usock = Arc::new(usock);
    tracing::info!(target = %pick, src = %header.udp_src_addr, "quic exit udp picked");

    // up: QUIC datagram → UdpSocket::send(target)
    let conn_up = conn.clone();
    let usock_up = usock.clone();
    let mut up = tokio::spawn(async move {
        loop {
            match conn_up.read_datagram().await {
                Ok(b) if !b.is_empty() => {
                    if usock_up.send(&b).await.is_err() {
                        break;
                    }
                }
                Ok(_) => continue,
                Err(_) => break,
            }
        }
    });
    // down: UdpSocket::recv → QUIC datagram → entry
    let conn_dn = conn.clone();
    let usock_dn = usock;
    let mut down = tokio::spawn(async move {
        let mut buf = vec![0u8; UDP_BUF];
        loop {
            match usock_dn.recv(&mut buf).await {
                Ok(0) => break,
                Ok(n) => {
                    let pkt = bytes::Bytes::copy_from_slice(&buf[..n]);
                    if conn_dn.send_datagram(pkt).is_err() {
                        // quinn: 队列满会替换最老的，错误通常是 conn closed
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });
    // 任意一端结束就显式 abort 另一端，避免 task / 半开连接残留
    tokio::select! {
        _ = &mut up => { down.abort(); let _ = down.await; }
        _ = &mut down => { up.abort(); let _ = up.await; }
    }
    conn.close(0u32.into(), b"exit done");
    Ok(())
}

async fn relay(
    upstream: quinn::Connection,
    header: TunnelHeader,
    endpoint: Endpoint,
    client_cfg: Arc<ClientConfig>,
    ctx: Arc<NodeCtx>,
    lb: Arc<LoadBalancer>,
) -> Result<()> {
    let downstream = open_next_hop_inner(
        &endpoint,
        &client_cfg,
        &ctx,
        &lb,
        &header.remaining_hops,
        &effective_targets(&header),
        &header.target_strategy,
        &header.client_ip,
        header.forward_id,
        header.hop_index,
        &header.udp_src_addr,
    )
    .await?;

    let up_conn = upstream.clone();
    let dn_conn = downstream.clone();
    let mut up = tokio::spawn(async move {
        loop {
            match up_conn.read_datagram().await {
                Ok(b) if !b.is_empty() => {
                    if dn_conn.send_datagram(b).is_err() {
                        break;
                    }
                }
                Ok(_) => continue,
                Err(_) => break,
            }
        }
    });
    let dn_conn2 = downstream.clone();
    let up_conn2 = upstream.clone();
    let mut down = tokio::spawn(async move {
        loop {
            match dn_conn2.read_datagram().await {
                Ok(b) if !b.is_empty() => {
                    if up_conn2.send_datagram(b).is_err() {
                        break;
                    }
                }
                Ok(_) => continue,
                Err(_) => break,
            }
        }
    });
    tokio::select! {
        _ = &mut up => { down.abort(); let _ = down.await; }
        _ = &mut down => { up.abort(); let _ = up.await; }
    }
    // 关掉两端 connection，避免半开链路在对端等 idle_timeout 才回收
    upstream.close(0u32.into(), b"relay done");
    downstream.close(0u32.into(), b"relay done");
    Ok(())
}

// ============================== Client ==============================

/// 入口节点 / relay 节点向下一跳建立 QUIC connection 并发首帧 header。
#[allow(clippy::too_many_arguments, deprecated)]
async fn open_next_hop_inner(
    endpoint: &Endpoint,
    client_cfg: &ClientConfig,
    ctx: &NodeCtx,
    lb: &LoadBalancer,
    remaining_hops: &[Hop],
    targets: &[TargetEndpoint],
    target_strategy: &str,
    client_ip: &str,
    forward_id: i64,
    hop_index: u32,
    udp_src_addr: &str,
) -> Result<quinn::Connection> {
    use crate::lb::NodeView;
    let view: NodeView = ctx.view();
    let hop = &remaining_hops[0];
    let ip: IpAddr = client_ip
        .parse()
        .unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED));
    let candidates = lb.select_ordered(forward_id, hop_index as usize, hop, ip, &view);
    let rest: Vec<Hop> = remaining_hops[1..].to_vec();
    for node_id in &candidates {
        let addr = match ctx.addr_of(node_id) {
            Some(a) => a,
            None => continue,
        };
        let quic_addr_s = match grpc_to_quic_addr(&addr) {
            Some(a) => a,
            None => continue,
        };
        let quic_addr: SocketAddr = match tokio::net::lookup_host(&quic_addr_s)
            .await
            .ok()
            .and_then(|mut it| it.next())
        {
            Some(a) => a,
            None => continue,
        };
        // SNI 用对方 node_id：rustls 校验对端 cert SAN 含此 node_id，绑定 cert 到具体节点身份。
        let conn = match dial(endpoint, client_cfg, quic_addr, node_id).await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(node = %node_id, error = %e, "quic next-hop dial");
                continue;
            }
        };
        let header = TunnelHeader {
            remaining_hops: rest.clone(),
            target: targets.first().map(|t| t.addr.clone()).unwrap_or_default(),
            client_ip: client_ip.to_string(),
            forward_id,
            hop_index: hop_index + 1,
            targets: targets.to_vec(),
            target_strategy: target_strategy.to_string(),
            udp_src_addr: udp_src_addr.to_string(),
            link_encryption: String::new(),
        };
        if let Err(e) = send_header(&conn, &header).await {
            tracing::warn!(node = %node_id, error = %e, "quic next-hop send_header");
            continue;
        }
        tracing::info!(hop = hop_index, pick = %node_id, "quic next-hop selected");
        return Ok(conn);
    }
    Err(anyhow!("hop {}: all quic candidates failed", hop_index))
}

/// 入口 UDP forward 调用：建立到下一跳的 QUIC 连接（带 header）。
#[allow(clippy::too_many_arguments)]
pub async fn open_next_hop(
    endpoint: &Endpoint,
    client_cfg: &ClientConfig,
    ctx: &NodeCtx,
    lb: &LoadBalancer,
    remaining_hops: &[Hop],
    targets: &[TargetEndpoint],
    target_strategy: &str,
    client_ip: &str,
    forward_id: i64,
    udp_src_addr: &str,
) -> Result<quinn::Connection> {
    open_next_hop_inner(
        endpoint,
        client_cfg,
        ctx,
        lb,
        remaining_hops,
        targets,
        target_strategy,
        client_ip,
        forward_id,
        1,
        udp_src_addr,
    )
    .await
}

/// 入口反向 recv：从 QUIC datagram 读 → 主 UDP socket send_to(src)。
/// traffic.add_out 在 send_to 成功后累加 (下行字节)。
pub async fn udp_recv_loop(
    conn: quinn::Connection,
    sock: Arc<UdpSocket>,
    src: SocketAddr,
    last_seen: Arc<std::sync::atomic::AtomicI64>,
    traffic: Arc<crate::dataplane::TrafficCounter>,
) {
    loop {
        match conn.read_datagram().await {
            Ok(b) if !b.is_empty() => {
                last_seen.store(now_ms(), std::sync::atomic::Ordering::Relaxed);
                let n = b.len();
                if sock.send_to(&b, src).await.is_ok() {
                    traffic.add_out(n);
                }
            }
            Ok(_) => continue,
            Err(_) => break,
        }
    }
}

/// 入口写 datagram：直接 send_datagram（非阻塞），失败为 conn dead。
pub fn udp_send_packet(conn: &quinn::Connection, data: &[u8]) -> Result<(), quinn::SendDatagramError> {
    conn.send_datagram(bytes::Bytes::copy_from_slice(data))
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
