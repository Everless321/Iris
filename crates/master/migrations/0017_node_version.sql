-- M8 节点版本上报：heartbeat 写入 node_version 字符串。
-- "" = 老节点不支持上报；UI 显示 "—"。
ALTER TABLE nodes ADD COLUMN version TEXT NOT NULL DEFAULT '';
ALTER TABLE nodes ADD COLUMN version_updated_at_ms INTEGER NOT NULL DEFAULT 0;
