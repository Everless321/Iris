-- M7 节点资源监控：每节点最新一行 latest + 30s 采样历史
CREATE TABLE IF NOT EXISTS node_metrics_latest (
    node_id          TEXT PRIMARY KEY NOT NULL,
    -- 静态
    cpu_name         TEXT NOT NULL DEFAULT '',
    cpu_cores        INTEGER NOT NULL DEFAULT 0,
    arch             TEXT NOT NULL DEFAULT '',
    os               TEXT NOT NULL DEFAULT '',
    kernel           TEXT NOT NULL DEFAULT '',
    virtualization   TEXT NOT NULL DEFAULT '',
    -- 动态
    cpu_usage        REAL NOT NULL DEFAULT 0,
    ram_total        INTEGER NOT NULL DEFAULT 0,
    ram_used         INTEGER NOT NULL DEFAULT 0,
    swap_total       INTEGER NOT NULL DEFAULT 0,
    swap_used        INTEGER NOT NULL DEFAULT 0,
    disk_total       INTEGER NOT NULL DEFAULT 0,
    disk_used        INTEGER NOT NULL DEFAULT 0,
    load1            REAL NOT NULL DEFAULT 0,
    load5            REAL NOT NULL DEFAULT 0,
    load15           REAL NOT NULL DEFAULT 0,
    net_up_bps       INTEGER NOT NULL DEFAULT 0,
    net_down_bps     INTEGER NOT NULL DEFAULT 0,
    net_total_up     INTEGER NOT NULL DEFAULT 0,
    net_total_down   INTEGER NOT NULL DEFAULT 0,
    tcp_conns        INTEGER NOT NULL DEFAULT 0,
    udp_conns        INTEGER NOT NULL DEFAULT 0,
    uptime_secs      INTEGER NOT NULL DEFAULT 0,
    process_count    INTEGER NOT NULL DEFAULT 0,
    updated_at       INTEGER NOT NULL
);

-- 时序历史，30s 采样保留 24h（master 定期 trim）
CREATE TABLE IF NOT EXISTS node_metrics_history (
    node_id          TEXT NOT NULL,
    ts_ms            INTEGER NOT NULL,
    cpu_usage        REAL NOT NULL DEFAULT 0,
    ram_used         INTEGER NOT NULL DEFAULT 0,
    disk_used        INTEGER NOT NULL DEFAULT 0,
    load1            REAL NOT NULL DEFAULT 0,
    net_up_bps       INTEGER NOT NULL DEFAULT 0,
    net_down_bps     INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (node_id, ts_ms)
);
CREATE INDEX IF NOT EXISTS idx_node_metrics_history_ts ON node_metrics_history(ts_ms);
