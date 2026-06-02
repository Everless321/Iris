// Phase 9a: 节点间数据面替代实现。
// 直接 mTLS over TCP + 4-byte BE length-prefix framing，绕过 tonic / gRPC HTTP/2。
// 每条连接服务一条 forward tunnel（与 gRPC Tunnel 语义一致）。
//
// #27 链路加密开关：
//   - TLS 模式（默认）：监听 grpc+1 端口（7445），rustls mTLS
//   - plain 模式：监听 grpc+3 端口（7447），TCP 不裹 TLS
//   两者协议层完全一致（4B len-prefix header + 流式 frame），仅传输层差异。
//   节点端永远同时监听两个端口；客户端按 forward.link_encryption 决定连哪个。
//   relay 节点从 TunnelHeader.link_encryption 读取并 propagate 到下一跳。

use anyhow::{anyhow, Context, Result};
use prost::Message;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::{ClientConfig, RootCertStore, ServerConfig};
use std::io::BufReader;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_rustls::{TlsAcceptor, TlsConnector};

use crate::dataplane::{effective_targets, link, NodeCtx, TargetRouter, TrafficCounter};
use crate::lb::{LoadBalancer, NodeView};
use crate::sock;
use iris_proto::control::{Hop, TargetEndpoint, TunnelHeader};

const BUF: usize = 64 * 1024;
const MAX_HEADER: usize = 64 * 1024;
const MAX_FRAME: usize = 256 * 1024;

/// 统一的 transport 读写半端（TLS 或 plain TCP 都装箱成它）。
pub type DynRead = Box<dyn AsyncRead + Unpin + Send>;
pub type DynWrite = Box<dyn AsyncWrite + Unpin + Send>;

/// 用启动时已加载的 PEM 构造 rustls 双向 mTLS 配置（serve + dial）。
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

async fn read_frame<R: AsyncRead + Unpin>(
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

async fn write_frame<W: AsyncWrite + Unpin>(w: &mut W, data: &[u8]) -> std::io::Result<()> {
    let len = data.len() as u32;
    w.write_all(&len.to_be_bytes()).await?;
    if !data.is_empty() {
        w.write_all(data).await?;
    }
    Ok(())
}

/// "host:7444" → "host:7445" (TLS raw)
pub(crate) fn grpc_to_raw_addr(addr: &str) -> Option<String> {
    let (host, port_s) = addr.rsplit_once(':')?;
    let port: u16 = port_s.parse().ok()?;
    Some(format!("{host}:{}", port.checked_add(1)?))
}

/// "host:7444" → "host:7447" (plain raw, #27)
pub(crate) fn grpc_to_raw_plain_addr(addr: &str) -> Option<String> {
    let (host, port_s) = addr.rsplit_once(':')?;
    let port: u16 = port_s.parse().ok()?;
    Some(format!("{host}:{}", port.checked_add(3)?))
}

/// "plain" → plain；其余视为 "tls"
fn is_plain(s: &str) -> bool {
    s.eq_ignore_ascii_case("plain")
}

// ============================== Server ==============================

/// TLS raw_tunnel listener（端口 = grpc + 1）。
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
            let (r, w) = tokio::io::split(tls);
            let dr: DynRead = Box::new(r);
            let dw: DynWrite = Box::new(w);
            if let Err(e) = handle_conn(dr, dw, connector, ctx, lb, tr).await {
                tracing::debug!(error = %e, "raw conn ended");
            }
        });
    }
}

/// #27 plain raw_tunnel listener（端口 = grpc + 3）。TCP 直通，不裹 TLS。
pub async fn serve_plain(
    addr: SocketAddr,
    tls_connector: TlsConnector,
    ctx: Arc<NodeCtx>,
    lb: Arc<LoadBalancer>,
    target_router: Arc<TargetRouter>,
) -> Result<()> {
    let listener = sock::tcp_listen(addr)?;
    tracing::info!(%addr, "raw_tunnel server listening (plain, #27)");
    loop {
        let (sock, _peer) = match listener.accept().await {
            Ok(x) => x,
            Err(e) => {
                tracing::warn!(error = %e, "raw_plain accept");
                continue;
            }
        };
        sock::tune_accepted(&sock);
        let connector = tls_connector.clone();
        let ctx = ctx.clone();
        let lb = lb.clone();
        let tr = target_router.clone();
        tokio::spawn(async move {
            let (r, w) = sock.into_split();
            let dr: DynRead = Box::new(r);
            let dw: DynWrite = Box::new(w);
            if let Err(e) = handle_conn(dr, dw, connector, ctx, lb, tr).await {
                tracing::debug!(error = %e, "raw_plain conn ended");
            }
        });
    }
}

