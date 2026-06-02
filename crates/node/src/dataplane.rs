use anyhow::Result;
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc;

/// 单条 forward 的字节流计数器（仅入口节点累计）。
/// bytes_in = 客户端→入口（上行）；bytes_out = 入口→客户端（下行）。
/// 用 AtomicU64 + Relaxed：单次 fetch_add ~5ns，9Gbps 路径无感知。
#[derive(Default, Debug)]
pub struct TrafficCounter {
    pub bytes_in: AtomicU64,
    pub bytes_out: AtomicU64,
}
impl TrafficCounter {
    #[inline]
    pub fn add_in(&self, n: usize) {
        self.bytes_in.fetch_add(n as u64, Ordering::Relaxed);
    }
    #[inline]
    pub fn add_out(&self, n: usize) {
        self.bytes_out.fetch_add(n as u64, Ordering::Relaxed);
    }
    /// M4.2-D fast path 用：把 kernel counter 绝对值灌进来。
    /// 与 add_in/add_out 互斥使用（fast forward 只用 set_absolute，slow forward 只用 add_*）。
    #[inline]
    pub fn set_in_absolute(&self, n: u64) {
        self.bytes_in.store(n, Ordering::Relaxed);
    }
    #[allow(dead_code)] // V2 反向 counter 用
    #[inline]
    pub fn set_out_absolute(&self, n: u64) {
        self.bytes_out.store(n, Ordering::Relaxed);
    }
}

use crate::sock;
use tokio_stream::wrappers::ReceiverStream;
use tonic::transport::{Channel, ClientTlsConfig, Endpoint};
use tonic::{Request, Response, Status, Streaming};

use crate::lb::{ConnGuard, LoadBalancer, NodeView};
use iris_proto::control::data_plane_client::DataPlaneClient;
use iris_proto::control::data_plane_server::DataPlane;
use iris_proto::control::{
    Chunk, Hop, ProbeReachReply, ProbeReachRequest, TargetEndpoint, TunnelHeader,
};

/// 出口 target 选择：按 target_strategy 排序候选 addr，调用方依次尝试 failover。
pub struct TargetRouter {
    rr: Mutex<HashMap<i64, AtomicUsize>>,
}
impl TargetRouter {
    pub fn new() -> Self {
        Self { rr: Mutex::new(HashMap::new()) }
    }
    pub fn order(
        &self,
        targets: &[TargetEndpoint],
        strategy: &str,
        client_ip: &str,
        forward_id: i64,
    ) -> Vec<String> {
        let valid: Vec<&TargetEndpoint> =
            targets.iter().filter(|t| !t.addr.trim().is_empty()).collect();
        if valid.is_empty() {
            return Vec::new();
        }
        if valid.len() == 1 {
            return vec![valid[0].addr.clone()];
        }
        match strategy {
            "source_hash" => self.source_hash(&valid, client_ip),
            // least_conn / latency 在 target 维度没有可靠统计，回退到 weighted
            _ => self.weighted(&valid, forward_id),
        }
    }
    fn weighted(&self, pool: &[&TargetEndpoint], fid: i64) -> Vec<String> {
        let mut expanded: Vec<&str> = Vec::new();
        for t in pool {
            for _ in 0..t.weight.max(1) {
                expanded.push(&t.addr);
            }
        }
        let primary_idx = {
            let mut g = self.rr.lock().unwrap();
            let c = g.entry(fid).or_insert_with(|| AtomicUsize::new(0));
            c.fetch_add(1, Ordering::Relaxed) % expanded.len()
        };
        let primary = expanded[primary_idx].to_string();
        let mut out = vec![primary.clone()];
        for t in pool {
            if t.addr != primary && !out.contains(&t.addr) {
                out.push(t.addr.clone());
            }
        }
        out
    }
    fn source_hash(&self, pool: &[&TargetEndpoint], client_ip: &str) -> Vec<String> {
        let ip: IpAddr = client_ip
            .parse()
            .unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED));
        let mut scored: Vec<(f64, String)> = pool
            .iter()
            .map(|t| {
                let mut h = DefaultHasher::new();
                ip.hash(&mut h);
                t.addr.hash(&mut h);
                let frac = h.finish() as f64 / u64::MAX as f64;
                (t.weight.max(1) as f64 * frac, t.addr.clone())
            })
            .collect();
        scored.sort_by(|a, b| {
            b.0.partial_cmp(&a.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.1.cmp(&b.1))
        });
        scored.into_iter().map(|(_, a)| a).collect()
    }
}
impl Default for TargetRouter {
    fn default() -> Self {
        Self::new()
    }
}

