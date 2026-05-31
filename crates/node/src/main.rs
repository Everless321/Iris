mod dataplane;
mod forward;
mod lb;
mod raw_tunnel;
mod sock;
mod udp_forward;

use anyhow::Result;
use dataplane::{DataPlaneSvc, NodeCtx, NodeInfo};
use lb::LoadBalancer;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tonic::transport::{
    Certificate, ClientTlsConfig, Endpoint, Identity, Server, ServerTlsConfig,
};
use zhuanfa_proto::control::control_client::ControlClient;
use zhuanfa_proto::control::data_plane_server::DataPlaneServer;
use zhuanfa_proto::control::{HeartbeatRequest, NodeAddr, SyncRequest};

fn build_nodes(ns: &[NodeAddr]) -> HashMap<String, NodeInfo> {
    ns.iter()
        .map(|n| {
            (
                n.id.clone(),
                NodeInfo {
                    addr: n.addr.clone(),
                    health: n.health.clone(),
                    latency_ms: n.latency_ms,
                },
            )
        })
        .collect()
}

fn env(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.into())
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr()))
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let master = env("ZF_MASTER", "https://127.0.0.1:7443");
    let cert_dir = env("ZF_CERT_DIR", "certs");
    let node_id = env("ZF_NODE_ID", "node-dev-1");
    let data_addr = env("ZF_DATA_ADDR", "0.0.0.0:7444");

    // 等待证书就绪（容器编排下启动顺序不保证）
    let p = |f: &str| format!("{cert_dir}/{f}");
    while !Path::new(&p("client.pem")).exists() {
        tracing::info!("waiting for certs...");
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    let ca_pem = std::fs::read(p("ca.pem"))?;
    let cert_pem = std::fs::read(p("client.pem"))?;
    let key_pem = std::fs::read(p("client-key.pem"))?;
    // tonic 用（保持 gRPC 数据面 + 控制面兼容）
    let ca = Certificate::from_pem(&ca_pem);
    let identity = Identity::from_pem(&cert_pem, &key_pem);
    let tls_client = ClientTlsConfig::new()
        .ca_certificate(ca.clone())
        .identity(identity.clone())
        .domain_name("localhost");
    let dp_tls = ServerTlsConfig::new()
        .identity(identity)
        .client_ca_root(ca);
    // raw_tunnel 用（rustls 原生，bypass HTTP/2 framing）
    let (raw_server_cfg, raw_client_cfg) =
        raw_tunnel::build_configs(&ca_pem, &cert_pem, &key_pem)?;
    let raw_acceptor = tokio_rustls::TlsAcceptor::from(raw_server_cfg);
    let raw_connector = tokio_rustls::TlsConnector::from(raw_client_cfg);

    // 连 master（重试直到就绪）
    let channel = loop {
        match Endpoint::from_shared(master.clone())
            .and_then(|e| e.tls_config(tls_client.clone()))
        {
            Ok(ep) => match ep.connect().await {
                Ok(c) => break c,
                Err(e) => tracing::warn!(error = %e, "connect failed, retry 2s"),
            },
            Err(e) => tracing::warn!(error = %e, "endpoint config error, retry 2s"),
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    };
    let mut client = ControlClient::new(channel);
    tracing::info!(%master, %node_id, "connected (mTLS)");

    // 首次拉配置：得转发规则 + 全量节点地址表
    let reply = client
        .sync_config(SyncRequest { node_id: node_id.clone() })
        .await?
        .into_inner();
    tracing::info!(forwards = reply.forwards.len(), nodes = reply.nodes.len(), "config synced");
    let ctx = Arc::new(NodeCtx {
        nodes: RwLock::new(build_nodes(&reply.nodes)),
        tls_client,
        raw_connector: raw_connector.clone(),
    });

    // LB 由 DataPlane server 与入口共享（中转节点也需 LB 选下一跳）
    let lb = Arc::new(LoadBalancer::new());

    // 出口 target 路由（按 forward_id 维护加权 RR 游标）
    let target_router = Arc::new(dataplane::TargetRouter::new());

    // gRPC 数据面服务（端口 7444，被 master ProbeReach 调用 + raw 不可用时的 fallback）
    let dp_addr: SocketAddr = data_addr.parse()?;
    {
        let (ctx, lb, target_router) = (ctx.clone(), lb.clone(), target_router.clone());
        tokio::spawn(async move {
            let svc = DataPlaneSvc { ctx, lb, target_router };
            if let Err(e) = Server::builder()
                .tls_config(dp_tls)
                .expect("dp tls")
                .add_service(DataPlaneServer::new(svc))
                .serve(dp_addr)
                .await
            {
                tracing::error!(error = %e, "dataplane server exited");
            }
        });
        tracing::info!(%data_addr, "dataplane listening (mTLS, gRPC)");
    }

    // raw_tunnel 数据面服务（端口 = grpc + 1，bypass HTTP/2 framing；Phase 9a）
    let raw_addr = SocketAddr::new(dp_addr.ip(), dp_addr.port() + 1);
    {
        let (ctx, lb, target_router) = (ctx.clone(), lb.clone(), target_router.clone());
        let acceptor = raw_acceptor.clone();
        let connector = raw_connector.clone();
        tokio::spawn(async move {
            if let Err(e) =
                raw_tunnel::serve(raw_addr, acceptor, connector, ctx, lb, target_router).await
            {
                tracing::error!(error = %e, "raw_tunnel server exited");
            }
        });
    }

    // 启动入口监听器（本节点出现在某转发的第一跳节点组中）
    for f in reply.forwards {
        let is_entry = f
            .hops
            .first()
            .map(|h| h.nodes.iter().any(|n| n.id == node_id))
            == Some(true);
        if !is_entry {
            continue;
        }
        let port = f.listen_port as u16;
        // 取出多 target；兼容旧 master 只填了单 target 字符串的情形
        let targets: Vec<zhuanfa_proto::control::TargetEndpoint> = if !f.targets.is_empty() {
            f.targets.clone()
        } else {
            #[allow(deprecated)]
            let t = f.target.trim().to_string();
            if t.is_empty() {
                Vec::new()
            } else {
                vec![zhuanfa_proto::control::TargetEndpoint { addr: t, weight: 1 }]
            }
        };
        let target_strategy = if f.target_strategy.is_empty() {
            "weighted".to_string()
        } else {
            f.target_strategy.clone()
        };
        if targets.is_empty() {
            tracing::warn!(forward_id = f.id, "forward 没有 target，跳过 entry 启动");
            continue;
        }
        let fid = f.id;
        // 协议派发：tcp / udp / tcp+udp 任意组合；空字符串当 tcp（兼容）
        let parts: Vec<&str> = f
            .protocol
            .split('+')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        let has_tcp = parts.is_empty() || parts.iter().any(|p| *p == "tcp");
        let has_udp = parts.iter().any(|p| *p == "udp");

        if has_tcp {
            if f.hops.len() == 1 {
                let (t, s) = (targets.clone(), target_strategy.clone());
                tokio::spawn(async move {
                    if let Err(e) = forward::run_single_hop(port, fid, t, s).await {
                        tracing::error!(error = %e, "tcp single-hop entry exited");
                    }
                });
            } else {
                let (hops, t, s, ctx2, lb2) = (
                    f.hops.clone(), targets.clone(), target_strategy.clone(),
                    ctx.clone(), lb.clone(),
                );
                tokio::spawn(async move {
                    if let Err(e) =
                        dataplane::run_multi_hop_entry(port, fid, hops, t, s, ctx2, lb2).await
                    {
                        tracing::error!(error = %e, "tcp multi-hop entry exited");
                    }
                });
            }
        }
        if has_udp {
            if f.hops.len() == 1 {
                let (t, s, tr) = (
                    targets.clone(), target_strategy.clone(), target_router.clone(),
                );
                tokio::spawn(async move {
                    if let Err(e) = udp_forward::run_udp_single_hop(port, fid, t, s, tr).await {
                        tracing::error!(error = %e, "udp single-hop entry exited");
                    }
                });
            } else {
                let (hops, t, s, ctx2, lb2) = (
                    f.hops.clone(), targets.clone(), target_strategy.clone(),
                    ctx.clone(), lb.clone(),
                );
                tokio::spawn(async move {
                    if let Err(e) =
                        udp_forward::run_udp_multi_hop(port, fid, hops, t, s, ctx2, lb2).await
                    {
                        tracing::error!(error = %e, "udp multi-hop entry exited");
                    }
                });
            }
        }
    }

    // 心跳循环
    let mut seq = 0u64;
    let mut tick = tokio::time::interval(Duration::from_secs(5));
    loop {
        tick.tick().await;
        seq += 1;
        if let Err(e) = client
            .heartbeat(HeartbeatRequest { node_id: node_id.clone(), seq, load: 0.0 })
            .await
        {
            tracing::warn!(seq, error = %e, "heartbeat failed");
        }
        // 周期刷新节点健康/延迟视图（驱动 LB 跳过不健康节点）
        if let Ok(r) = client
            .sync_config(SyncRequest { node_id: node_id.clone() })
            .await
        {
            *ctx.nodes.write().unwrap() = build_nodes(&r.into_inner().nodes);
        }
    }
}
