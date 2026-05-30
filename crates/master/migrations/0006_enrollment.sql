-- 节点注册令牌：admin 生成 → 节点安装脚本携带 → 一次性兑换证书
CREATE TABLE IF NOT EXISTS node_enrollment_tokens (
    token       TEXT PRIMARY KEY,
    node_id     TEXT NOT NULL,
    expires_at  INTEGER NOT NULL,   -- unix ms
    used_at     INTEGER,
    created_at  INTEGER NOT NULL,
    FOREIGN KEY (node_id) REFERENCES nodes(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_enroll_node ON node_enrollment_tokens(node_id);
