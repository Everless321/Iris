-- 防止同一 owner 抢同一 listen_port（不同 owner 允许同端口，因为入口节点可能不同）
-- 注意：实际端口冲突的最终检测在 node 端 TcpListener::bind，这是控制面侧的提前拒绝
CREATE UNIQUE INDEX IF NOT EXISTS idx_forwards_owner_port
  ON forwards(owner_id, listen_port);

-- 加速 sla_samples 按时间窗口查询（避免复合索引最左前缀失效导致全表扫描）
CREATE INDEX IF NOT EXISTS idx_probe_ts ON probe_samples(ts);

-- owner_id=0 表示历史数据/admin 创建（0004 引入时未回填）。
-- 文档化此约定，避免后续误删。
