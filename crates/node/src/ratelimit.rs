//! #39 速率限制：per-forward token bucket。
//!
//! 一个 forward 创建两个独立 bucket：
//! - `up`：客户端→target 方向（节点视角 `bytes_in`，UI 的"上传"）
//! - `down`：target→客户端方向（节点视角 `bytes_out`，UI 的"下载"）
//!
//! 1 cell = 1 byte。`Quota::per_second(N)` 表示每秒补充 N 个 byte。governor 内部
//! lock-free token bucket（GCRA），read/write 量级千次/s 量级开销可忽略。
//!
//! `rate=0` 表示不限速（`Option::None`）；上限 `u32::MAX-1 ≈ 4 GB/s`，超出 cap 处理。
//! 每个 listener task 单独 clone `Arc<Limiter>`；多个 spawned per-connection task 共享同一桶。

use governor::{DefaultDirectRateLimiter, Quota, RateLimiter};
use std::num::NonZeroU32;
use std::sync::Arc;

pub type Limiter = DefaultDirectRateLimiter;

/// per-forward 速率限制配置。两个方向独立，均可缺省（None = 不限）。
#[derive(Default, Clone)]
pub struct RateLimit {
    pub up: Option<Arc<Limiter>>,
    pub down: Option<Arc<Limiter>>,
}

impl RateLimit {
    /// `in_bps` / `out_bps` 单位 bytes/sec。0 表示该方向不限速。
    pub fn new(in_bps: u64, out_bps: u64) -> Self {
        Self {
            up: make_limiter(in_bps),
            down: make_limiter(out_bps),
        }
    }
}

fn make_limiter(bps: u64) -> Option<Arc<Limiter>> {
    if bps == 0 {
        return None;
    }
    // governor 用 u32 cells per period；u32::MAX-1 ≈ 4.29 GB/s 足够。
    let bps = bps.min(u32::MAX as u64 - 1) as u32;
    let nz = NonZeroU32::new(bps)?;
    let quota = Quota::per_second(nz)
        // burst = 1 秒额度（默认）；防长时间空闲后突发把 N 秒数据一次释放完。
        .allow_burst(nz);
    Some(Arc::new(RateLimiter::direct(quota)))
}

/// 等待 `n` 字节配额放行。`lim=None` 或 `n=0` 立即返回。
/// 单次 `n` 超过 burst 上限（u32::MAX-1）会 cap 到上限分多次（governor `until_n_ready`
/// 内部已处理大 n 拆分）。读写每次 buf <= 64KB 远小于上限，无需 caller 关心。
#[inline]
pub async fn throttle(lim: &Option<Arc<Limiter>>, n: usize) {
    let Some(l) = lim else { return };
    if n == 0 {
        return;
    }
    let n_u32 = n.min(u32::MAX as usize - 1) as u32;
    if let Some(nz) = NonZeroU32::new(n_u32) {
        // 错误情形：requested > max_capacity → governor 返回 Err，跳过限速（避免死锁）
        let _ = l.until_n_ready(nz).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_bps_means_no_limit() {
        let r = RateLimit::new(0, 0);
        assert!(r.up.is_none());
        assert!(r.down.is_none());
    }

    #[test]
    fn nonzero_bps_creates_limiter() {
        let r = RateLimit::new(1024, 0);
        assert!(r.up.is_some());
        assert!(r.down.is_none());
    }

    #[test]
    fn overflow_bps_caps_at_u32_max() {
        // 不应 panic
        let r = RateLimit::new(u64::MAX, u64::MAX);
        assert!(r.up.is_some());
        assert!(r.down.is_some());
    }

    #[tokio::test]
    async fn throttle_none_is_noop() {
        // 不阻塞
        throttle(&None, 1024).await;
    }

    #[tokio::test]
    async fn throttle_zero_n_is_noop() {
        let r = RateLimit::new(1024, 0);
        // n=0 应立即返回
        throttle(&r.up, 0).await;
    }
}
