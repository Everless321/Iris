-- forward 累计流量统计（master 层）。
-- node 端按 forward_id 维度 AtomicU64 计数,heartbeat 上报 current 值；
-- master 跟踪 per-(node,forward) last_reported 算 delta 累加到此处。
-- INTEGER (SQLite) 是 i64，u64 上限 2^63-1 ≈ 9.2 EB 足够。
ALTER TABLE forwards ADD COLUMN bytes_in INTEGER NOT NULL DEFAULT 0;
ALTER TABLE forwards ADD COLUMN bytes_out INTEGER NOT NULL DEFAULT 0;
