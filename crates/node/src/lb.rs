use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use iris_proto::control::{Hop, HopNode};

/// 单节点最大权重，防止恶意/误配权重导致 expand 内存爆炸。
const MAX_WEIGHT: u32 = 1000;

/// 节点健康/延迟视图快照。
#[derive(Default)]
pub struct NodeView {
    pub health: HashMap<String, String>,
    pub latency: HashMap<String, u32>,
}

impl NodeView {
    pub fn usable(&self, id: &str) -> bool {
        self.health.get(id).map(|h| h != "unhealthy").unwrap_or(true)
    }
    pub fn latency(&self, id: &str) -> u32 {
        self.latency.get(id).copied().unwrap_or(u32::MAX)
    }
}

/// 入口/中转选路器：为「下一跳节点组」按策略产出有序候选（主选在前，其余兜底用于 failover）。
pub struct LoadBalancer {
    rr: Mutex<HashMap<(i64, usize), usize>>,
    conns: Arc<Mutex<HashMap<String, usize>>>,
}

/// 连接计数守卫：跟踪实际建连成功的单个节点，drop 时递减。
pub struct ConnGuard {
    conns: Arc<Mutex<HashMap<String, usize>>>,
    node: String,
}

impl Drop for ConnGuard {
    fn drop(&mut self) {
        let mut c = self.conns.lock().unwrap();
        if let Some(v) = c.get_mut(&self.node) {
            *v = v.saturating_sub(1);
        }
    }
}

impl LoadBalancer {
    pub fn new() -> Self {
        Self {
            rr: Mutex::new(HashMap::new()),
            conns: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// 建连成功后调用，登记该节点活跃连接，返回守卫。
    pub fn track(&self, node: &str) -> ConnGuard {
        *self.conns.lock().unwrap().entry(node.to_string()).or_insert(0) += 1;
        ConnGuard {
            conns: self.conns.clone(),
            node: node.to_string(),
        }
    }

    /// 产出本跳的有序候选节点：[主选, 次选, ...]，全用于 failover 逐个尝试。
    /// 仅含健康节点；若全不健康则退化为全部（尽力转发）。
    pub fn select_ordered(
        &self,
        forward_id: i64,
        hop_idx: usize,
        hop: &Hop,
        client_ip: IpAddr,
        view: &NodeView,
    ) -> Vec<String> {
        let healthy: Vec<&HopNode> = hop.nodes.iter().filter(|n| view.usable(&n.id)).collect();
        let pool: Vec<&HopNode> = if healthy.is_empty() {
            hop.nodes.iter().collect()
        } else {
            healthy
        };
        if pool.len() == 1 {
            return vec![pool[0].id.clone()];
        }
        match hop.strategy.as_str() {
            "source_hash" => rendezvous(&pool, client_ip),
            "least_conn" => self.least_conn_ordered(&pool),
            "latency" => latency_ordered(&pool, view),
            _ => self.weighted_ordered(forward_id, hop_idx, &pool),
        }
    }

    fn weighted_ordered(&self, fid: i64, hop_idx: usize, pool: &[&HopNode]) -> Vec<String> {
        let expanded = expand(pool);
        let primary = {
            let mut rr = self.rr.lock().unwrap();
            let c = rr.entry((fid, hop_idx)).or_insert(0);
            let id = expanded[*c % expanded.len()].clone();
            *c = c.wrapping_add(1);
            id
        };
        // 主选在前，其余健康节点兜底（去重）
        let mut out = vec![primary.clone()];
        for n in pool {
            if n.id != primary && !out.contains(&n.id) {
                out.push(n.id.clone());
            }
        }
        out
    }

    fn least_conn_ordered(&self, pool: &[&HopNode]) -> Vec<String> {
        let c = self.conns.lock().unwrap();
        let mut v: Vec<&&HopNode> = pool.iter().collect();
        v.sort_by(|a, b| {
            let la = *c.get(&a.id).unwrap_or(&0) as f64 / a.weight.max(1) as f64;
            let lb = *c.get(&b.id).unwrap_or(&0) as f64 / b.weight.max(1) as f64;
            la.partial_cmp(&lb)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.id.cmp(&b.id))
        });
        v.into_iter().map(|n| n.id.clone()).collect()
    }
}

impl Default for LoadBalancer {
    fn default() -> Self {
        Self::new()
    }
}

/// 延迟最优：按探测延迟升序（平局按 id）。
fn latency_ordered(pool: &[&HopNode], view: &NodeView) -> Vec<String> {
    let mut v: Vec<&&HopNode> = pool.iter().collect();
    v.sort_by(|a, b| {
        view.latency(&a.id)
            .cmp(&view.latency(&b.id))
            .then_with(|| a.id.cmp(&b.id))
    });
    v.into_iter().map(|n| n.id.clone()).collect()
}

/// 加权 Rendezvous（HRW）哈希：节点上下线不影响其余节点相对顺序，保证会话保持。
/// score = weight * (hash(ip,id) / u64::MAX)，按 score 降序。
fn rendezvous(pool: &[&HopNode], ip: IpAddr) -> Vec<String> {
    let mut scored: Vec<(f64, String)> = pool
        .iter()
        .map(|n| {
            let mut h = DefaultHasher::new();
            ip.hash(&mut h);
            n.id.hash(&mut h);
            let frac = h.finish() as f64 / u64::MAX as f64;
            (n.weight.max(1) as f64 * frac, n.id.clone())
        })
        .collect();
    scored.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.1.cmp(&b.1))
    });
    scored.into_iter().map(|(_, id)| id).collect()
}

