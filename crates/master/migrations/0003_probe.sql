-- 健康探测与 SLA 统计字段
ALTER TABLE nodes ADD COLUMN health       TEXT    NOT NULL DEFAULT 'unknown';
ALTER TABLE nodes ADD COLUMN latency_ms   INTEGER;                    -- 最近探测 RTT
ALTER TABLE nodes ADD COLUMN fail_count   INTEGER NOT NULL DEFAULT 0; -- 连续失败计数
ALTER TABLE nodes ADD COLUMN probe_total  INTEGER NOT NULL DEFAULT 0; -- 探测总次数
ALTER TABLE nodes ADD COLUMN probe_ok     INTEGER NOT NULL DEFAULT 0; -- 探测成功次数（可用率分子）
ALTER TABLE nodes ADD COLUMN fail_events  INTEGER NOT NULL DEFAULT 0; -- 故障事件次数
ALTER TABLE nodes ADD COLUMN down_since   INTEGER;                    -- 当前故障起始 unix ms
ALTER TABLE nodes ADD COLUMN downtime_ms  INTEGER NOT NULL DEFAULT 0; -- 累计不可用时长
