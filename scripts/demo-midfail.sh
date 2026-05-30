#!/usr/bin/env bash
# P3 进阶演示：中转节点故障转移。
# 路径 a -> [m1,m2] -> c。杀掉中转 m1，验证流量经 m2 继续到 c。
set -e
cd "$(dirname "$0")/.."

echo "==> 编译"; cargo build -q

echo "==> 启动 master（探测 2s）"
rm -f data/zhuanfa.db*
ZF_PROBE_INTERVAL=2 ZF_FAIL_THRESHOLD=2 ./target/debug/zhuanfa-master > /tmp/zf-m.log 2>&1 &
MPID=$!; sleep 2

CLI=./target/debug/zhuanfa-cli
$CLI add-node --id a  --name entry --addr 127.0.0.1:7444 >/dev/null
$CLI add-node --id m1 --name mid1  --addr 127.0.0.1:7445 >/dev/null
$CLI add-node --id m2 --name mid2  --addr 127.0.0.1:7446 >/dev/null
$CLI add-node --id c  --name exit  --addr 127.0.0.1:7447 >/dev/null
# 中转跳是节点组 [m1,m2]，出口 c
$CLI add-forward --name midha --listen 10080 --hops "a | m1,m2@weighted | c" --target 127.0.0.1:7080 >/dev/null

echo "==> 启动 a/m1/m2/c"
ZF_NODE_ID=m1 ZF_DATA_ADDR=0.0.0.0:7445 ./target/debug/zhuanfa-node > /tmp/zf-m1.log 2>&1 & M1=$!
ZF_NODE_ID=m2 ZF_DATA_ADDR=0.0.0.0:7446 ./target/debug/zhuanfa-node > /tmp/zf-m2.log 2>&1 & M2=$!
ZF_NODE_ID=c  ZF_DATA_ADDR=0.0.0.0:7447 ./target/debug/zhuanfa-node > /tmp/zf-c.log 2>&1 & C=$!
ZF_NODE_ID=a  ZF_DATA_ADDR=0.0.0.0:7444 ./target/debug/zhuanfa-node > /tmp/zf-a.log 2>&1 & A=$!
sleep 5

echo "==> [正常] 4 连接经中转组分流"
for i in $(seq 1 4); do curl -s --max-time 5 localhost:10080/healthz >/dev/null; done
echo "    a 选中转: m1=$(grep -c 'pick=m1' /tmp/zf-a.log) m2=$(grep -c 'pick=m2' /tmp/zf-a.log)"

echo "==> 杀掉中转 m1"
kill $M1 2>/dev/null
echo "==> 等待探测标记 m1 不健康（~12s）"
sleep 12

echo "==> [中转故障后] 8 连接，应经 m2 继续到 c，100% 成功"
OK=0
for i in $(seq 1 8); do
  [ "$(curl -s --max-time 5 localhost:10080/healthz)" = "ok" ] && OK=$((OK+1))
done
echo "    成功: $OK/8"
curl -s localhost:7080/api/sla | python3 -c "import sys,json; d=json.load(sys.stdin); print('    节点状态:', {n['id']: n['health'] for n in d['nodes']})" 2>/dev/null

echo "==> 清理"
kill $A $M2 $C $MPID 2>/dev/null || true
pkill -f zhuanfa-node 2>/dev/null || true
[ "$OK" = "8" ] && echo "✅ 中转故障转移通过（m1 挂后流量经 m2 到达 c）" || echo "⚠️ 查看 /tmp/zf-*.log"
