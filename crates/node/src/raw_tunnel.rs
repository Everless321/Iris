// Phase 9a: 节点间数据面替代实现。
// 直接 mTLS over TCP + 4-byte BE length-prefix framing，绕过 tonic / gRPC HTTP/2。
// 每条连接服务一条 forward tunnel（与 gRPC Tunnel 语义一致）。

use anyhow::{anyhow, Context, Result};
use prost::Message;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::{ClientConfig, RootCertStore, ServerConfig};
use std::io::BufReader;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt, ReadHalf, WriteHalf};
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream as ClientTlsStream;
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_rustls::server::TlsStream as ServerTlsStream;
use tokio_rustls::{TlsAcceptor, TlsConnector};

use crate::dataplane::{effective_targets, link, NodeCtx, TargetRouter, TrafficCounter};
use crate::lb::{LoadBalancer, NodeView};
use crate::sock;
use iris_proto::control::{Hop, TargetEndpoint, TunnelHeader};

const BUF: usize = 64 * 1024;
const MAX_HEADER: usize = 64 * 1024;
const MAX_FRAME: usize = 256 * 1024;

/// 用启动时已加载的 PEM 构造 rustls 双向 mTLS 配置（serve + dial）。
/// 与现有 tonic ServerTlsConfig/ClientTlsConfig 同源（同 CA / 同 identity），
/// 因此老 gRPC 7444 与新 raw 7445 是同一信任域。
pub fn build_configs(
    ca_pem: &[u8],
    cert_pem: &[u8],
    key_pem: &[u8],
) -> Result<(Arc<ServerConfig>, Arc<ClientConfig>)> {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    let ca_certs: Vec<CertificateDer<'static>> = rustls_pemfile::certs(&mut BufReader::new(ca_pem))
        .filter_map(|r| r.ok())
        .collect();
    if ca_certs.is_empty() {
        return Err(anyhow!("no CA cert found in ca.pem"));
    }
    let mut roots = RootCertStore::empty();
    for c in &ca_certs {
        roots
            .add(c.clone())
            .map_err(|e| anyhow!("add CA to root store: {e:?}"))?;
    }
    let roots_arc = Arc::new(roots);

    let identity_certs: Vec<CertificateDer<'static>> =
        rustls_pemfile::certs(&mut BufReader::new(cert_pem))
            .filter_map(|r| r.ok())
            .collect();
    if identity_certs.is_empty() {
        return Err(anyhow!("no identity cert found in client.pem"));
    }
    let private_key: PrivateKeyDer<'static> = rustls_pemfile::private_key(&mut BufReader::new(key_pem))
        .map_err(|e| anyhow!("parse private key: {e}"))?
        .ok_or_else(|| anyhow!("no private key in client-key.pem"))?;

    let client_verifier =
        rustls::server::WebPkiClientVerifier::builder(roots_arc.clone()).build()?;
    let server_cfg = ServerConfig::builder()
        .with_client_cert_verifier(client_verifier)
        .with_single_cert(identity_certs.clone(), private_key.clone_key())?;

    let client_cfg = ClientConfig::builder()
        .with_root_certificates(roots_arc.as_ref().clone())
        .with_client_auth_cert(identity_certs, private_key)?;

    Ok((Arc::new(server_cfg), Arc::new(client_cfg)))
}

async fn read_frame<R: AsyncReadExt + Unpin>(
    r: &mut R,
    max: usize,
) -> std::io::Result<Option<Vec<u8>>> {
    let mut len_buf = [0u8; 4];
    match r.read_exact(&mut len_buf).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_be_bytes(len_buf) as usize;
    if len == 0 {
        return Ok(None);
    }
    if len > max {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("frame too large: {len}"),
        ));
    }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf).await?;
    Ok(Some(buf))
}

async fn write_frame<W: AsyncWriteExt + Unpin>(w: &mut W, data: &[u8]) -> std::io::Result<()> {
    let len = data.len() as u32;
    w.write_all(&len.to_be_bytes()).await?;
    if !data.is_empty() {
        w.write_all(data).await?;
    }
    Ok(())
}

/// "host:7444" → "host:7445"
pub(crate) fn grpc_to_raw_addr(addr: &str) -> Option<String> {
    let (host, port_s) = addr.rsplit_once(':')?;
    let port: u16 = port_s.parse().ok()?;
    Some(format!("{host}:{}", port.checked_add(1)?))
}

// ============================== Server ==============================

