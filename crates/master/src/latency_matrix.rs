//! M9.2 节点间延迟矩阵
//!
//! 每个 node 周期 TCP connect 其余 node 测 RTT，心跳里上报本视角的「邻居行」
//! `(from=自己, neighbors={to: rtt_ms})`。master 在内存里聚合成 N×N 矩阵，
//! `sync_config` 下发时按调用方 `from` 改写每个 `NodeAddr.latency_ms`，
//! 让 `latency` 策略选路用「以入口节点为原点」的真实 RTT。
//!
//! 仅内存：不持久化（重启后 30s 内由心跳重建）；TTL 兜底防下线节点污染。

use std::collections::HashMap;
use std::sync::RwLock;
use std::time::{Duration, Instant};

/// 单条边的最近样本：EWMA 已在 node 侧平滑过，master 只存最新值。
#[derive(Clone, Copy, Debug)]
struct Edge {
    rtt_ms: u32,
    updated_at: Instant,
}

/// 节点间延迟矩阵（仅内存）。
pub struct LatencyMatrix {
    /// (from_node_id, to_node_id) → Edge
    edges: RwLock<HashMap<(String, String), Edge>>,
    /// 超过此 TTL 未更新视为陈旧，查询时跳过。
    ttl: Duration,
}

impl LatencyMatrix {
    pub fn new(ttl: Duration) -> Self {
        Self { edges: RwLock::new(HashMap::new()), ttl }
    }

    /// 写入 `from` 节点的一行（来自其心跳）。
    /// 空行表示老节点未上报，调用方应在心跳层判定后再调；这里不做特判，直接覆盖。
    pub fn record_row(&self, from: &str, neighbors: &HashMap<String, u32>) {
        let now = Instant::now();
        let mut g = self.edges.write().unwrap();
        for (to, rtt) in neighbors {
            if to.is_empty() || to == from {
                continue;
            }
            g.insert(
                (from.to_string(), to.clone()),
                Edge { rtt_ms: *rtt, updated_at: now },
            );
        }
    }

    /// 取 `from` 看向各 `to_ids` 的 RTT 行（仅返回未过期且确有样本的项）。
    pub fn row(&self, from: &str, to_ids: &[String]) -> HashMap<String, u32> {
        let now = Instant::now();
        let g = self.edges.read().unwrap();
        let mut out = HashMap::new();
        for to in to_ids {
            if to == from {
                continue;
            }
            if let Some(e) = g.get(&(from.to_string(), to.clone())) {
                if now.duration_since(e.updated_at) <= self.ttl {
                    out.insert(to.clone(), e.rtt_ms);
                }
            }
        }
        out
    }

    /// 删除某节点相关的所有边（节点离线/注销时调用）。
    #[allow(dead_code)]
    pub fn drop_node(&self, node_id: &str) {
        let mut g = self.edges.write().unwrap();
        g.retain(|(f, t), _| f != node_id && t != node_id);
    }

    /// 导出全量矩阵 (仅未过期边)，给 admin API 展示用。
    /// 返回 {from: {to: rtt_ms}}。
    pub fn dump(&self) -> HashMap<String, HashMap<String, u32>> {
        let now = Instant::now();
        let g = self.edges.read().unwrap();
        let mut out: HashMap<String, HashMap<String, u32>> = HashMap::new();
        for ((from, to), e) in g.iter() {
            if now.duration_since(e.updated_at) <= self.ttl {
                out.entry(from.clone()).or_default().insert(to.clone(), e.rtt_ms);
            }
        }
        out
    }

    /// 全表 GC，按 TTL 清陈旧边。后台周期任务调用即可。
    pub fn gc(&self) -> usize {
        let now = Instant::now();
        let mut g = self.edges.write().unwrap();
        let before = g.len();
        g.retain(|_, e| now.duration_since(e.updated_at) <= self.ttl);
        before - g.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nbrs(pairs: &[(&str, u32)]) -> HashMap<String, u32> {
        pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    #[test]
    fn record_and_row() {
        let m = LatencyMatrix::new(Duration::from_secs(60));
        m.record_row("a", &nbrs(&[("b", 10), ("c", 30)]));
        let row = m.row("a", &["b".into(), "c".into(), "d".into()]);
        assert_eq!(row.get("b"), Some(&10));
        assert_eq!(row.get("c"), Some(&30));
        assert!(row.get("d").is_none(), "d 无样本应缺失");
    }

    #[test]
    fn skips_self_edge() {
        let m = LatencyMatrix::new(Duration::from_secs(60));
        m.record_row("a", &nbrs(&[("a", 1), ("b", 9)]));
        assert!(m.row("a", &["a".into(), "b".into()]).get("a").is_none());
    }

    #[test]
    fn ttl_evicts_stale_on_query() {
        let m = LatencyMatrix::new(Duration::from_millis(20));
        m.record_row("a", &nbrs(&[("b", 5)]));
        std::thread::sleep(Duration::from_millis(40));
        assert!(m.row("a", &["b".into()]).is_empty(), "TTL 过期应跳过");
    }

    #[test]
    fn gc_cleans_stale() {
        let m = LatencyMatrix::new(Duration::from_millis(20));
        m.record_row("a", &nbrs(&[("b", 5), ("c", 8)]));
        std::thread::sleep(Duration::from_millis(40));
        m.record_row("a", &nbrs(&[("b", 7)])); // 仅 b 刷新
        let removed = m.gc();
        assert_eq!(removed, 1, "仅 (a,c) 应被清掉");
    }

    #[test]
    fn drop_node_removes_both_directions() {
        let m = LatencyMatrix::new(Duration::from_secs(60));
        m.record_row("a", &nbrs(&[("b", 5)]));
        m.record_row("b", &nbrs(&[("a", 6)]));
        m.drop_node("a");
        assert!(m.row("a", &["b".into()]).is_empty());
        assert!(m.row("b", &["a".into()]).is_empty());
    }
}
