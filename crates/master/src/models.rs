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
    /// migration 0009：节点 mTLS client cert NotAfter (unix ms)。0 = 未上报 / 老节点。
    /// UI 用此显示到期倒计时；剩 ≤ 30 天节点自动触发 RenewCert。
    #[sqlx(default)]
    #[serde(default)]
    pub cert_not_after_ms: i64,
    /// migration 0013 M4.2-A：fast path 能力 JSON。空 / NULL = 老节点未上报。
    /// 例：{"fastpath":true,"kernel":"6.1.0","reason":"ok","in_container":false}
    #[sqlx(default)]
    #[serde(default)]
    pub capabilities: String,
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
    /// migration 0008：累计上行（客户端→入口）字节数。i64 由 SQLite 限制，u64 上限够用。
    #[sqlx(default)]
    pub bytes_in: i64,
    #[sqlx(default)]
    pub bytes_out: i64,
    // migration 0011 #39 流量限制：NULL 表示该机制不启用
    #[sqlx(default)]
    pub quota_in_bytes: Option<i64>,
    #[sqlx(default)]
    pub quota_out_bytes: Option<i64>,
    #[sqlx(default)]
    pub rate_in_bps: Option<i64>,
    #[sqlx(default)]
    pub rate_out_bps: Option<i64>,
    #[sqlx(default)]
    pub quota_reset: Option<String>,           // 'none' | 'daily' | 'monthly'
    #[sqlx(default)]
    pub quota_reset_at_ms: Option<i64>,
    #[sqlx(default)]
    pub quota_exhausted_at_ms: Option<i64>,
    /// migration 0012 #27 节点间链路加密。'tls' (默认) | 'plain'
    #[sqlx(default)]
    pub link_encryption: String,
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
    /// 每个入口节点的 listener 实际状态。空 = 还未收到 heartbeat（master 重启后第一轮）
    /// 或者该 forward 不是入口（已被 master sync_config 过滤）
    #[serde(default)]
    pub listener_status: Vec<ListenerNodeStatus>,
    /// 累计流量（自 forward 创建以来 / 上次 quota reset 以来）。bytes_in = 上行，bytes_out = 下行。
    #[serde(default)]
    pub bytes_in: i64,
    #[serde(default)]
    pub bytes_out: i64,
    // #39 流量限制 - UI 展示 + 编辑
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota_in_bytes: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota_out_bytes: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_in_bps: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_out_bps: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota_reset: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota_reset_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota_exhausted_at_ms: Option<i64>,
    /// #27 节点间链路加密。'tls' (默认) | 'plain'
    #[serde(default)]
    pub link_encryption: String,
}

/// 单个入口节点的 listener 状态。前端 forward list 显示徽章 + tooltip。
#[derive(Debug, Serialize, Clone)]
pub struct ListenerNodeStatus {
    pub node_id: String,
    pub ok: bool,
    pub error: String,
    pub updated_at: i64,
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
            listener_status: Vec::new(),
            bytes_in: r.bytes_in,
            bytes_out: r.bytes_out,
            quota_in_bytes: r.quota_in_bytes,
            quota_out_bytes: r.quota_out_bytes,
            rate_in_bps: r.rate_in_bps,
            rate_out_bps: r.rate_out_bps,
            quota_reset: r.quota_reset,
            quota_reset_at_ms: r.quota_reset_at_ms,
            quota_exhausted_at_ms: r.quota_exhausted_at_ms,
            link_encryption: if r.link_encryption.is_empty() {
                "tls".into()
            } else {
                r.link_encryption
            },
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
    // #39 流量限制（创建 + 编辑共用）；None / 0 视为"不启用该方向"，全部 NULL 等价无限制
    #[serde(default)]
    pub quota_in_bytes: Option<i64>,
    #[serde(default)]
    pub quota_out_bytes: Option<i64>,
    #[serde(default)]
    pub rate_in_bps: Option<i64>,
    #[serde(default)]
    pub rate_out_bps: Option<i64>,
    #[serde(default)]
    pub quota_reset: Option<String>, // 'none' | 'daily' | 'monthly'
    /// #27 节点间链路加密。'tls' | 'plain'。admin-only。
    #[serde(default)]
    pub link_encryption: Option<String>,
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
    pub data_addr_hint: String, // 推荐节点的 IRIS_DATA_ADDR
}

#[derive(Debug, Serialize, FromRow)]
pub struct InviteCode {
    pub code: String,
    pub created_by: i64,
    pub used_by: Option<i64>,
    pub used_at: Option<i64>,
    pub created_at: i64,
}

