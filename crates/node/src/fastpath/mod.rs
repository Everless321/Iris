//! M4 fast path：内核态 nftables DNAT 替代用户态 tokio 转发。
//! 仅适用：单跳 + 非 TLS + 节点支持 nftables。其他场景永远 slow path（现有 tokio 实现）。
//!
//! 三层兜底（任一失败 → slow path）：
//! 1. 编译期：非 Linux 整模块 stub
//! 2. 启动期：probe 检测内核 / 二进制 / CAP_NET_ADMIN
//! 3. 运行期：add_rule 失败立即 fallthrough slow path（per-forward 粒度）

pub mod probe;

#[cfg(target_os = "linux")]
mod nft_linux;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

/// 节点 fast path 能力快照。heartbeat 上报，写入 nodes.capabilities (JSON)。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FastPathCapability {
    /// 整体可用性。false → 节点永远 slow path
    pub fastpath: bool,
    /// 不支持时的具体原因（kernel-too-old / no-nft / no-cap / container / probe-error）
    pub reason: String,
    /// 内核版本字符串
    pub kernel: String,
    /// 是否在容器内（Docker / OpenVZ）
    pub in_container: bool,
}

impl FastPathCapability {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".into())
    }
}

/// 单条 fast path 规则定义（master sync_config → node spawn_forward）。
#[derive(Debug, Clone)]
pub struct FastPathRule {
    pub forward_id: i64,
    pub protocol: String, // "tcp" | "udp"
    pub listen_port: u16,
    pub target_addr: SocketAddr,
}

/// 单 forward 的内核态 counter 快照。bytes_in = PREROUTING 命中 DNAT 前的客户端→target 字节数。
/// bytes_out V2 再加（需 conntrack 反向 SNAT counter，先记 0）。
#[derive(Debug, Clone, Default)]
#[allow(dead_code)] // forward_id 在 HashMap key 引用；bytes_out 留 V2
pub struct CounterSnapshot {
    pub forward_id: i64,
    pub bytes_in: u64,
    pub bytes_out: u64,
}

/// fast path 控制接口。Linux 端走 nft shell；非 Linux 走 noop（probe 已挡，不应被调）。
#[allow(dead_code)] // cleanup/get_counters 由 M4.2-D + 优雅 shutdown 调用
pub trait FastPathManager: Send + Sync {
    /// 启动期：建 `inet iris` table + nat chain。idempotent，先 delete 再 add。
    fn init(&self) -> Result<()>;
    /// agent shutdown / probe 失败时：删 `iris` table 留干净环境。
    fn cleanup(&self) -> Result<()>;
    /// 加一条 DNAT 规则 + 匿名 counter，comment 标 forward_id 用于后续查找/删除。
    fn add_rule(&self, rule: &FastPathRule) -> Result<()>;
    /// 按 forward_id 删 prerouting 链上的规则。
    fn delete_rule(&self, forward_id: i64) -> Result<()>;
    /// 拉所有 forward 的 counter 快照（一次 nft -j list）。
    fn get_counters(&self) -> Result<HashMap<i64, CounterSnapshot>>;
    /// 当前是否真的可用（已 init 过）。
    fn is_available(&self) -> bool;
}

#[cfg(target_os = "linux")]
#[allow(dead_code)] // M4.2-C 接 spawn_forward 后启用
pub fn new_manager() -> Arc<dyn FastPathManager> {
    Arc::new(nft_linux::NftFastPath::default())
}

#[cfg(not(target_os = "linux"))]
#[allow(dead_code)]
pub fn new_manager() -> Arc<dyn FastPathManager> {
    Arc::new(NoopFastPath)
}

#[cfg(not(target_os = "linux"))]
#[allow(dead_code)]
struct NoopFastPath;

#[cfg(not(target_os = "linux"))]
impl FastPathManager for NoopFastPath {
    fn init(&self) -> Result<()> { Ok(()) }
    fn cleanup(&self) -> Result<()> { Ok(()) }
    fn add_rule(&self, _: &FastPathRule) -> Result<()> {
        anyhow::bail!("fastpath not available on this platform")
    }
    fn delete_rule(&self, _: i64) -> Result<()> { Ok(()) }
    fn get_counters(&self) -> Result<HashMap<i64, CounterSnapshot>> { Ok(HashMap::new()) }
    fn is_available(&self) -> bool { false }
}
