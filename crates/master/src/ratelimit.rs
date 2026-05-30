use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// 简单内存版滑动窗口限速。P4 阶段 10-50 客户够用；规模上来再换 Redis。
pub struct RateLimiter {
    window: Duration,
    max_hits: u32,
    state: Mutex<HashMap<String, Vec<Instant>>>,
}

impl RateLimiter {
    pub fn new(window: Duration, max_hits: u32) -> Self {
        Self {
            window,
            max_hits,
            state: Mutex::new(HashMap::new()),
        }
    }

    /// 返回 true 表示允许；false 表示已超限。
    pub fn check(&self, key: &str) -> bool {
        let mut g = self.state.lock().unwrap();
        let now = Instant::now();
        let cutoff = now - self.window;
        let v = g.entry(key.to_string()).or_default();
        v.retain(|t| *t >= cutoff);
        if v.len() as u32 >= self.max_hits {
            return false;
        }
        v.push(now);
        // 顺便淘汰长期不活跃 key（避免无界增长）
        if g.len() > 2048 {
            g.retain(|_, v| v.last().map(|t| *t >= cutoff).unwrap_or(false));
        }
        true
    }
}
