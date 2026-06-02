//! 启动时 fast path 能力探测。失败永不影响 agent 启动。
//!
//! 检查项（全部通过才标 fastpath=true）：
//!   1. 内核版本 ≥ 5.4（nft flowtable / 较稳定 inet 表）
//!   2. nft 二进制存在（PATH 内）
//!   3. CAP_NET_ADMIN 持有（root 或 systemd AmbientCapabilities）
//!   4. 非容器（Docker bridge / OpenVZ / LXC 不可信）— host 网络 Docker 通过
//!   5. dry-run 写测试：建 `iris-probe` table → 删 — 真验 netlink 可写

use super::FastPathCapability;
use std::process::Command;

const MIN_KERNEL_MAJOR: u32 = 5;
const MIN_KERNEL_MINOR: u32 = 4;

/// 同步阻塞探测。在 agent boot 早期调用一次。
pub fn detect() -> FastPathCapability {
    let mut cap = FastPathCapability::default();
    cap.kernel = read_kernel_version();

    #[cfg(not(target_os = "linux"))]
    {
        cap.fastpath = false;
        cap.reason = "non-linux".into();
    }

    #[cfg(target_os = "linux")]
    {
        if !check_kernel_version(&cap.kernel) {
            cap.reason = format!(
                "kernel-too-old (need ≥ {}.{}, got {})",
                MIN_KERNEL_MAJOR, MIN_KERNEL_MINOR, cap.kernel
            );
            return cap;
        }
        if !check_nft_binary() {
            cap.reason = "nft-binary-missing".into();
            return cap;
        }
        cap.in_container = detect_container();
        // 容器内 + bridge 网络通常 nftables 受限；保守降级。host network 用户已经看不到 cgroup proc/1 marker，自然 fall through。
        if cap.in_container {
            cap.reason = "container-network-untrusted".into();
            return cap;
        }
        if !check_cap_net_admin() {
            cap.reason = "missing-CAP_NET_ADMIN".into();
            return cap;
        }
        if let Err(e) = dry_run_nft_table() {
            cap.reason = format!("dry-run-failed: {e}");
            return cap;
        }
        cap.fastpath = true;
        cap.reason = "ok".into();
    }
    cap
}

fn read_kernel_version() -> String {
    Command::new("uname")
        .arg("-r")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn check_kernel_version(s: &str) -> bool {
    // 形如 "6.1.0-23-amd64" 或 "5.15.0-1058-gcp"
    let mut parts = s.split(|c: char| !c.is_ascii_digit()).filter(|p| !p.is_empty());
    let major: u32 = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
    let minor: u32 = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
    if major > MIN_KERNEL_MAJOR {
        return true;
    }
    major == MIN_KERNEL_MAJOR && minor >= MIN_KERNEL_MINOR
}

#[cfg(target_os = "linux")]
fn check_nft_binary() -> bool {
    Command::new("nft")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(target_os = "linux")]
fn check_cap_net_admin() -> bool {
    // 简单办法：试 `nft list tables` — 无权限会 EACCES / EPERM；有权限即使无表也输出空行 + ok。
    Command::new("nft")
        .args(["list", "tables"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(target_os = "linux")]
fn detect_container() -> bool {
    // /.dockerenv 或 cgroup 含 docker/lxc/containerd — 任一命中视作容器
    if std::path::Path::new("/.dockerenv").exists() {
        return true;
    }
    match std::fs::read_to_string("/proc/1/cgroup") {
        Ok(s) => s.contains("docker") || s.contains("lxc") || s.contains("containerd"),
        Err(_) => false,
    }
}

#[cfg(target_os = "linux")]
fn dry_run_nft_table() -> Result<(), String> {
    // 建 + 删 `iris-probe` table 验真写权限
    let create = Command::new("nft")
        .args(["add", "table", "inet", "iris-probe"])
        .output()
        .map_err(|e| format!("spawn nft: {e}"))?;
    if !create.status.success() {
        return Err(format!(
            "create probe table: {}",
            String::from_utf8_lossy(&create.stderr).trim()
        ));
    }
    let delete = Command::new("nft")
        .args(["delete", "table", "inet", "iris-probe"])
        .output()
        .map_err(|e| format!("spawn nft: {e}"))?;
    if !delete.status.success() {
        // 创建成功但删除失败 — 留一行残留，下次启动会被 cleanup 接走。不阻塞 fast path
        tracing::warn!(stderr = %String::from_utf8_lossy(&delete.stderr), "probe cleanup");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kernel_parse() {
        assert!(check_kernel_version("6.1.0-23-amd64"));
        assert!(check_kernel_version("5.15.0-1058-gcp"));
        assert!(check_kernel_version("5.4.0"));
        assert!(!check_kernel_version("4.19.0"));
        assert!(!check_kernel_version("3.10.0"));
        assert!(!check_kernel_version(""));
    }
}