const BUF: usize = 64 * 1024;
pub(crate) const UDP_BUF: usize = 64 * 1024;

/// 从 TunnelHeader 取出有效目标列表：优先用新字段 targets，
/// 不存在时回退到旧的单字符串 target（兼容滚动升级期）。
#[allow(deprecated)]
pub(crate) fn effective_targets(h: &TunnelHeader) -> Vec<TargetEndpoint> {
    if !h.targets.is_empty() {
        return h.targets.clone();
    }
    let t = h.target.trim();
    if t.is_empty() {
        Vec::new()
    } else {
        vec![TargetEndpoint { addr: t.into(), weight: 1 }]
    }
}

/// 双向桥接协调：任一方向结束即中止另一方向。
pub(crate) fn link(mut a: tokio::task::JoinHandle<()>, mut b: tokio::task::JoinHandle<()>) {
    tokio::spawn(async move {
        tokio::select! {
            _ = &mut a => b.abort(),
            _ = &mut b => a.abort(),
        }
    });
}

pub struct NodeInfo {
    pub addr: String,
    pub health: String,
    pub latency_ms: u32,
}

pub struct NodeCtx {
    pub nodes: RwLock<HashMap<String, NodeInfo>>,
    pub tls_client: ClientTlsConfig,
    /// Phase 9a：raw_tunnel TCP 拨号用，与 tls_client 同信任域
    pub raw_connector: tokio_rustls::TlsConnector,
    /// Phase 9c：QUIC endpoint（同 endpoint 兼 server + client，避免双开 UDP port）
    pub quic_endpoint: quinn::Endpoint,
    pub quic_client_cfg: Arc<quinn::ClientConfig>,
}

impl NodeCtx {
    pub fn addr_of(&self, id: &str) -> Option<String> {
        self.nodes.read().unwrap().get(id).map(|n| n.addr.clone())
    }
    pub fn view(&self) -> NodeView {
        let g = self.nodes.read().unwrap();
        NodeView {
            health: g.iter().map(|(k, v)| (k.clone(), v.health.clone())).collect(),
            latency: g.iter().map(|(k, v)| (k.clone(), v.latency_ms)).collect(),
        }
    }
}

pub async fn connect_dataplane(
    ctx: &NodeCtx,
    peer_node_id: &str,
    addr: &str,
) -> Result<Channel> {
    // SNI 用对方 node_id：rustls 校验对端 cert SAN 包含此 node_id，绑定 cert 到具体身份。
    let tls = ctx.tls_client.clone().domain_name(peer_node_id);
    Ok(Endpoint::from_shared(format!("https://{addr}"))?
        .tls_config(tls)?
        .connect()
        .await?)
}

/// 为下一跳节点组选路并建连，组内逐个候选 failover。
/// 返回（下游响应流, 下游发送端, 实际节点连接计数守卫）。
/// `udp_src_addr` 非空 → 透传给下游表明这是 UDP 隧道；TCP 路径传 ""。
pub(crate) async fn connect_next(
    ctx: &NodeCtx,
    lb: &LoadBalancer,
    remaining_hops: &[Hop],
    targets: &[TargetEndpoint],
    target_strategy: &str,
    client_ip: &str,
    forward_id: i64,
    hop_index: u32,
    view: &NodeView,
    udp_src_addr: &str,
) -> Result<(Streaming<Chunk>, mpsc::Sender<Chunk>, ConnGuard)> {
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
        match try_open(
            ctx, node_id, &addr, &rest, targets, target_strategy, client_ip, forward_id,
            hop_index + 1, udp_src_addr,
        )
        .await
        {
            Ok((resp, req_tx)) => {
                tracing::info!(hop = hop_index, pick = %node_id, "next-hop selected");
                return Ok((resp, req_tx, lb.track(node_id)));
            }
            Err(e) => {
                tracing::warn!(node = %node_id, error = %e, "next-hop failed, failover");
                continue;
            }
        }
    }
    anyhow::bail!("hop {hop_index}: all candidates failed")
}

