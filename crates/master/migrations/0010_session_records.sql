-- #36 TCP 会话级历史记录。
-- forward_sessions 是 source of truth：每条 TCP 连接一行。
-- forward_sessions_hourly 是降采样聚合，永久保留（即使明细被归档）。

CREATE TABLE forward_sessions (
    id TEXT PRIMARY KEY,
    forward_id INTEGER NOT NULL,
    entry_node_id TEXT NOT NULL,
    client_ip TEXT NOT NULL,
    client_port INTEGER NOT NULL,
    target_addr TEXT NOT NULL,
    hops_path TEXT NOT NULL,
    protocol TEXT NOT NULL,
    opened_at_ms INTEGER NOT NULL,
    closed_at_ms INTEGER,
    bytes_in INTEGER NOT NULL DEFAULT 0,
    bytes_out INTEGER NOT NULL DEFAULT 0,
    close_reason TEXT
);
CREATE INDEX idx_sessions_forward_time ON forward_sessions(forward_id, opened_at_ms DESC);
CREATE INDEX idx_sessions_active ON forward_sessions(forward_id, closed_at_ms);
CREATE INDEX idx_sessions_client_ip ON forward_sessions(client_ip);

CREATE TABLE forward_sessions_hourly (
    forward_id INTEGER NOT NULL,
    hour_start_ms INTEGER NOT NULL,
    session_count INTEGER NOT NULL DEFAULT 0,
    total_bytes_in INTEGER NOT NULL DEFAULT 0,
    total_bytes_out INTEGER NOT NULL DEFAULT 0,
    unique_clients INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (forward_id, hour_start_ms)
);
CREATE INDEX idx_sessions_hourly_time ON forward_sessions_hourly(hour_start_ms DESC);
