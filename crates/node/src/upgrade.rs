//! M8 agent 远程升级 —— command stream client + 7 阶段安全升级。
//!
//! ## 流程
//! 1. 收到 UpgradeCommand → ACK status=RUNNING(preflight)
//! 2. preflight: 磁盘 / cert / 升级中标记 / sysctl 可写
//! 3. 写 .upgrade-pending 标记（watchdog 会读）
//! 4. 下载新 binary → /opt/iris/iris-node.new
//! 5. sha256 校验（expected_sha256 非空才校验）
//! 6. 子进程 dry-run 新 binary
//! 7. 原子 swap：iris-node → iris-node.bak.<ts>；iris-node.new → iris-node
//! 8. ACK SUCCESS → exit 0 → systemd restart → 新进程启动 → watchdog 60s 内确认健康
//!
//! 任一步失败 → ACK FAILED + 删 .upgrade-pending + 不动旧 binary。

use std::path::Path;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use iris_proto::control::control_client::ControlClient;
use iris_proto::control::{
    command::Kind as CommandKind, Command, CommandAck, CommandStatus, CommandsRequest,
    UpgradeCommand,
};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use tonic::transport::Channel;
use tonic::Request;

/// install 目录（systemd unit 里固定）。
const INSTALL_DIR: &str = "/opt/iris";
const BINARY_PATH: &str = "/opt/iris/iris-node";
const NEW_PATH: &str = "/opt/iris/iris-node.new";
const PENDING_MARK: &str = "/opt/iris/.upgrade-pending";

/// 升级前磁盘最小剩余字节（100 MB）—— binary ~10MB + 备份 + 临时文件，留余量。
const MIN_FREE_DISK: u64 = 100 * 1024 * 1024;

/// cert 临近过期阈值：升级期间若 < 7 天，可能续签来不及，先拒。
const MIN_CERT_REMAINING_MS: i64 = 7 * 24 * 3600 * 1000;

/// 维持 Commands stream + 处理升级。断流自动重连（指数退避，cap 60s）。
pub async fn run_command_stream(
    client: ControlClient<Channel>,
    node_id: String,
    cert_not_after_ms: std::sync::Arc<std::sync::atomic::AtomicI64>,
) {
    let mut backoff = Duration::from_secs(2);
    loop {
        match stream_once(&client, &node_id, &cert_not_after_ms).await {
            Ok(()) => {
                tracing::warn!("command stream closed by server, reconnecting in 2s");
                backoff = Duration::from_secs(2);
            }
            Err(e) => {
                tracing::warn!(error = %e, "command stream error, retry in {:?}", backoff);
            }
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(Duration::from_secs(60));
    }
}

async fn stream_once(
    client: &ControlClient<Channel>,
    node_id: &str,
    cert_not_after_ms: &std::sync::Arc<std::sync::atomic::AtomicI64>,
) -> Result<()> {
    let mut client = client.clone();
    let mut stream = client
        .commands(Request::new(CommandsRequest { node_id: node_id.to_string() }))
        .await
        .context("open Commands stream")?
        .into_inner();
    tracing::info!("command stream opened");
    while let Some(msg) = stream.message().await? {
        let req_id = msg.request_id.clone();
        match msg.kind {
            Some(CommandKind::Upgrade(u)) => {
                // 首个 ack：节点已收到
                let _ = ack(
                    &mut client,
                    node_id,
                    &req_id,
                    CommandStatus::Received,
                    "received",
                    "",
                )
                .await;
                match handle_upgrade(&mut client, node_id, &req_id, &u, cert_not_after_ms).await {
                    Ok(_) => {
                        // 不会回到这里 —— handle_upgrade 内部 exit(0)
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "upgrade failed");
                        let _ = ack(
                            &mut client,
                            node_id,
                            &req_id,
                            CommandStatus::Failed,
                            "error",
                            &e.to_string(),
                        )
                        .await;
                        // 失败保留 .bak，但要清掉 .upgrade-pending 避免下次启动误判
                        let _ = std::fs::remove_file(PENDING_MARK);
                        let _ = std::fs::remove_file(NEW_PATH);
                    }
                }
            }
            None => {
                tracing::warn!(req = %req_id, "command with empty kind, ignored");
            }
        }
    }
    Ok(())
}