pub async fn serve(
    addr: SocketAddr,
    tls_acceptor: TlsAcceptor,
    tls_connector: TlsConnector,
    ctx: Arc<NodeCtx>,
    lb: Arc<LoadBalancer>,
    target_router: Arc<TargetRouter>,
) -> Result<()> {
    let listener = sock::tcp_listen(addr)?;
    tracing::info!(%addr, "raw_tunnel server listening (mTLS)");
    loop {
        let (sock, _peer) = match listener.accept().await {
            Ok(x) => x,
            Err(e) => {
                tracing::warn!(error = %e, "raw accept");
                continue;
            }
        };
        sock::tune_accepted(&sock);
        let acceptor = tls_acceptor.clone();
        let connector = tls_connector.clone();
        let ctx = ctx.clone();
        let lb = lb.clone();
        let tr = target_router.clone();
        tokio::spawn(async move {
            let tls = match acceptor.accept(sock).await {
                Ok(s) => s,
                Err(e) => {
                    tracing::debug!(error = %e, "raw tls handshake");
                    return;
                }
            };
            if let Err(e) = handle_conn(tls, connector, ctx, lb, tr).await {
                tracing::debug!(error = %e, "raw conn ended");
            }
        });
    }
}

async fn handle_conn(
    tls: ServerTlsStream<TcpStream>,
    tls_connector: TlsConnector,
    ctx: Arc<NodeCtx>,
    lb: Arc<LoadBalancer>,
    tr: Arc<TargetRouter>,
) -> Result<()> {
    let (mut r, w) = tokio::io::split(tls);
    let header_bytes = read_frame(&mut r, MAX_HEADER)
        .await?
        .ok_or_else(|| anyhow!("missing tunnel header"))?;
    let header = TunnelHeader::decode(&*header_bytes).context("decode TunnelHeader")?;
    // Phase 9c 后 raw_tunnel 仅服务 TCP 路径。UDP 走 quic_tunnel。
    // 容错：收到带 udp_src_addr 的 header 说明对端是 stale node，记 warn 拒绝。
    if !header.udp_src_addr.is_empty() {
        return Err(anyhow!(
            "raw_tunnel received UDP-marked header from stale peer (src={})",
            header.udp_src_addr
        ));
    }
    if header.remaining_hops.is_empty() {
        exit_tcp(&header, &tr, r, w).await
    } else {
        relay(&header, ctx, lb, tls_connector, r, w).await
    }
}

async fn exit_tcp(
    header: &TunnelHeader,
    tr: &TargetRouter,
    mut r: ReadHalf<ServerTlsStream<TcpStream>>,
    mut w: WriteHalf<ServerTlsStream<TcpStream>>,
) -> Result<()> {
    let targets = effective_targets(header);
    if targets.is_empty() {
        return Err(anyhow!("no targets"));
    }
    let ordered = tr.order(
        &targets,
        &header.target_strategy,
        &header.client_ip,
        header.forward_id,
    );
    let mut tcp_opt: Option<TcpStream> = None;
    let mut picked = String::new();
    for addr in &ordered {
        match sock::tcp_connect(addr).await {
            Ok(s) => {
                picked = addr.clone();
                tcp_opt = Some(s);
                break;
            }
            Err(e) => tracing::debug!(target = %addr, error = %e, "raw exit failover"),
        }
    }
    let tcp = tcp_opt.ok_or_else(|| anyhow!("all tcp targets failed"))?;
    tracing::info!(target = %picked, "raw exit tcp picked");
    let (mut t_r, mut t_w) = tcp.into_split();

    let up = tokio::spawn(async move {
        loop {
            match read_frame(&mut r, MAX_FRAME).await {
                Ok(Some(data)) => {
                    if t_w.write_all(&data).await.is_err() {
                        break;
                    }
                }
                _ => break,
            }
        }
    });
    let down = tokio::spawn(async move {
        let mut buf = vec![0u8; BUF];
        loop {
            match t_r.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if write_frame(&mut w, &buf[..n]).await.is_err() {
                        break;
                    }
                }
            }
        }
    });
    link(up, down);
    Ok(())
}

async fn relay(
    header: &TunnelHeader,
    ctx: Arc<NodeCtx>,
    lb: Arc<LoadBalancer>,
    tls_connector: TlsConnector,
    mut r: ReadHalf<ServerTlsStream<TcpStream>>,
    mut w: WriteHalf<ServerTlsStream<TcpStream>>,
) -> Result<()> {
    let relay_targets = effective_targets(header);
    let view = ctx.view();
    let (mut nr, mut nw) = open_next_hop(
        &ctx,
        &lb,
        &header.remaining_hops,
        &relay_targets,
        &header.target_strategy,
        &header.client_ip,
        header.forward_id,
        header.hop_index,
        &header.udp_src_addr,
        &view,
        tls_connector,
    )
    .await?;

    // 中转 = 解密后的明文字节流透传。frame 边界在 entry/exit 维护，relay 不需要解析。
    // tokio::io::copy 用 8KB 栈 buffer 流式拷贝，无 Vec alloc + 无 length-prefix decode + 无 task spawn 开销。
    // 任一方向 EOF/Err 自动结束（try_join 任一 Err 立即 cancel 另一边，等价原 link 行为）。
    let _ = tokio::try_join!(
        tokio::io::copy(&mut r, &mut nw),
        tokio::io::copy(&mut nr, &mut w),
    );
    Ok(())
}

