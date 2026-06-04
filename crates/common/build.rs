// 编译时注入 git short hash + 编译时间戳，供运行时上报版本。
// git 不可用 / shallow clone 时 fallback "unknown"。
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/refs");

    let git_hash = Command::new("git")
        .args(["rev-parse", "--short=8", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=IRIS_GIT_HASH={git_hash}");

    // 编译时 UTC 时间戳（秒）。release CI 重复编译会刷新这个 — 用于"同 hash 不同构建"区分。
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    println!("cargo:rustc-env=IRIS_BUILD_TS={ts}");
}