async fn handle_conn(
    mut r: DynRead,
    w: DynWrite,
    tls_connector: TlsConnector,
    ctx: Arc<NodeCtx>,
    lb: Arc<LoadBalancer>,
    tr: Arc<TargetRouter>,
) -> Result<()> {
    let header_bytes = read_frame(&mut r, MAX_HEADER)
        .await?
        .ok_or_else(|| anyhow!("missing tunnel header"))?;
    let header = TunnelHeader::decode(&*header_bytes).context("decode TunnelHeader")?;
    // raw_tunnel 仅服务 TCP；UDP 走 quic_tunnel。
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
    mut r: DynRead,
    mut w: DynWrite,
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
    mut r: DynRead,
    mut w: DynWrite,
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
        &header.link_encryption,
        &view,
        tls_connector,
    )
    .await?;

    // 中转 = 流式透传。frame 边界在 entry/exit 维护，relay 不解析。
    let _ = tokio::try_join!(
        tokio::io::copy(&mut r, &mut nw),
        tokio::io::copy(&mut nr, &mut w),
    );
    Ok(())
}

// ============================== Client ==============================

/// 向下一跳建立 raw tunnel（TLS or plain），发送 header 首帧，返回 split 读写两端。
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
    link_encryption: &str,
    view: &NodeView,
    tls_connector: TlsConnector,
) -> Result<(DynRead, DynWrite)> {
    let hop = &remaining_hops[0];
    let ip: IpAddr = client_ip
        .parse()
        .unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED));
    let candidates = lb.select_ordered(forward_id, hop_index as usize, hop, ip, view);
    let rest: Vec<Hop> = remaining_hops[1..].to_vec();
    let plain = is_plain(link_encryption);
    for node_id in &candidates {
        let addr = match ctx.addr_of(node_id) {
            Some(a) => a,
            None => continue,
        };
        let next_addr = if plain {
            grpc_to_raw_plain_addr(&addr)
        } else {
            grpc_to_raw_addr(&addr)
        };
        let next_addr = match next_addr {
            Some(a) => a,
            None => continue,
        };
        match try_open(
            node_id,
            &next_addr,
            &rest,
            targets,
            target_strategy,
            client_ip,
            forward_id,
            hop_index + 1,
            udp_src_addr,
            link_encryption,
            plain,
            tls_connector.clone(),
        )
        .await
        {
            Ok(parts) => {
                tracing::info!(hop = hop_index, pick = %node_id, plain, "raw next-hop selected");
                return Ok(parts);
            }
            Err(e) => {
                tracing::warn!(node = %node_id, error = %e, plain, "raw next-hop failed");
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
    link_encryption: &str,
    plain: bool,
    tls_connector: TlsConnector,
) -> Result<(DynRead, DynWrite)> {
    let sock = sock::tcp_connect(addr).await?;
    let (dr, mut dw): (DynRead, DynWrite) = if plain {
        let (r, w) = sock.into_split();
        (Box::new(r), Box::new(w))
    } else {
        // SNI = peer node_id → rustls 校验 cert SAN
        let server_name = ServerName::try_from(peer_node_id.to_string())?;
        let tls = tls_connector.connect(server_name, sock).await?;
        let (r, w) = tokio::io::split(tls);
        (Box::new(r), Box::new(w))
    };

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
        link_encryption: link_encryption.to_string(),
    };
    let buf = header.encode_to_vec();
    write_frame(&mut dw, &buf).await?;
    Ok((dr, dw))
}

// ============================== Entry helpers ==============================

/// TCP 入口连接 → raw tunnel 桥接。
pub async fn handle_entry_tcp(
    inbound: TcpStream,
    nr: DynRead,
    nw: DynWrite,
    traffic: Arc<TrafficCounter>,
    session: Arc<crate::session::SessionState>,
    rate: Arc<crate::ratelimit::RateLimit>,
) {
    let (mut ir, mut iw) = inbound.into_split();
    let mut nr = nr;
    let mut nw = nw;
    let traffic_up = traffic.clone();
    let sess_up = session.clone();
    let rate_up = rate.up.clone();
    let up = tokio::spawn(async move {
        let mut buf = vec![0u8; BUF];
        loop {
            match ir.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    crate::ratelimit::throttle(&rate_up, n).await;
                    traffic_up.add_in(n);
                    sess_up.add_in(n);
                    if write_frame(&mut nw, &buf[..n]).await.is_err() {
                        break;
                    }
                }
            }
        }
    });
    let traffic_dn = traffic;
    let sess_dn = session;
    let rate_down = rate.down.clone();
    let down = tokio::spawn(async move {
        loop {
            match read_frame(&mut nr, MAX_FRAME).await {
                Ok(Some(data)) => {
                    let n = data.len();
                    crate::ratelimit::throttle(&rate_down, n).await;
                    if iw.write_all(&data).await.is_err() {
                        break;
                    }
                    traffic_dn.add_out(n);
                    sess_dn.add_out(n);
                }
                _ => break,
            }
        }
    });
    link(up, down);
}
