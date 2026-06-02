use anyhow::Result;
use clap::{Parser, Subcommand};
use serde_json::json;

#[derive(Parser)]
#[command(name = "iris-cli", about = "Iris 转发平台命令行")]
struct Cli {
    /// master HTTP API 地址
    #[arg(long, default_value = "http://127.0.0.1:7080", global = true)]
    api: String,
    /// Bearer token（可选；未传时回退到 IRIS_TOKEN 环境变量）。
    /// 通过 `iris-cli login` 获取，或从浏览器/admin 拷贝。
    #[arg(long, global = true)]
    token: Option<String>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// 登录获取 JWT token；打印到 stdout 便于 `export IRIS_TOKEN=$(iris-cli login ...)`
    Login {
        #[arg(long)]
        username: String,
        #[arg(long)]
        password: String,
    },
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

fn auth_header(req: ureq::Request, token: Option<&str>) -> ureq::Request {
    match token {
        Some(t) if !t.is_empty() => req.set("Authorization", &format!("Bearer {t}")),
        _ => req,
    }
}

fn get(url: &str, token: Option<&str>) -> Result<()> {
    let body = auth_header(ureq::get(url), token).call()?.into_string()?;
    println!("{}", pretty(&body));
    Ok(())
}

fn post(url: &str, payload: serde_json::Value, token: Option<&str>) -> Result<()> {
    let body = auth_header(ureq::post(url), token).send_json(payload)?.into_string()?;
    println!("{}", pretty(&body));
    Ok(())
}

fn delete(url: &str, token: Option<&str>) -> Result<()> {
    auth_header(ureq::delete(url), token).call()?;
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
    let token = cli.token.or_else(|| std::env::var("IRIS_TOKEN").ok());
    let tok = token.as_deref();
    match cli.cmd {
        Cmd::Login { username, password } => {
            let resp = ureq::post(&format!("{api}/api/auth/login"))
                .send_json(json!({"username": username, "password": password}))?
                .into_string()?;
            let v: serde_json::Value = serde_json::from_str(&resp)?;
            match v.get("token").and_then(|t| t.as_str()) {
                Some(t) => println!("{t}"),
                None => {
                    eprintln!("登录失败：{resp}");
                    std::process::exit(1);
                }
            }
        }
        Cmd::ListNodes => get(&format!("{api}/api/nodes"), tok)?,
        Cmd::AddNode { id, name, addr, weight } => post(
            &format!("{api}/api/nodes"),
            json!({"id": id, "name": name, "addr": addr, "weight": weight}),
            tok,
        )?,
        Cmd::DelNode { id } => delete(&format!("{api}/api/nodes/{id}"), tok)?,
        Cmd::ListForwards => get(&format!("{api}/api/forwards"), tok)?,
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
            post(&format!("{api}/api/forwards"), body, tok)?
        }
        Cmd::DelForward { id } => delete(&format!("{api}/api/forwards/{id}"), tok)?,
    }
    Ok(())
}