/// 按权重展开节点列表（weighted_rr 用）。
fn expand(pool: &[&HopNode]) -> Vec<String> {
    let mut v = Vec::new();
    for n in pool {
        for _ in 0..n.weight.clamp(1, MAX_WEIGHT) {
            v.push(n.id.clone());
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn hop(strategy: &str, nodes: &[(&str, u32)]) -> Hop {
        Hop {
            strategy: strategy.into(),
            nodes: nodes
                .iter()
                .map(|(id, w)| HopNode { id: (*id).into(), weight: *w })
                .collect(),
        }
    }
    fn ip(b: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(10, 0, 0, b))
    }
    fn view(health: &[(&str, &str)], latency: &[(&str, u32)]) -> NodeView {
        NodeView {
            health: health.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            latency: latency.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
        }
    }

    #[test]
    fn single_node() {
        let lb = LoadBalancer::new();
        let h = hop("weighted", &[("b", 1)]);
        assert_eq!(lb.select_ordered(1, 1, &h, ip(1), &NodeView::default()), vec!["b"]);
    }

    #[test]
    fn weighted_primary_respects_weights() {
        let lb = LoadBalancer::new();
        let h = hop("weighted", &[("b1", 3), ("b2", 1)]);
        let (mut b1, mut b2) = (0, 0);
        for _ in 0..400 {
            match lb.select_ordered(1, 1, &h, ip(1), &NodeView::default())[0].as_str() {
                "b1" => b1 += 1,
                "b2" => b2 += 1,
                _ => unreachable!(),
            }
        }
        assert_eq!((b1, b2), (300, 100));
    }

    #[test]
    fn weighted_has_failover_fallback() {
        let lb = LoadBalancer::new();
        let h = hop("weighted", &[("b1", 1), ("b2", 1)]);
        // 候选应包含全部健康节点用于 failover
        let ord = lb.select_ordered(1, 1, &h, ip(1), &NodeView::default());
        assert_eq!(ord.len(), 2);
    }

    #[test]
    fn source_hash_sticky() {
        let lb = LoadBalancer::new();
        let h = hop("source_hash", &[("b1", 1), ("b2", 1)]);
        let first = lb.select_ordered(1, 1, &h, ip(7), &NodeView::default())[0].clone();
        for _ in 0..50 {
            assert_eq!(lb.select_ordered(1, 1, &h, ip(7), &NodeView::default())[0], first);
        }
    }

    #[test]
    fn source_hash_stable_on_topology_change() {
        // rendezvous 关键性质：移除「非选中」节点不改变同 IP 的主选
        let lb = LoadBalancer::new();
        let h3 = hop("source_hash", &[("b1", 1), ("b2", 1), ("b3", 1)]);
        let chosen = lb.select_ordered(1, 1, &h3, ip(7), &NodeView::default())[0].clone();
        // 移除一个非选中节点
        let others: Vec<(&str, u32)> = [("b1", 1u32), ("b2", 1), ("b3", 1)]
            .into_iter()
            .filter(|(id, _)| *id != chosen)
            .collect();
        let removed = others[0].0;
        let h2 = hop(
            "source_hash",
            &[("b1", 1), ("b2", 1), ("b3", 1)]
                .into_iter()
                .filter(|(id, _)| *id != removed)
                .collect::<Vec<_>>(),
        );
        assert_eq!(
            lb.select_ordered(1, 1, &h2, ip(7), &NodeView::default())[0],
            chosen,
            "移除非选中节点不应改变主选（拓扑稳定）"
        );
    }

    #[test]
    fn unhealthy_skipped() {
        let lb = LoadBalancer::new();
        let h = hop("weighted", &[("b1", 1), ("b2", 1)]);
        let v = view(&[("b1", "unhealthy"), ("b2", "healthy")], &[]);
        for _ in 0..20 {
            assert_eq!(lb.select_ordered(1, 1, &h, ip(1), &v)[0], "b2");
        }
    }

    #[test]
    fn latency_lowest_first() {
        let lb = LoadBalancer::new();
        let h = hop("latency", &[("b1", 1), ("b2", 1)]);
        let v = view(&[], &[("b1", 50), ("b2", 8)]);
        assert_eq!(lb.select_ordered(1, 1, &h, ip(1), &v)[0], "b2");
    }

    #[test]
    fn least_conn_orders_by_load() {
        let lb = LoadBalancer::new();
        let h = hop("least_conn", &[("b1", 1), ("b2", 1)]);
        let _g = lb.track("b1"); // b1 占用一个连接
        assert_eq!(lb.select_ordered(1, 1, &h, ip(1), &NodeView::default())[0], "b2");
    }

    #[test]
    fn conn_guard_decrements() {
        let lb = LoadBalancer::new();
        {
            let _g = lb.track("x");
            assert_eq!(*lb.conns.lock().unwrap().get("x").unwrap(), 1);
        }
        assert_eq!(*lb.conns.lock().unwrap().get("x").unwrap(), 0);
    }

    #[test]
    fn huge_weight_capped() {
        let n = HopNode { id: "x".into(), weight: u32::MAX };
        assert_eq!(expand(&[&n]).len(), MAX_WEIGHT as usize);
    }

    #[test]
    fn all_unhealthy_degrades() {
        let lb = LoadBalancer::new();
        let h = hop("weighted", &[("b1", 1), ("b2", 1)]);
        let v = view(&[("b1", "unhealthy"), ("b2", "unhealthy")], &[]);
        assert_eq!(lb.select_ordered(1, 1, &h, ip(1), &v).len(), 2, "全挂退化为全部候选");
    }
}