#[allow(deprecated, clippy::too_many_arguments)]
async fn try_open(
    ctx: &NodeCtx,
    peer_node_id: &str,
    addr: &str,
    rest_hops: &[Hop],
    targets: &[TargetEndpoint],
    target_strategy: &str,
    client_ip: &str,
    forward_id: i64,
    next_hop_index: u32,
    udp_src_addr: &str,
) -> Result<(Streaming<Chunk>, mpsc::Sender<Chunk>)> {
    let channel = connect_dataplane(ctx, peer_node_id, addr).await?;
    let mut client = DataPlaneClient::new(channel);
    let (req_tx, req_rx) = mpsc::channel::<Chunk>(64);
    // 兼容字段：填入 targets[0] 给可能存在的旧版本节点
    let legacy_target = targets.first().map(|t| t.addr.clone()).unwrap_or_default();
    req_tx
        .send(Chunk {
            header: Some(TunnelHeader {
                remaining_hops: rest_hops.to_vec(),
                target: legacy_target,
                client_ip: client_ip.to_string(),
                forward_id,
                hop_index: next_hop_index,
                targets: targets.to_vec(),
                target_strategy: target_strategy.to_string(),
                udp_src_addr: udp_src_addr.to_string(),
                link_encryption: String::new(),
            }),
            data: vec![],
        })
        .await?;
    let resp = client
        .tunnel(ReceiverStream::new(req_rx))
        .await?
        .into_inner();
    Ok((resp, req_tx))
}

/// 入口节点：监听端口，每个客户端连接经全链路（每跳 failover）转发。
/// `sessions` / `entry_node_id` 用于 #36 会话级历史记录。
pub async fn run_multi_hop_entry(
    listen_port: u16,
    forward_id: i64,
    hops: Vec<Hop>,
    targets: Vec<TargetEndpoint>,
    target_strategy: String,
    ctx: Arc<NodeCtx>,
    lb: Arc<LoadBalancer>,
    traffic: Arc<TrafficCounter>,
    sessions: Arc<crate::session::SessionTable>,
    entry_node_id: Arc<String>,
    rate: Arc<crate::ratelimit::RateLimit>,
    link_encryption: String,
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
        hops = hops.len(),
        targets = targets.len(),
        "entry listening (distributed LB + failover)"
    );
    let targets = Arc::new(targets);
    let target_strategy = Arc::new(target_strategy);
    let link_encryption = Arc::new(link_encryption);
    loop {
        let (inbound, peer) = l.accept().await?;
        sock::tune_accepted(&inbound);
        let hops_rest = hops[1..].to_vec();
        let client_ip = peer.ip().to_string();
        let client_port = peer.port() as u32;
        let (targets, strategy, ctx, lb, traffic, sessions, entry_node_id, rate, link_enc) = (
            targets.clone(),
            target_strategy.clone(),
            ctx.clone(),
            lb.clone(),
            traffic.clone(),
            sessions.clone(),
            entry_node_id.clone(),
            rate.clone(),
            link_encryption.clone(),
        );
        tokio::spawn(async move {
            if let Err(e) = handle_entry_conn(
                inbound, hops_rest, &targets, &strategy, &client_ip, client_port,
                forward_id, &ctx, &lb, &traffic, &sessions, &entry_node_id, &rate, &link_enc,
            )
            .await
            {
                tracing::warn!(error = %e, "entry conn failed");
            }
        });
    }
}

#[allow(clippy::too_many_arguments)]
async fn handle_entry_conn(
    inbound: TcpStream,
    hops_rest: Vec<Hop>,
    targets: &[TargetEndpoint],
    target_strategy: &str,
    client_ip: &str,
    client_port: u32,
    forward_id: i64,
    ctx: &NodeCtx,
    lb: &LoadBalancer,
    traffic: &Arc<TrafficCounter>,
    sessions: &Arc<crate::session::SessionTable>,
    entry_node_id: &str,
    rate: &Arc<crate::ratelimit::RateLimit>,
    link_encryption: &str,
) -> Result<()> {
    let view = ctx.view();
    // hops_path V1：仅入口节点 id（V2 再展开实际选中的每跳节点）。
    // target_addr：配置中第一个 target（V2 再换成实际选中）。
    let target_addr = targets.first().map(|t| t.addr.clone()).unwrap_or_default();
    let session = sessions.create(
        forward_id,
        entry_node_id,
        client_ip.to_string(),
        client_port,
        target_addr,
        vec![entry_node_id.to_string()],
        "tcp",
    );
    let result = if std::env::var("IRIS_DISABLE_RAW").as_deref() == Ok("1") {
        let (resp, req_tx, guard) = connect_next(
            ctx, lb, &hops_rest, targets, target_strategy, client_ip,
            forward_id, 1, &view, "",
        )
        .await?;
        let _g = guard;
        bridge_tcp(inbound, resp, req_tx, traffic.clone(), session.clone(), rate.clone()).await;
        Ok::<(), anyhow::Error>(())
    } else {
        let (nr, nw) = crate::raw_tunnel::open_next_hop(
            ctx, lb, &hops_rest, targets, target_strategy, client_ip,
            forward_id, 1, "", link_encryption, &view, ctx.raw_connector.clone(),
        )
        .await?;
        crate::raw_tunnel::handle_entry_tcp(inbound, nr, nw, traffic.clone(), session.clone(), rate.clone()).await;
        Ok::<(), anyhow::Error>(())
    };
    session.close(if result.is_ok() { "normal" } else { "error" });
    result
}

