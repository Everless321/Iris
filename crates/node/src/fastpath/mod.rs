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
#[cfg(not(target_os = "linux"))]
mod stub;

use serde::{Deserialize, Serialize};

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
