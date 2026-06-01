use iris_proto::control::SessionEvent;
use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 单条 forward TCP/UDP 会话的运行时状态。bytes_in/out 由 forward read/write loop
/// 通过 add_in/add_out 累加（AtomicU64，~5ns 开销）。close_reason 在 task drop 时设。
pub struct SessionState {
    pub id: String,
    pub forward_id: i64,
    pub entry_node_id: String,
    pub client_ip: String,
    pub client_port: u32,
    pub target_addr: String,
    pub hops_path: Vec<String>,
    pub protocol: String,
    pub opened_at_ms: i64,
    pub bytes_in: AtomicU64,
    pub bytes_out: AtomicU64,
    /// 0 = active，关闭后填入 unix ms 时间戳
    pub closed_at_ms: AtomicI64,
    pub close_reason: RwLock<String>,
}

impl SessionState {
    #[inline]
    pub fn add_in(&self, n: usize) {
        self.bytes_in.fetch_add(n as u64, Ordering::Relaxed);
    }
    #[inline]
    pub fn add_out(&self, n: usize) {
        self.bytes_out.fetch_add(n as u64, Ordering::Relaxed);
    }

    /// 标记会话关闭。多次调用幂等（仅首次生效）。
    pub fn close(&self, reason: &str) {
        if self
            .closed_at_ms
            .compare_exchange(0, now_ms(), Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        {
            if let Ok(mut w) = self.close_reason.write() {
                *w = reason.into();
            }
        }
    }

    fn to_event(&self) -> SessionEvent {
        SessionEvent {
            id: self.id.clone(),
            forward_id: self.forward_id,
            entry_node_id: self.entry_node_id.clone(),
            client_ip: self.client_ip.clone(),
            client_port: self.client_port,
            target_addr: self.target_addr.clone(),
            hops_path: self.hops_path.clone(),
            protocol: self.protocol.clone(),
            opened_at_ms: self.opened_at_ms,
            closed_at_ms: self.closed_at_ms.load(Ordering::Relaxed),
            bytes_in: self.bytes_in.load(Ordering::Relaxed),
            bytes_out: self.bytes_out.load(Ordering::Relaxed),
            close_reason: self
                .close_reason
                .read()
                .map(|s| s.clone())
                .unwrap_or_default(),
        }
    }
}

/// 节点全局会话表。entry forward 创建 session 时 insert；
/// heartbeat 时 snapshot_and_gc 拷贝当前所有 + 移除已 close 1 轮以上的 entry。
#[derive(Default)]
pub struct SessionTable {
    sessions: RwLock<HashMap<String, Arc<SessionState>>>,
}

impl SessionTable {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// 入口 accept 时创建 session。target_addr/hops_path 由调用方填实际值。
    pub fn create(
        &self,
        forward_id: i64,
        entry_node_id: &str,
        client_ip: String,
        client_port: u32,
        target_addr: String,
        hops_path: Vec<String>,
        protocol: &str,
    ) -> Arc<SessionState> {
        let s = Arc::new(SessionState {
            id: Uuid::new_v4().simple().to_string(),
            forward_id,
            entry_node_id: entry_node_id.into(),
            client_ip,
            client_port,
            target_addr,
            hops_path,
            protocol: protocol.into(),
            opened_at_ms: now_ms(),
            bytes_in: AtomicU64::new(0),
            bytes_out: AtomicU64::new(0),
            closed_at_ms: AtomicI64::new(0),
            close_reason: RwLock::new(String::new()),
        });
        self.sessions
            .write()
            .unwrap()
            .insert(s.id.clone(), s.clone());
        s
    }

    /// heartbeat 调用：返回当前所有会话的 SessionEvent 快照，并 GC 已上报 close 的条目。
    /// 策略：close 后保留 1 轮（让 master 至少拿到一次 final 状态），下一轮 GC。
    /// 用 closed_at_ms 是否 > 0 + reported_close 标记区分。简化版：close 后立刻让下一轮 snapshot 看到，
    /// 再下一轮调用时清理。这里用 grace_ms 时间窗判定：close 超过 6s 即可清除（覆盖 5s 心跳间隔）。
    pub fn snapshot_and_gc(&self) -> Vec<SessionEvent> {
        let now = now_ms();
        const CLOSE_GRACE_MS: i64 = 6_000;
        let mut events = Vec::new();
        let mut to_remove = Vec::new();
        {
            let g = self.sessions.read().unwrap();
            for (id, s) in g.iter() {
                let closed = s.closed_at_ms.load(Ordering::Relaxed);
                events.push(s.to_event());
                if closed > 0 && now - closed > CLOSE_GRACE_MS {
                    to_remove.push(id.clone());
                }
            }
        }
        if !to_remove.is_empty() {
            let mut w = self.sessions.write().unwrap();
            for id in to_remove {
                w.remove(&id);
            }
        }
        events
    }

    #[allow(dead_code)]
    pub fn active_count(&self) -> usize {
        self.sessions
            .read()
            .unwrap()
            .values()
            .filter(|s| s.closed_at_ms.load(Ordering::Relaxed) == 0)
            .count()
    }
}
