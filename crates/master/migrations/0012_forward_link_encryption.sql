-- #27 链路加密开关：每条 forward 可选「节点↔节点」段是否走 TLS。
-- 'tls'   = 默认，节点间 raw_tunnel 走 rustls + aws-lc-rs（不变）
-- 'plain' = 节点间 raw_tunnel TCP 不裹 TLS（同机房 / 信任内网 / 极致性能）
-- 注意：仅影响 TCP raw_tunnel 链路；UDP 走 QUIC 协议层强制 TLS 无法关闭；
-- master ↔ node gRPC 控制面永远 mTLS（安全红线）。
-- 仅 admin 可设。
ALTER TABLE forwards ADD COLUMN link_encryption TEXT NOT NULL DEFAULT 'tls';
