-- 把 forwards.target 从单 host:port 字符串升级成 JSON 数组 [{addr,weight}]
-- 新增 target_strategy 列，默认 weighted。

ALTER TABLE forwards ADD COLUMN target_strategy TEXT NOT NULL DEFAULT 'weighted';

-- 旧值是 host:port 裸字符串，逐行 UPDATE 转成 JSON。
-- 用 SQLite 字符串拼接保证转义最小：host:port 不含 " 字符，对 JSON 是安全的。
UPDATE forwards
   SET target = '[{"addr":"' || target || '","weight":1}]'
 WHERE target IS NOT NULL
   AND target <> ''
   AND substr(target, 1, 1) <> '[';

-- 兼容：空字符串置成空数组（极少见）
UPDATE forwards
   SET target = '[]'
 WHERE target IS NULL OR target = '';
