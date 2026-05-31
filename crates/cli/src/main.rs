use anyhow::Result;
use clap::{Parser, Subcommand};
use serde_json::json;

#[derive(Parser)]
#[command(name = "iris-cli", about = "Iris 转发平台命令行")]
struct Cli {
    /// master HTTP API 地址
    #[arg(long, default_value = "http://127.0.0.1:7080", global = true)]
    api: String,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// 列出节点
    ListNodes,
    /// 新增节点
    AddNode {
        #[arg(long)]
        id: String,
        #[arg(long)]
        name: String,
        /// 节点 DataPlane 可达地址 host:port
        #[arg(long)]
        addr: String,
        /// 节点权重（加权负载均衡）
        #[arg(long, default_value_t = 1)]
        weight: i64,
    },
    /// 删除节点
    DelNode {
        #[arg(long)]
        id: String,
    },
    /// 列出转发
    ListForwards,
    /// 新增转发。简单：--path a,b,c。负载均衡：
    /// --hops "a | b1:3,b2:1@weighted | c1,c2@source_hash"
    /// （| 分隔跳，, 分隔组内节点，id:weight 配权重，@strategy 配策略）
    AddForward {
        #[arg(long)]
        name: String,
        #[arg(long)]
        listen: i64,
        /// 简单路径，每跳单节点，如 a,b,c
        #[arg(long, value_delimiter = ',')]
        path: Option<Vec<String>>,
        /// 负载均衡 DSL（与 --path 二选一）
        #[arg(long)]
        hops: Option<String>,
        /// 最终目标 host:port
        #[arg(long)]
        target: String,
        #[arg(long, default_value = "tcp")]
        protocol: String,
    },
    /// 删除转发
    DelForward {
        #[arg(long)]
        id: i64,
    },
}

fn get(url: &str) -> Result<()> {
    let body = ureq::get(url).call()?.into_string()?;
    println!("{}", pretty(&body));
    Ok(())
}

fn post(url: &str, payload: serde_json::Value) -> Result<()> {
    let body = ureq::post(url).send_json(payload)?.into_string()?;
    println!("{}", pretty(&body));
    Ok(())
}

fn delete(url: &str) -> Result<()> {
    ureq::delete(url).call()?;
    println!("deleted");
    Ok(())
}

/// 解析 hops DSL：`a | b1:3,b2:1@weighted | c1,c2@source_hash`
fn parse_hops_dsl(s: &str) -> Result<serde_json::Value> {
    let hops: Vec<serde_json::Value> = s
        .split('|')
        .map(|hop| {
            let hop = hop.trim();
            let (nodes_part, strategy) = match hop.split_once('@') {
                Some((n, st)) => (n.trim(), st.trim()),
                None => (hop, "weighted"),
            };
            let nodes: Vec<serde_json::Value> = nodes_part
                .split(',')
                .filter_map(|n| {
                    let n = n.trim();
                    if n.is_empty() {
                        return None;
                    }
                    let (id, w) = match n.split_once(':') {
                        // 权重上界 1000，防 expand 内存爆炸
                        Some((id, w)) => (id.trim(), w.trim().parse::<u32>().unwrap_or(1).clamp(1, 1000)),
                        None => (n, 1),
                    };
                    if id.is_empty() {
                        return None;
                    }
                    Some(json!({"id": id, "weight": w}))
                })
                .collect();
            json!({"strategy": strategy, "nodes": nodes})
        })
        .collect();
    Ok(json!(hops))
}

fn pretty(s: &str) -> String {
    serde_json::from_str::<serde_json::Value>(s)
        .ok()
        .and_then(|v| serde_json::to_string_pretty(&v).ok())
        .unwrap_or_else(|| s.to_string())
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let api = cli.api.trim_end_matches('/');
    match cli.cmd {
        Cmd::ListNodes => get(&format!("{api}/api/nodes"))?,
        Cmd::AddNode { id, name, addr, weight } => post(
            &format!("{api}/api/nodes"),
            json!({"id": id, "name": name, "addr": addr, "weight": weight}),
        )?,
        Cmd::DelNode { id } => delete(&format!("{api}/api/nodes/{id}"))?,
        Cmd::ListForwards => get(&format!("{api}/api/forwards"))?,
        Cmd::AddForward { name, listen, path, hops, target, protocol } => {
            let mut body = json!({
                "name": name,
                "listen_port": listen,
                "target": target,
                "protocol": protocol,
            });
            match (hops, path) {
                (Some(dsl), _) => body["hops"] = parse_hops_dsl(&dsl)?,
                (None, Some(p)) => body["path"] = json!(p),
                (None, None) => anyhow::bail!("需提供 --path 或 --hops"),
            }
            post(&format!("{api}/api/forwards"), body)?
        }
        Cmd::DelForward { id } => delete(&format!("{api}/api/forwards/{id}"))?,
    }
    Ok(())
}
