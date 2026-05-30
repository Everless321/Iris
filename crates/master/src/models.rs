use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct Node {
    pub id: String,
    pub name: String,
    pub addr: String,
    pub status: String,
    pub last_seen: Option<i64>,
    pub created_at: i64,
    #[serde(default = "one")]
    pub weight: i64,
    #[serde(default)]
    pub health: String,
    #[serde(default)]
    pub latency_ms: Option<i64>,
    #[serde(default)]
    pub fail_count: i64,
    #[serde(default)]
    pub probe_total: i64,
    #[serde(default)]
    pub probe_ok: i64,
    #[serde(default)]
    pub fail_events: i64,
    #[serde(default)]
    pub down_since: Option<i64>,
    #[serde(default)]
    pub downtime_ms: i64,
}

#[derive(Debug, Deserialize)]
pub struct NodeCreate {
    pub id: String,
    pub name: String,
    pub addr: String,
    #[serde(default = "one")]
    pub weight: i64,
}

fn one() -> i64 {
    1
}
fn one_u32() -> u32 {
    1
}
fn default_strategy() -> String {
    "weighted".into()
}

// ---- hops 结构 ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HopNode {
    pub id: String,
    #[serde(default = "one_u32")]
    pub weight: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hop {
    #[serde(default = "default_strategy")]
    pub strategy: String,
    pub nodes: Vec<HopNode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetEndpoint {
    pub addr: String,
    #[serde(default = "one_u32")]
    pub weight: u32,
}

// 数据库行：hops 以 JSON 文本存储在 path 列。
// owner_id 由 0004 迁移引入；0=未归属（兼容历史数据），>0=用户 id
#[derive(Debug, FromRow)]
pub struct ForwardRow {
    pub id: i64,
    pub name: String,
    pub listen_port: i64,
    pub protocol: String,
    pub path: String,   // JSON of Vec<Hop>
    pub target: String, // JSON of Vec<TargetEndpoint>（migration 0007 之后）
    pub enabled: i64,
    pub created_at: i64,
    #[sqlx(default)]
    pub owner_id: i64,
    #[sqlx(default)]
    pub target_strategy: String,
}

impl ForwardRow {
    pub fn hops(&self) -> Vec<Hop> {
        match serde_json::from_str(&self.path) {
            Ok(h) => h,
            Err(e) => {
                tracing::warn!(
                    forward_id = self.id,
                    error = %e,
                    raw = %self.path,
                    "forward hops JSON 解析失败，该规则将被忽略"
                );
                Vec::new()
            }
        }
    }

    pub fn targets(&self) -> Vec<TargetEndpoint> {
        let t = self.target.trim();
        if t.starts_with('[') {
            serde_json::from_str(t).unwrap_or_else(|e| {
                tracing::warn!(
                    forward_id = self.id,
                    error = %e,
                    raw = %self.target,
                    "forward targets JSON 解析失败，回退空"
                );
                Vec::new()
            })
        } else if !t.is_empty() {
            // 兜底：未跑迁移的兼容情形
            vec![TargetEndpoint { addr: t.into(), weight: 1 }]
        } else {
            Vec::new()
        }
    }
}

// 对外 DTO
#[derive(Debug, Serialize)]
pub struct Forward {
    pub id: i64,
    pub name: String,
    pub listen_port: i64,
    pub protocol: String,
    pub hops: Vec<Hop>,
    pub targets: Vec<TargetEndpoint>,
    pub target_strategy: String,
    pub enabled: bool,
    pub created_at: i64,
    pub owner_id: i64,
}

impl From<ForwardRow> for Forward {
    fn from(r: ForwardRow) -> Self {
        let hops = r.hops();
        let targets = r.targets();
        let target_strategy = if r.target_strategy.is_empty() {
            "weighted".into()
        } else {
            r.target_strategy
        };
        Forward {
            id: r.id,
            name: r.name,
            listen_port: r.listen_port,
            protocol: r.protocol,
            hops,
            targets,
            target_strategy,
            enabled: r.enabled != 0,
            created_at: r.created_at,
            owner_id: r.owner_id,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct ForwardCreate {
    pub name: String,
    pub listen_port: i64,
    #[serde(default = "tcp")]
    pub protocol: String,
    // 新格式：节点组序列
    #[serde(default)]
    pub hops: Option<Vec<Hop>>,
    // 旧格式：单节点序列（向后兼容），自动升级为单节点组
    #[serde(default)]
    pub path: Option<Vec<String>>,
    // 旧格式：单字符串目标
    #[serde(default)]
    pub target: Option<String>,
    // 新格式：多目标 + 策略
    #[serde(default)]
    pub targets: Option<Vec<TargetEndpoint>>,
    #[serde(default = "default_strategy")]
    pub target_strategy: String,
}

impl ForwardCreate {
    /// 归一化为 hops，兼容旧 path 输入。
    pub fn normalized_hops(&self) -> Vec<Hop> {
        if let Some(h) = &self.hops {
            return h.clone();
        }
        self.path
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .map(|id| Hop {
                strategy: "weighted".into(),
                nodes: vec![HopNode { id: id.clone(), weight: 1 }],
            })
            .collect()
    }

    /// 归一化目标列表：新字段优先，否则把旧单字符串包成单项数组。
    pub fn normalized_targets(&self) -> Vec<TargetEndpoint> {
        if let Some(ts) = &self.targets {
            return ts.clone();
        }
        match self.target.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            Some(t) => vec![TargetEndpoint { addr: t.into(), weight: 1 }],
            None => Vec::new(),
        }
    }
}

fn tcp() -> String {
    "tcp".into()
}

/// 归一化协议串：接受 tcp / udp / tcp+udp / both / tcp,udp / udp+tcp（大小写无关）。
/// 返回规范形式 "tcp" / "udp" / "tcp+udp"，非法值返回 None。
pub fn parse_protocol(raw: &str) -> Option<String> {
    let s = raw.trim().to_ascii_lowercase();
    let has = |k: &str| s.split(|c: char| c == '+' || c == ',' || c == '/').any(|p| p.trim() == k);
    let tcp = has("tcp") || s == "both";
    let udp = has("udp") || s == "both";
    match (tcp, udp) {
        (true, true) => Some("tcp+udp".into()),
        (true, false) => Some("tcp".into()),
        (false, true) => Some("udp".into()),
        _ => None,
    }
}

// ---- 用户 / 邀请码 / 鉴权 ----

#[derive(Debug, Serialize, FromRow)]
pub struct UserRow {
    pub id: i64,
    pub username: String,
    pub password_hash: String,
    pub role: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Serialize)]
pub struct UserDto {
    pub id: i64,
    pub username: String,
    pub role: String,
    pub created_at: i64,
}

impl From<UserRow> for UserDto {
    fn from(u: UserRow) -> Self {
        UserDto {
            id: u.id,
            username: u.username,
            role: u.role,
            created_at: u.created_at,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    pub username: String,
    pub password: String,
    pub invite_code: String,
}

#[derive(Debug, Serialize)]
pub struct AuthResponse {
    pub token: String,
    pub user: UserDto,
}

#[derive(Debug, Serialize, FromRow)]
pub struct EnrollmentToken {
    pub token: String,
    pub node_id: String,
    pub expires_at: i64,
    pub used_at: Option<i64>,
    pub created_at: i64,
}

#[derive(Debug, Deserialize)]
pub struct EnrollRequest {
    pub token: String,
}

#[derive(Debug, Serialize)]
pub struct EnrollResponse {
    pub node_id: String,
    pub ca_pem: String,
    pub cert_pem: String,
    pub key_pem: String,
    pub master_grpc: String,   // 节点该连的 master 控制面地址 host:port
    pub data_addr_hint: String, // 推荐节点的 ZF_DATA_ADDR
}

#[derive(Debug, Serialize, FromRow)]
pub struct InviteCode {
    pub code: String,
    pub created_by: i64,
    pub used_by: Option<i64>,
    pub used_at: Option<i64>,
    pub created_at: i64,
}

