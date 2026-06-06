//! M9.2 邻居延迟探测器
//!
//! 周期对 `NodeCtx.nodes` 里所有非自己的节点 TCP connect 测 RTT，本地维护 EWMA
//! 平滑值（α=0.5），心跳时上报 `neighbor_rtt_ms` 给 master 聚合矩阵。
//!
//! 设计要点：
//! - 不写 RPC，直接 TCP connect 对端 data 端口（与 control 共用 mTLS 端，握手前
//!   就能拿到 connect RTT；不进 TLS 避免握手抖动放大）。
//! - 连续 `FAIL_THRESHOLD` 次失败才把样本从 snapshot 里移除，避免单次抖动翻盘。
//! - 单次探测自带 timeout，全轮 join 防慢节点拖死后续轮次。

use std::collections::HashMap;
use std::net::ToSocketAddrs;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use tokio::net::TcpStream;

const ALPHA_NUM: u32 = 1;
const ALPHA_DEN: u32 = 2; // α = 0.5
const FAIL_THRESHOLD: u32 = 3;

#[derive(Clone, Copy, Debug, Default)]
struct Stat {
    ewma_ms: f64,
    fail_count: u32,
    has_sample: bool,
}

impl Stat {
    fn on_ok(&mut self, sample_ms: u32) {
        let s = sample_ms as f64;
        self.ewma_ms = if self.has_sample {
            (ALPHA_NUM as f64 * s + (ALPHA_DEN - ALPHA_NUM) as f64 * self.ewma_ms)
                / ALPHA_DEN as f64
        } else {
            s
        };
        self.has_sample = true;
        self.fail_count = 0;
    }

    fn on_fail(&mut self) {
        self.fail_count = self.fail_count.saturating_add(1);
        if self.fail_count >= FAIL_THRESHOLD {
            self.has_sample = false;
        }
    }
}

/// 邻居延迟视图：心跳时 `snapshot()` 拿当前 EWMA 行上报。
pub struct NeighborProbe {
    stats: RwLock<HashMap<String, Stat>>,
}

impl NeighborProbe {
    pub fn new() -> Self {
        Self { stats: RwLock::new(HashMap::new()) }
    }

    /// 当前 EWMA 平滑后的 (邻居 → rtt_ms)。仅返回有有效样本的项。
    pub fn snapshot(&self) -> HashMap<String, u32> {
        let g = self.stats.read().unwrap();
        g.iter()
            .filter(|(_, s)| s.has_sample)
            .map(|(k, s)| (k.clone(), s.ewma_ms.round().max(0.0).min(u32::MAX as f64) as u32))
            .collect()
    }

    fn record_ok(&self, peer: &str, rtt_ms: u32) {
        self.stats.write().unwrap().entry(peer.to_string()).or_default().on_ok(rtt_ms);
    }

    fn record_fail(&self, peer: &str) {
        self.stats.write().unwrap().entry(peer.to_string()).or_default().on_fail();
    }

    /// 清掉已离线节点的 stat（节点列表不再包含时）。
    fn gc_unknown(&self, known: &[(String, String)]) {
        let known_ids: std::collections::HashSet<&String> =
            known.iter().map(|(id, _)| id).collect();
        self.stats.write().unwrap().retain(|k, _| known_ids.contains(k));
    }
}

impl Default for NeighborProbe {
    fn default() -> Self {
        Self::new()
    }
}

/// 启动周期探测后台任务。`list_neighbors` 每轮返回 `(node_id, addr)`，
/// `self_id` 用于排除自探。返回值是 `Arc<NeighborProbe>`，心跳处用它 snapshot。
pub fn spawn(
    self_id: String,
    interval: Duration,
    timeout: Duration,
    list_neighbors: Arc<dyn Fn() -> Vec<(String, String)> + Send + Sync>,
) -> Arc<NeighborProbe> {
    let probe = Arc::new(NeighborProbe::new());
    let p = probe.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(interval.max(Duration::from_secs(1)));
        // 让首次 sync_config 把节点列表灌进来再探，避免空轮日志噪声。
        tick.tick().await;
        loop {
            tick.tick().await;
            let neighbors = (list_neighbors)();
            p.gc_unknown(&neighbors);
            let mut joins = Vec::with_capacity(neighbors.len());
            for (id, addr) in neighbors {
                if id == self_id || addr.is_empty() {
                    continue;
                }
                let p = p.clone();
                let to = timeout;
                joins.push(tokio::spawn(async move {
                    match probe_once(&addr, to).await {
                        Ok(rtt) => p.record_ok(&id, rtt),
                        Err(_) => p.record_fail(&id),
                    }
                }));
            }
            for j in joins {
                let _ = j.await;
            }
        }
    });
    probe
}

async fn probe_once(addr: &str, timeout: Duration) -> Result<u32, ()> {
    // 解析放在 spawn_blocking 外：addr 一般是 ip:port，DNS 阻塞不大；
    // 失败直接当连不上，不区分 DNS / connect 错。
    let sa = match addr.to_socket_addrs().ok().and_then(|mut it| it.next()) {
        Some(a) => a,
        None => return Err(()),
    };
    let start = Instant::now();
    match tokio::time::timeout(timeout, TcpStream::connect(sa)).await {
        Ok(Ok(_s)) => Ok(start.elapsed().as_millis().min(u32::MAX as u128) as u32),
        _ => Err(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ewma_smooths_samples() {
        let p = NeighborProbe::new();
        p.record_ok("a", 100);
        p.record_ok("a", 50);
        // α=0.5：(50 + 100) / 2 = 75
        let snap = p.snapshot();
        assert_eq!(snap.get("a"), Some(&75));
    }

    #[test]
    fn first_sample_is_raw() {
        let p = NeighborProbe::new();
        p.record_ok("a", 42);
        assert_eq!(p.snapshot().get("a"), Some(&42));
    }

    #[test]
    fn fail_threshold_evicts_sample() {
        let p = NeighborProbe::new();
        p.record_ok("a", 30);
        assert!(p.snapshot().contains_key("a"));
        for _ in 0..FAIL_THRESHOLD {
            p.record_fail("a");
        }
        assert!(!p.snapshot().contains_key("a"), "连续失败应让 a 从 snapshot 移除");
    }

    #[test]
    fn single_fail_keeps_sample() {
        let p = NeighborProbe::new();
        p.record_ok("a", 30);
        p.record_fail("a");
        assert!(p.snapshot().contains_key("a"), "单次失败不应翻盘");
    }

    #[test]
    fn gc_drops_unknown() {
        let p = NeighborProbe::new();
        p.record_ok("a", 10);
        p.record_ok("b", 20);
        p.gc_unknown(&[("a".into(), "x:1".into())]);
        let snap = p.snapshot();
        assert!(snap.contains_key("a"));
        assert!(!snap.contains_key("b"));
    }
}
