// 编译时注入 git short hash —— 供运行时上报"代码版本"。
//
// 关键：使用「最后改 crates/{node,common,proto} 的 commit」hash,
// 而非 HEAD。这样 UI/master-only 改动不会刷新节点版本字符串,
// 避免 VersionBadge 误判节点 outdated。
//
// git 不可用 / shallow clone（fetch-depth=1）→ 退化到 HEAD short hash。

use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/refs");
    // CI / 外部预先算好的 hash 通过此 env 注入 → 避免 build.rs 被 cargo 增量缓存
    // 跳过造成 hash 卡死。本地构建若未设此 env，自己 git log 算。
    println!("cargo:rerun-if-env-changed=IRIS_GIT_HASH");

    let git_hash = std::env::var("IRIS_GIT_HASH")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| relevant_code_hash().unwrap_or_else(|| "unknown".into()));
    println!("cargo:rustc-env=IRIS_GIT_HASH={git_hash}");

    // 编译时间戳，区分同 hash 不同构建。
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    println!("cargo:rustc-env=IRIS_BUILD_TS={ts}");
}

/// 优先：最后改 node/common/proto 的 commit short hash。
/// 兜底：HEAD short hash（shallow clone 时 git log -- path 会返回空）。
fn relevant_code_hash() -> Option<String> {
    let from_paths = git_short(&[
        "log",
        "-1",
        "--format=%h",
        "--abbrev=8",
        "--",
        "crates/node",
        "crates/common",
        "crates/proto",
    ]);
    if let Some(h) = from_paths.as_ref() {
        if !h.is_empty() {
            return Some(h.clone());
        }
    }
    // shallow clone 时上面会返回空（git log -- path 无历史可遍历）
    git_short(&["rev-parse", "--short=8", "HEAD"])
}

fn git_short(args: &[&str]) -> Option<String> {
    Command::new("git")
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
}
