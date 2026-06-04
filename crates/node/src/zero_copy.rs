//! M7.2 单跳 fastpath：Linux `splice(2)` 零拷贝双向转发。
//!
//! 思路：socket → kernel pipe → socket，数据全程内核态搬运，不进用户态 buffer。
//! 相比 `read/write` 路径节省 2 次 memcpy + 减少 syscall 次数（pipe 1MB vs buf 64KB）。
//! 单流 TCP 实测预期：6.88 → 9.0+ Gbps（参考 realm zero_copy 实现）。
//!
//! 仅 Linux 启用；其他平台 `forward.rs` fallback 到 `copy_bidirectional`。
//! 多跳路径（dataplane.rs）走 TLS/QUIC，加密后无法 splice，不受影响。

#![cfg(target_os = "linux")]

use std::io;
use std::os::fd::{AsRawFd, RawFd};
use std::sync::Arc;
use tokio::io::Interest;
use tokio::net::TcpStream;

use crate::ratelimit::Limiter;

/// pipe 缓冲区上限。默认 64KB，扩到 1MB 减少 splice_in/out 切换频次。
/// 超出 `/proc/sys/fs/pipe-max-size`（默认 1MB）时 `F_SETPIPE_SZ` 返回 EPERM，
/// 内核仍保留默认 64KB —— 不致命，性能略降。
const PIPE_CAPACITY: i32 = 1024 * 1024;

/// 单次 splice 搬运字节上限。realm 用 isize::MAX，让 pipe 容量自然封顶 ——
/// 减少 syscall 次数（更少 async_io poll 切换）。kernel 实际只搬当前 pipe 剩余空间。
const SPLICE_CHUNK: usize = isize::MAX as usize;

/// 内核 pipe 对。drop 时关闭 fd。`pipe2(O_NONBLOCK | O_CLOEXEC)` 一次性设好 flag。
struct Pipe {
    r: RawFd,
    w: RawFd,
}

impl Pipe {
    fn new() -> io::Result<Self> {
        let mut fds = [0i32; 2];
        // SAFETY: fds 是 2 元素栈数组，pipe2 写入两个 fd。flag 是常量。
        let ret = unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_NONBLOCK | libc::O_CLOEXEC) };
        if ret != 0 {
            return Err(io::Error::last_os_error());
        }
        // best-effort 扩容；失败保留默认 64KB，不阻断。
        unsafe {
            libc::fcntl(fds[0], libc::F_SETPIPE_SZ, PIPE_CAPACITY);
        }
        Ok(Pipe { r: fds[0], w: fds[1] })
    }
}

impl Drop for Pipe {
    fn drop(&mut self) {
        unsafe {
            libc::close(self.r);
            libc::close(self.w);
        }
    }
}

#[inline]
fn splice_move(from: RawFd, to: RawFd, len: usize) -> io::Result<usize> {
    // SAFETY: from/to 是合法 fd（pipe 或 socket），off_in/off_out NULL 适用于 pipe 和 socket。
    let r = unsafe {
        libc::splice(
            from,
            std::ptr::null_mut(),
            to,
            std::ptr::null_mut(),
            len,
            (libc::SPLICE_F_MOVE | libc::SPLICE_F_NONBLOCK) as u32,
        )
    };
    if r < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(r as usize)
    }
}

/// 单方向 splice 循环：src socket → pipe → dst socket。
/// 每搬运一段调用 `on_bytes(n)` 更新计数 + `throttle` 限速。
/// 任一端 EOF 或错误 → Ok 返回，调用方据此触发整体 shutdown。
async fn one_way<F>(
    src: Arc<TcpStream>,
    dst: Arc<TcpStream>,
    rate: Option<Arc<Limiter>>,
    mut on_bytes: F,
) -> io::Result<()>
where
    F: FnMut(usize),
{
    let pipe = Pipe::new()?;
    let pw = pipe.w;
    let pr = pipe.r;
    let src_fd = src.as_raw_fd();
    let dst_fd = dst.as_raw_fd();

    // 限速器存在时才走 throttle async path —— None 时省 future state machine 开销
    let has_rate = rate.is_some();

    loop {
        // 阶段 1：socket → pipe。tokio 处理 EAGAIN/READABLE 等待。
        let n = match src
            .async_io(Interest::READABLE, || splice_move(src_fd, pw, SPLICE_CHUNK))
            .await
        {
            Ok(0) => return Ok(()), // 源端 EOF
            Ok(n) => n,
            Err(e) => return Err(e),
        };

        // 阶段 2：pipe → socket。可能一次不能全 drain，循环到 left == 0。
        // pipe 排空后下一轮 splice_in 才不会卡满。
        let mut left = n;
        while left > 0 {
            let m = match dst
                .async_io(Interest::WRITABLE, || splice_move(pr, dst_fd, left))
                .await
            {
                Ok(0) => return Ok(()), // 目标端关闭
                Ok(m) => m,
                Err(e) => return Err(e),
            };
            left -= m;
        }

        // 统计 + 限速移到 outer iter 末尾：一次 splice_in 对应一次回调，
        // 总字节量等价但减少 async_io drain loop 内的 await 切换开销。
        on_bytes(n);
        if has_rate {
            crate::ratelimit::throttle(&rate, n).await;
        }
    }
}

/// 双向 splice 转发。语义对齐 `forward.rs` 原 read/write 循环：
/// - `on_up(n)`：客户端 → target 方向每搬运 n 字节回调（bytes_in）
/// - `on_down(n)`：target → 客户端方向（bytes_out）
/// - 任一方向退出 → 整个连接结束（match 现有 select 行为）
pub async fn splice_bidirectional<U, D>(
    inbound: TcpStream,
    outbound: TcpStream,
    rate_up: Option<Arc<Limiter>>,
    rate_down: Option<Arc<Limiter>>,
    on_up: U,
    on_down: D,
) where
    U: FnMut(usize) + Send + 'static,
    D: FnMut(usize) + Send + 'static,
{
    let inbound = Arc::new(inbound);
    let outbound = Arc::new(outbound);

    let up = tokio::spawn(one_way(inbound.clone(), outbound.clone(), rate_up, on_up));
    let down = tokio::spawn(one_way(outbound, inbound, rate_down, on_down));

    tokio::select! {
        r = up => { if let Ok(Err(e)) = r { tracing::debug!(error = %e, "splice up exited"); } }
        r = down => { if let Ok(Err(e)) = r { tracing::debug!(error = %e, "splice down exited"); } }
    }
}
