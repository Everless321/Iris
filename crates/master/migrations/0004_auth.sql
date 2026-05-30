-- 用户密码哈希（argon2）；admin/customer 角色已在 0001 建好
ALTER TABLE users ADD COLUMN updated_at INTEGER NOT NULL DEFAULT 0;

-- 邀请码：admin 生成，customer 自助注册时一次性使用
CREATE TABLE IF NOT EXISTS invite_codes (
    code        TEXT PRIMARY KEY,
    created_by  INTEGER NOT NULL,         -- 生成此码的 admin user_id
    used_by     INTEGER,                  -- 使用此码注册的 user_id（NULL=未使用）
    used_at     INTEGER,                  -- 使用时刻
    created_at  INTEGER NOT NULL,
    FOREIGN KEY (created_by) REFERENCES users(id),
    FOREIGN KEY (used_by)    REFERENCES users(id)
);

-- 转发归属：owner_id 关联 users.id。0=admin 创建（保留兼容历史数据）
ALTER TABLE forwards ADD COLUMN owner_id INTEGER NOT NULL DEFAULT 0;
CREATE INDEX IF NOT EXISTS idx_forwards_owner ON forwards(owner_id);

-- SLA 探测样本环形缓冲：每节点每次探测追加一行，按节点滚动删旧
CREATE TABLE IF NOT EXISTS probe_samples (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    node_id     TEXT NOT NULL,
    ts          INTEGER NOT NULL,         -- 探测时刻 unix ms
    ok          INTEGER NOT NULL,         -- 1/0
    latency_ms  INTEGER,                  -- 探测 RTT（失败为 NULL）
    FOREIGN KEY (node_id) REFERENCES nodes(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_probe_node_ts ON probe_samples(node_id, ts);
