-- M8: agent 远程命令队列 + ack 历史
-- 一行 = 一条命令；ack 通过 status / stage / detail 原地 update 推进
CREATE TABLE IF NOT EXISTS node_commands (
  request_id   TEXT PRIMARY KEY,
  node_id      TEXT NOT NULL,
  kind         TEXT NOT NULL,           -- 'upgrade' | 'reload' | ...
  payload      TEXT NOT NULL,           -- JSON 序列化 UpgradeCommand 等
  status       INTEGER NOT NULL,        -- CommandStatus enum int
  stage        TEXT NOT NULL DEFAULT '',
  detail       TEXT NOT NULL DEFAULT '',
  issued_by    INTEGER,                 -- user_id 触发，NULL = 系统
  issued_at_ms INTEGER NOT NULL,
  delivered_at_ms INTEGER,              -- 节点首次 ACK RECEIVED 的时间
  finished_at_ms  INTEGER,              -- SUCCESS/FAILED/REJECTED 终态时间
  FOREIGN KEY (node_id) REFERENCES nodes(id) ON DELETE CASCADE,
  FOREIGN KEY (issued_by) REFERENCES users(id)
);

CREATE INDEX IF NOT EXISTS idx_node_commands_node ON node_commands(node_id, issued_at_ms);
CREATE INDEX IF NOT EXISTS idx_node_commands_pending ON node_commands(node_id, status) WHERE status IN (1, 2);
