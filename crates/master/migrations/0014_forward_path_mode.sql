-- M4.2-C 转发路径模式：admin 可强制走 fast / slow，或留 'auto' 由节点决策。
-- 'auto'：单跳+非加密+节点支持 fastpath → fast；否则 slow
-- 'fast'：强制 fast path；条件不满足时节点 fallback slow + 上报
-- 'slow'：强制 user-space tokio 转发
ALTER TABLE forwards ADD COLUMN path_mode TEXT NOT NULL DEFAULT 'auto';