/// 桥接 client TCP <-> 下游隧道流。
async fn bridge_tcp(
    inbound: TcpStream,
    mut resp: Streaming<Chunk>,
    req_tx: mpsc::Sender<Chunk>,
    traffic: Arc<TrafficCounter>,
    session: Arc<crate::session::SessionState>,
    rate: Arc<crate::ratelimit::RateLimit>,
) {
    let (mut tr, mut tw) = inbound.into_split();
    let traffic_up = traffic.clone();
    let traffic_dn = traffic;
    let sess_up = session.clone();
    let sess_dn = session;
    let rate_up = rate.up.clone();
    let rate_down = rate.down.clone();
    tokio::select! {
        _ = async {
            let mut buf = vec![0u8; BUF];
            loop {
                match tr.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        crate::ratelimit::throttle(&rate_up, n).await;
                        traffic_up.add_in(n);
                        sess_up.add_in(n);
                        if req_tx.send(Chunk { header: None, data: buf[..n].to_vec() }).await.is_err() {
                            break;
                        }
                    }
                }
            }
        } => {}
        _ = async {
            while let Ok(Some(c)) = resp.message().await {
                let n = c.data.len();
                crate::ratelimit::throttle(&rate_down, n).await;
                if tw.write_all(&c.data).await.is_err() {
                    break;
                }
                traffic_dn.add_out(n);
                sess_dn.add_out(n);
            }
        } => {}
    }
}

/// 中转 / 出口节点的隧道服务。
pub struct DataPlaneSvc {
    pub ctx: Arc<NodeCtx>,
    pub lb: Arc<LoadBalancer>,
    pub target_router: Arc<TargetRouter>,
}

#[tonic::async_trait]
impl DataPlane for DataPlaneSvc {
    type TunnelStream = ReceiverStream<Result<Chunk, Status>>;