async fn handle_upgrade(
    client: &mut ControlClient<Channel>,
    node_id: &str,
    req_id: &str,
    u: &UpgradeCommand,
    cert_not_after_ms: &std::sync::Arc<std::sync::atomic::AtomicI64>,
) -> Result<()> {
    // [1] preflight
    ack(client, node_id, req_id, CommandStatus::Running, "preflight", "").await.ok();
    preflight(cert_not_after_ms)?;

    // [2] 写 pending 标记 → watchdog 60s 后会查 .heartbeat-state
    std::fs::write(PENDING_MARK, format!("req={req_id}\n")).context("write pending mark")?;

    // [3] 下载
    ack(client, node_id, req_id, CommandStatus::Running, "download", "").await.ok();
    let url = build_url(&u.target_ref);
    download(&url, NEW_PATH).await.with_context(|| format!("download {url}"))?;

    // [4] sha256 校验
    ack(client, node_id, req_id, CommandStatus::Running, "verify", "").await.ok();
    if !u.expected_sha256.is_empty() {
        let got = sha256_file(NEW_PATH).context("sha256 new binary")?;
        if !got.eq_ignore_ascii_case(&u.expected_sha256) {
            return Err(anyhow!(
                "sha256 mismatch: expected {} got {}",
                u.expected_sha256,
                got
            ));
        }
    }

    // [5] dry-run 新 binary
    ack(client, node_id, req_id, CommandStatus::Running, "dry-run", "").await.ok();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(NEW_PATH, std::fs::Permissions::from_mode(0o755))
        .context("chmod new binary")?;
    dry_run(NEW_PATH).context("new binary --version failed")?;

    // [6] 原子 swap
    ack(client, node_id, req_id, CommandStatus::Running, "swap", "").await.ok();
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let bak = format!("{INSTALL_DIR}/iris-node.bak.{ts}");
    std::fs::rename(BINARY_PATH, &bak).context("rename old → bak")?;
    if let Err(e) = std::fs::rename(NEW_PATH, BINARY_PATH) {
        // 回滚：把 bak 还原
        let _ = std::fs::rename(&bak, BINARY_PATH);
        return Err(anyhow!("rename new → iris-node failed: {e}"));
    }

    // [7] ACK SUCCESS → 进程退出 → systemd 重启 → watchdog 验证
    ack(
        client,
        node_id,
        req_id,
        CommandStatus::Success,
        "restart",
        &format!("backup={bak}"),
    )
    .await
    .ok();
    tracing::warn!(req = %req_id, %bak, "upgrade swap done, exiting for systemd restart");
    // 给 ack RPC 一点时间发出
    tokio::time::sleep(Duration::from_millis(500)).await;
    std::process::exit(0);
}

fn preflight(cert_not_after_ms: &std::sync::Arc<std::sync::atomic::AtomicI64>) -> Result<()> {
    use std::sync::atomic::Ordering;
    // 1. 已在升级中
    if Path::new(PENDING_MARK).exists() {
        return Err(anyhow!("upgrade already in progress (.upgrade-pending exists)"));
    }
    // 2. 磁盘空间
    let free = disk_free_bytes(INSTALL_DIR).unwrap_or(0);
    if free < MIN_FREE_DISK {
        return Err(anyhow!("insufficient disk: {} < {}", free, MIN_FREE_DISK));
    }
    // 3. cert 剩余有效期
    let not_after = cert_not_after_ms.load(Ordering::Relaxed);
    if not_after > 0 {
        let remaining = not_after - iris_common::now_ms();
        if remaining < MIN_CERT_REMAINING_MS {
            return Err(anyhow!(
                "cert too close to expiry: {} ms remaining < {} ms required",
                remaining,
                MIN_CERT_REMAINING_MS
            ));
        }
    }
    Ok(())
}

/// statvfs 取剩余字节。失败返回 None。
fn disk_free_bytes(path: &str) -> Option<u64> {
    use std::ffi::CString;
    let cpath = CString::new(path).ok()?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    let r = unsafe { libc::statvfs(cpath.as_ptr(), &mut st) };
    if r != 0 {
        return None;
    }
    Some((st.f_bavail as u64) * (st.f_frsize as u64))
}

fn build_url(target_ref: &str) -> String {
    if target_ref.is_empty() {
        "https://github.com/Everless321/Iris/releases/download/rolling/iris-node-musl-x86_64".into()
    } else {
        format!(
            "https://github.com/Everless321/Iris/releases/download/{}/iris-node-musl-x86_64",
            target_ref
        )
    }
}

async fn download(url: &str, dest: &str) -> Result<()> {
    let _ = std::fs::remove_file(dest);
    // 用 curl —— 跟 install.sh 一致，不引新依赖。
    let status = tokio::process::Command::new("curl")
        .args(["-fsSL", "-o", dest, url])
        .status()
        .await?;
    if !status.success() {
        return Err(anyhow!("curl exit {}", status));
    }
    Ok(())
}

fn sha256_file(path: &str) -> Result<String> {
    use std::io::Read;
    let mut f = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn dry_run(path: &str) -> Result<()> {
    let out = std::process::Command::new(path).arg("--version").output()?;
    if !out.status.success() {
        return Err(anyhow!(
            "exit={} stderr={}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(())
}

async fn ack(
    client: &mut ControlClient<Channel>,
    node_id: &str,
    request_id: &str,
    status: CommandStatus,
    stage: &str,
    detail: &str,
) -> Result<()> {
    client
        .ack_command(CommandAck {
            node_id: node_id.to_string(),
            request_id: request_id.to_string(),
            status: status as i32,
            stage: stage.to_string(),
            detail: detail.to_string(),
        })
        .await?;
    Ok(())
}

