-- M4.2-A 节点能力上报：fast path 支持状态等。JSON 文本，schema 自由。
-- 例：{"fastpath":true,"kernel":"6.1.0","reason":"ok","in_container":false}
-- NULL / 空 = 老节点未上报，UI 视为 fast path 不可用。
ALTER TABLE nodes ADD COLUMN capabilities TEXT NOT NULL DEFAULT '';
