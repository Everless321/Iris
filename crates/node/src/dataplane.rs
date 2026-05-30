use anyhow::Result;
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::{Arc, RwLock};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::transport::{Channel, ClientTlsConfig, Endpoint};
use tonic::{Request, Response, Status, Streaming};

use crate::lb::{ConnGuard, LoadBalancer, NodeView};
use zhuanfa_proto::control::data_plane_client::DataPlaneClient;
use zhuanfa_proto::control::data_plane_server::DataPlane;
use zhuanfa_proto::control::{Chunk, Hop, TunnelHeader};

const BUF: usize = 16 * 1024;

/// 双向桥接协调：任一方向结束即中止另一方向。
fn link(mut a: tokio::task::JoinHandle<()>, mut b: tokio::task::JoinHandle<()>) {
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

pub async fn connect_dataplane(ctx: &NodeCtx, addr: &str) -> Result<Channel> {
    Ok(Endpoint::from_shared(format!("https://{addr}"))?
        .tls_config(ctx.tls_client.clone())?
        .connect()
        .await?)
}

/// 为下一跳节点组选路并建连，组内逐个候选 failover。
/// 返回（下游响应流, 下游发送端, 实际节点连接计数守卫）。
async fn connect_next(
    ctx: &NodeCtx,
    lb: &LoadBalancer,
    remaining_hops: &[Hop],
    target: &str,
    client_ip: &str,
    forward_id: i64,
    hop_index: u32,
    view: &NodeView,
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
        match try_open(ctx, &addr, &rest, target, client_ip, forward_id, hop_index + 1).await {
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

async fn try_open(
    ctx: &NodeCtx,
    addr: &str,
    rest_hops: &[Hop],
    target: &str,
    client_ip: &str,
    forward_id: i64,
    next_hop_index: u32,
) -> Result<(Streaming<Chunk>, mpsc::Sender<Chunk>)> {
    let channel = connect_dataplane(ctx, addr).await?;
    let mut client = DataPlaneClient::new(channel);
    let (req_tx, req_rx) = mpsc::channel::<Chunk>(64);
    req_tx
        .send(Chunk {
            header: Some(TunnelHeader {
                remaining_hops: rest_hops.to_vec(),
                target: target.to_string(),
                client_ip: client_ip.to_string(),
                forward_id,
                hop_index: next_hop_index,
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
pub async fn run_multi_hop_entry(
    listen_port: u16,
    forward_id: i64,
    hops: Vec<Hop>,
    target: String,
    ctx: Arc<NodeCtx>,
    lb: Arc<LoadBalancer>,
) -> Result<()> {
    let l = tokio::net::TcpListener::bind(("0.0.0.0", listen_port)).await?;
    tracing::info!(listen_port, hops = hops.len(), %target, "entry listening (distributed LB + failover)");
    loop {
        let (inbound, peer) = l.accept().await?;
        let hops_rest = hops[1..].to_vec();
        let client_ip = peer.ip().to_string();
        let (target, ctx, lb) = (target.clone(), ctx.clone(), lb.clone());
        tokio::spawn(async move {
            if let Err(e) =
                handle_entry_conn(inbound, hops_rest, &target, &client_ip, forward_id, &ctx, &lb)
                    .await
            {
                tracing::warn!(error = %e, "entry conn failed");
            }
        });
    }
}

async fn handle_entry_conn(
    inbound: TcpStream,
    hops_rest: Vec<Hop>,
    target: &str,
    client_ip: &str,
    forward_id: i64,
    ctx: &NodeCtx,
    lb: &LoadBalancer,
) -> Result<()> {
    let view = ctx.view();
    let (resp, req_tx, guard) =
        connect_next(ctx, lb, &hops_rest, target, client_ip, forward_id, 1, &view).await?;
    let _g = guard; // 连接存活期间维持计数
    bridge_tcp(inbound, resp, req_tx).await;
    Ok(())
}

/// 桥接 client TCP <-> 下游隧道流。
async fn bridge_tcp(inbound: TcpStream, mut resp: Streaming<Chunk>, req_tx: mpsc::Sender<Chunk>) {
    let (mut tr, mut tw) = inbound.into_split();
    tokio::select! {
        _ = async {
            let mut buf = vec![0u8; BUF];
            loop {
                match tr.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if req_tx.send(Chunk { header: None, data: buf[..n].to_vec() }).await.is_err() {
                            break;
                        }
                    }
                }
            }
        } => {}
        _ = async {
            while let Ok(Some(c)) = resp.message().await {
                if tw.write_all(&c.data).await.is_err() {
                    break;
                }
            }
        } => {}
    }
}

/// 中转 / 出口节点的隧道服务。
pub struct DataPlaneSvc {
    pub ctx: Arc<NodeCtx>,
    pub lb: Arc<LoadBalancer>,
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
            // 出口：连最终目标
            let target = header.target.clone();
            let tcp = TcpStream::connect(&target)
                .await
                .map_err(|e| Status::unavailable(format!("connect {target}: {e}")))?;
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
            // 中转：为下一跳组选路 + failover，桥接上游流 <-> 下游流
            let view = self.ctx.view();
            let (mut resp, down_tx, guard) = connect_next(
                &self.ctx,
                &self.lb,
                &header.remaining_hops,
                &header.target,
                &header.client_ip,
                header.forward_id,
                header.hop_index,
                &view,
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
}
