-- 节点固有权重（加权负载均衡的默认权重，hops 内可覆盖）
ALTER TABLE nodes ADD COLUMN weight INTEGER NOT NULL DEFAULT 1;
