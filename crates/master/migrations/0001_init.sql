-- 节点：转发链路上的每一跳（入口 / 中转 / 家宽出口，对称设计）
CREATE TABLE IF NOT EXISTS nodes (
    id         TEXT PRIMARY KEY,            -- 节点唯一标识
    name       TEXT NOT NULL,
    addr       TEXT NOT NULL,               -- 节点间互联可达地址 host:port
    status     TEXT NOT NULL DEFAULT 'offline',
    last_seen  INTEGER,                     -- 最近心跳 unix ms
    created_at INTEGER NOT NULL
);

-- 转发规则：path 为 JSON 数组，支持任意 N 跳级联
CREATE TABLE IF NOT EXISTS forwards (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    name        TEXT NOT NULL,
    listen_port INTEGER NOT NULL,           -- 入口节点监听端口
    protocol    TEXT NOT NULL DEFAULT 'tcp',
    path        TEXT NOT NULL,              -- JSON: ["node_a","node_b",...]
    target      TEXT NOT NULL,              -- 最终目标 host:port
    enabled     INTEGER NOT NULL DEFAULT 1,
    created_at  INTEGER NOT NULL
);

-- 用户：P1 仅建表，鉴权逻辑留到 P4
CREATE TABLE IF NOT EXISTS users (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    username      TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    role          TEXT NOT NULL DEFAULT 'customer',
    created_at    INTEGER NOT NULL
);