// ============================== Client ==============================

/// 向下一跳建立 raw mTLS tunnel，发送 header 首帧，返回 split 读写两端。
/// 入口节点用、relay 节点中转也用。
#[allow(clippy::too_many_arguments)]
pub async fn open_next_hop(
    ctx: &NodeCtx,
    lb: &LoadBalancer,
    remaining_hops: &[Hop],
    targets: &[TargetEndpoint],
    target_strategy: &str,
    client_ip: &str,
    forward_id: i64,
    hop_index: u32,
    udp_src_addr: &str,
    view: &NodeView,
    tls_connector: TlsConnector,
) -> Result<(
    ReadHalf<ClientTlsStream<TcpStream>>,
    WriteHalf<ClientTlsStream<TcpStream>>,
)> {
    let hop = &remaining_hops[0];
    let ip: IpAddr = client_ip
        .parse()
        .unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED));
    let candidates = lb.select_ordered(forward_id, hop_index as usize, hop, ip, view);
    let rest: Vec<Hop> = remaining_hops[1..].to_vec();
    for node_id in &candidates {
        let addr = match ctx.addr_of(node_id) {
            Some(a) => a,
            None => continue,
        };
        let raw_addr = match grpc_to_raw_addr(&addr) {
            Some(a) => a,
            None => continue,
        };
        match try_open(
            node_id,
            &raw_addr,
            &rest,
            targets,
            target_strategy,
            client_ip,
            forward_id,
            hop_index + 1,
            udp_src_addr,
            tls_connector.clone(),
        )
        .await
        {
            Ok(parts) => {
                tracing::info!(hop = hop_index, pick = %node_id, "raw next-hop selected");
                return Ok(parts);
            }
            Err(e) => {
                tracing::warn!(node = %node_id, error = %e, "raw next-hop failed");
                continue;
            }
        }
    }
    Err(anyhow!("hop {}: all raw candidates failed", hop_index))
}

#[allow(deprecated, clippy::too_many_arguments)]
async fn try_open(
    peer_node_id: &str,
    addr: &str,
    rest_hops: &[Hop],
    targets: &[TargetEndpoint],
    target_strategy: &str,
    client_ip: &str,
    forward_id: i64,
    next_hop_index: u32,
    udp_src_addr: &str,
    tls_connector: TlsConnector,
) -> Result<(
    ReadHalf<ClientTlsStream<TcpStream>>,
    WriteHalf<ClientTlsStream<TcpStream>>,
)> {
    let sock = sock::tcp_connect(addr).await?;
    // SNI 用对方 node_id：rustls 校验对端 cert SAN 含此 node_id，绑定 cert 到具体节点身份。
    let server_name = ServerName::try_from(peer_node_id.to_string())?;
    let tls = tls_connector.connect(server_name, sock).await?;
    let (r, mut w) = tokio::io::split(tls);

    let legacy_target = targets
        .first()
        .map(|t| t.addr.clone())
        .unwrap_or_default();
    let header = TunnelHeader {
        remaining_hops: rest_hops.to_vec(),
        target: legacy_target,
        client_ip: client_ip.to_string(),
        forward_id,
        hop_index: next_hop_index,
        targets: targets.to_vec(),
        target_strategy: target_strategy.to_string(),
        udp_src_addr: udp_src_addr.to_string(),
    };
    let buf = header.encode_to_vec();
    write_frame(&mut w, &buf).await?;
    Ok((r, w))
}

// ============================== Entry helpers ==============================

/// TCP 入口连接 → raw tunnel 桥接。traffic 由入口节点的 ActiveForward 共享 (仅入口统计)。
pub async fn handle_entry_tcp(
    inbound: TcpStream,
    nr: ReadHalf<ClientTlsStream<TcpStream>>,
    nw: WriteHalf<ClientTlsStream<TcpStream>>,
    traffic: Arc<TrafficCounter>,
) {
    let (mut ir, mut iw) = inbound.into_split();
    let mut nr = nr;
    let mut nw = nw;
    let traffic_up = traffic.clone();
    let up = tokio::spawn(async move {
        let mut buf = vec![0u8; BUF];
        loop {
            match ir.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    traffic_up.add_in(n);
                    if write_frame(&mut nw, &buf[..n]).await.is_err() {
                        break;
                    }
                }
            }
        }
    });
    let traffic_dn = traffic;
    let down = tokio::spawn(async move {
        loop {
            match read_frame(&mut nr, MAX_FRAME).await {
                Ok(Some(data)) => {
                    let n = data.len();
                    if iw.write_all(&data).await.is_err() {
                        break;
                    }
                    traffic_dn.add_out(n);
                }
                _ => break,
            }
        }
    });
    link(up, down);
}

