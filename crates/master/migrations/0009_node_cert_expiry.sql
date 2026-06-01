-- 节点 cert NotAfter 时间戳。心跳由节点上报，UI 显示倒计时。
-- 0 / NULL = 老节点未上报（向后兼容）。
ALTER TABLE nodes ADD COLUMN cert_not_after_ms INTEGER NOT NULL DEFAULT 0;
