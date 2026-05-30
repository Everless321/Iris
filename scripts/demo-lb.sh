#!/usr/bin/env bash
# P2 演示：第二跳节点组加权负载均衡（b1:b2 = 3:1）。
set -e
cd "$(dirname "$0")/.."

echo "==> 编译"
cargo build -q

echo "==> 启动 master"
rm -f data/zhuanfa.db*
./target/debug/zhuanfa-master > /tmp/zf-m.log 2>&1 &
MPID=$!
sleep 2

CLI=./target/debug/zhuanfa-cli
echo "==> 注册节点：a 入口，b1(权重3)/b2(权重1) 出口组"
$CLI add-node --id a  --name entry --addr 127.0.0.1:7444 >/dev/null
$CLI add-node --id b1 --name exit1 --addr 127.0.0.1:7445 --weight 3 >/dev/null
$CLI add-node --id b2 --name exit2 --addr 127.0.0.1:7446 --weight 1 >/dev/null
echo "==> 创建加权负载均衡转发"
$CLI add-forward --name lb --listen 10080 --hops "a | b1:3,b2:1@weighted" --target 127.0.0.1:7080 >/dev/null

echo "==> 启动节点 a/b1/b2"
ZF_NODE_ID=b1 ZF_DATA_ADDR=0.0.0.0:7445 ./target/debug/zhuanfa-node > /tmp/zf-b1.log 2>&1 &
ZF_NODE_ID=b2 ZF_DATA_ADDR=0.0.0.0:7446 ./target/debug/zhuanfa-node > /tmp/zf-b2.log 2>&1 &
ZF_NODE_ID=a  ZF_DATA_ADDR=0.0.0.0:7444 ./target/debug/zhuanfa-node > /tmp/zf-a.log 2>&1 &
sleep 4

echo "==> 发 8 个连接，观察加权分流"
OK=0
for i in $(seq 1 8); do
  [ "$(curl -s --max-time 5 localhost:10080/healthz)" = "ok" ] && OK=$((OK+1))
done
B1=$(grep -c 'pick=b1' /tmp/zf-a.log || true)
B2=$(grep -c 'pick=b2' /tmp/zf-a.log || true)
echo "    成功连接: $OK/8   b1选中: $B1   b2选中: $B2   (期望 6:2)"

echo "==> 清理"
pkill -f zhuanfa-node 2>/dev/null || true
kill $MPID 2>/dev/null || true
[ "$OK" = "8" ] && [ "$B1" = "6" ] && [ "$B2" = "2" ] && echo "✅ P2 加权 LB 演示通过" || echo "⚠️ 结果与预期不符，查看 /tmp/zf-*.log"