    async fn tunnel(
        &self,
        req: Request<Streaming<Chunk>>,
    ) -> Result<Response<Self::TunnelStream>, Status> {
        let mut inbound = req.into_inner();
        let first = inbound
            .message()
            .await?
            .ok_or_else(|| Status::invalid_argument("empty tunnel stream"))?;
        let header = first
            .header
            .ok_or_else(|| Status::invalid_argument("missing tunnel header"))?;
        let (tx, rx) = mpsc::channel::<Result<Chunk, Status>>(64);

        if header.remaining_hops.is_empty() {
            let targets = effective_targets(&header);
            if targets.is_empty() {
                return Err(Status::invalid_argument("no targets in tunnel header"));
            }
            let ordered = self.target_router.order(
                &targets,
                &header.target_strategy,
                &header.client_ip,
                header.forward_id,
            );
            if header.udp_src_addr.is_empty() {
                // === TCP 出口：逐个 target failover ===
                let mut tcp_opt: Option<TcpStream> = None;
                let mut picked_addr = String::new();
                let mut last_err = String::from("no targets tried");
                for addr in &ordered {
                    match sock::tcp_connect(addr).await {
                        Ok(s) => {
                            picked_addr = addr.clone();
                            tcp_opt = Some(s);
                            break;
                        }
                        Err(e) => {
                            tracing::warn!(target = %addr, error = %e, "target failover");
                            last_err = format!("{addr}: {e}");
                        }
                    }
                }
                let tcp = tcp_opt.ok_or_else(|| {
                    Status::unavailable(format!("all targets failed: {last_err}"))
                })?;
                tracing::info!(target = %picked_addr, "exit: tcp target picked");
                let (mut tr, mut tw) = tcp.into_split();
                if !first.data.is_empty() {
                    if let Err(e) = tw.write_all(&first.data).await {
                        return Err(Status::unavailable(format!("write first frame: {e}")));
                    }
                }
                let up = tokio::spawn(async move {
                    while let Ok(Some(c)) = inbound.message().await {
                        if tw.write_all(&c.data).await.is_err() {
                            break;
                        }
                    }
                });
                let down = tokio::spawn(async move {
                    let mut buf = vec![0u8; BUF];
                    loop {
                        match tr.read(&mut buf).await {
                            Ok(0) | Err(_) => break,
                            Ok(n) => {
                                if tx
                                    .send(Ok(Chunk { header: None, data: buf[..n].to_vec() }))
                                    .await
                                    .is_err()
                                {
                                    break;
                                }
                            }
                        }
                    }
                });
                link(up, down);
            } else {
                // === UDP 出口：选一个 target 建 UdpSocket，session 跟 tunnel 同生命周期 ===
                // UDP 无连接 → 没法判断"连不通"，只取首选 target，不做 failover。
                let pick = ordered.first().cloned().ok_or_else(|| {
                    Status::unavailable("no usable udp target".to_string())
                })?;
                let usock = sock::udp_bind(SocketAddr::new(
                    IpAddr::V4(Ipv4Addr::UNSPECIFIED),
                    0,
                ))
                .map_err(|e| Status::unavailable(format!("udp bind: {e}")))?;
                usock.connect(&pick).await.map_err(|e| {
                    Status::unavailable(format!("udp connect {pick}: {e}"))
                })?;
                let usock = Arc::new(usock);
                tracing::info!(target = %pick, src = %header.udp_src_addr, "exit: udp target picked");
                if !first.data.is_empty() {
                    let _ = usock.send(&first.data).await;
                }
                let usock_up = usock.clone();
                let up = tokio::spawn(async move {
                    while let Ok(Some(c)) = inbound.message().await {
                        if !c.data.is_empty() && usock_up.send(&c.data).await.is_err() {
                            break;
                        }
                    }
                });
                let usock_dn = usock;
                let down = tokio::spawn(async move {
                    let mut buf = vec![0u8; UDP_BUF];
                    loop {
                        match usock_dn.recv(&mut buf).await {
                            Ok(0) => break,
                            Ok(n) => {
                                if tx
                                    .send(Ok(Chunk { header: None, data: buf[..n].to_vec() }))
                                    .await
                                    .is_err()
                                {
                                    break;
                                }
                            }
                            Err(_) => break,
                        }
                    }
                });
                link(up, down);
            }
        } else {
            // 中转：为下一跳组选路 + failover，桥接上游流 <-> 下游流
            let view = self.ctx.view();
            let relay_targets = effective_targets(&header);
            let (mut resp, down_tx, guard) = connect_next(
                &self.ctx,
                &self.lb,
                &header.remaining_hops,
                &relay_targets,
                &header.target_strategy,
                &header.client_ip,
                header.forward_id,
                header.hop_index,
                &view,
                &header.udp_src_addr,
            )
            .await
            .map_err(|e| Status::unavailable(e.to_string()))?;
            if !first.data.is_empty() {
                let _ = down_tx.send(Chunk { header: None, data: first.data }).await;
            }
            let up = tokio::spawn(async move {
                while let Ok(Some(c)) = inbound.message().await {
                    if down_tx.send(Chunk { header: None, data: c.data }).await.is_err() {
                        break;
                    }
                }
            });
            let dn = tokio::spawn(async move {
                let _g = guard; // 维持下游节点连接计数
                while let Ok(Some(c)) = resp.message().await {
                    if tx.send(Ok(c)).await.is_err() {
                        break;
                    }
                }
            });
            link(up, dn);
        }
        Ok(Response::new(ReceiverStream::new(rx)))
    }

    async fn probe_reach(
        &self,
        req: Request<ProbeReachRequest>,
    ) -> Result<Response<ProbeReachReply>, Status> {
        let r = req.into_inner();
        let timeout = match r.timeout_ms {
            0 => std::time::Duration::from_millis(2000),
            n => std::time::Duration::from_millis(n.min(10_000) as u64),
        };
        let started = std::time::Instant::now();
        match tokio::time::timeout(timeout, TcpStream::connect(&r.addr)).await {
            Ok(Ok(_)) => Ok(Response::new(ProbeReachReply {
                ok: true,
                latency_ms: started.elapsed().as_millis() as u64,
                error: String::new(),
            })),
            Ok(Err(e)) => Ok(Response::new(ProbeReachReply {
                ok: false,
                latency_ms: 0,
                error: e.to_string(),
            })),
            Err(_) => Ok(Response::new(ProbeReachReply {
                ok: false,
                latency_ms: 0,
                error: format!("timeout {}ms", timeout.as_millis()),
            })),
        }
    }
}
