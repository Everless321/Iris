#!/usr/bin/env bash
# P3 演示：健康探测 + 故障转移 + SLA。
# 出口组 [b1,b2]，杀掉 b1 后验证流量自动切到 b2，且 SLA 标记 b1 不健康。
set -e
cd "$(dirname "$0")/.."

echo "==> 编译"
cargo build -q

echo "==> 启动 master（探测间隔 2s，连续 2 次失败切流）"
rm -f data/iris.db*
IRIS_PROBE_INTERVAL=2 IRIS_FAIL_THRESHOLD=2 ./target/debug/iris-master > /tmp/zf-m.log 2>&1 &
MPID=$!
sleep 2

CLI=./target/debug/iris-cli
$CLI add-node --id a  --name entry --addr 127.0.0.1:7444 >/dev/null
$CLI add-node --id b1 --name exit1 --addr 127.0.0.1:7445 >/dev/null
$CLI add-node --id b2 --name exit2 --addr 127.0.0.1:7446 >/dev/null
$CLI add-forward --name ha --listen 10080 --hops "a | b1,b2@weighted" --target 127.0.0.1:7080 >/dev/null

echo "==> 启动 a/b1/b2"
IRIS_NODE_ID=b1 IRIS_DATA_ADDR=0.0.0.0:7445 ./target/debug/iris-node > /tmp/zf-b1.log 2>&1 & B1=$!
IRIS_NODE_ID=b2 IRIS_DATA_ADDR=0.0.0.0:7446 ./target/debug/iris-node > /tmp/zf-b2.log 2>&1 & B2=$!
IRIS_NODE_ID=a  IRIS_DATA_ADDR=0.0.0.0:7444 ./target/debug/iris-node > /tmp/zf-a.log 2>&1 & A=$!
sleep 5

echo "==> [正常] 8 连接，应分流 b1/b2"
for i in $(seq 1 8); do curl -s --max-time 5 localhost:10080/healthz >/dev/null; done
echo "    b1: $(grep -c 'pick=b1' /tmp/zf-a.log)  b2: $(grep -c 'pick=b2' /tmp/zf-a.log)"

echo "==> 杀掉 b1，模拟节点故障"
kill $B1 2>/dev/null
: > /tmp/zf-a.log   # 清空入口日志便于统计故障后分流
echo "==> 等待探测标记 b1 不健康 + 节点刷新视图（~12s）"
sleep 12

echo "==> [故障后] 8 连接，应全部切到 b2 且 100% 成功"
OK=0
for i in $(seq 1 8); do
  [ "$(curl -s --max-time 5 localhost:10080/healthz)" = "ok" ] && OK=$((OK+1))
done
echo "    成功: $OK/8   b1: $(grep -c 'pick=b1' /tmp/zf-a.log)  b2: $(grep -c 'pick=b2' /tmp/zf-a.log)"

echo "==> SLA 报告 (/api/sla)"
curl -s localhost:7080/api/sla | python3 -m json.tool 2>/dev/null || curl -s localhost:7080/api/sla
echo "==> Prometheus 指标节选 (/metrics)"
curl -s localhost:7080/metrics | grep -E "node_up|nodes_online|nodes_total"

echo "==> 清理"
kill $A $B2 $MPID 2>/dev/null || true
pkill -f iris-node 2>/dev/null || true
[ "$OK" = "8" ] && echo "✅ P3 故障转移演示通过（b1 故障后流量 100% 切到 b2）" || echo "⚠️ 查看 /tmp/zf-*.log"
