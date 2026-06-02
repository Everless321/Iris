-- #39 流量限制：每条 forward 可设置累计配额（quota）+ 速率上限（rate）+ 可选周期重置。
-- 上传 = bytes_in (客户端 → 入口 → target)；下载 = bytes_out (target → 入口 → 客户端)。
-- 所有字段 NULL = 该方向 / 该机制不限。配额超额时 master 端 enabled=0 + quota_exhausted_at_ms 记录，
-- 重置 cron 仅恢复 enabled=0 且 quota_exhausted_at_ms IS NOT NULL 的 forward（避免覆盖手动 disable）。

ALTER TABLE forwards ADD COLUMN quota_in_bytes INTEGER;
ALTER TABLE forwards ADD COLUMN quota_out_bytes INTEGER;
ALTER TABLE forwards ADD COLUMN rate_in_bps INTEGER;
ALTER TABLE forwards ADD COLUMN rate_out_bps INTEGER;
ALTER TABLE forwards ADD COLUMN quota_reset TEXT;             -- 'none' | 'daily' | 'monthly' (NULL 视为 none)
ALTER TABLE forwards ADD COLUMN quota_reset_at_ms INTEGER;    -- 下次重置 unix ms (UTC), NULL = 不会重置
ALTER TABLE forwards ADD COLUMN quota_exhausted_at_ms INTEGER; -- 触达上限时戳，NULL = 未触达

-- 加速重置 cron 扫描
CREATE INDEX IF NOT EXISTS idx_forwards_quota_reset_at ON forwards(quota_reset_at_ms);
