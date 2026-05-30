#!/usr/bin/env bash
# P1 演示：本地起 master + 3 节点，建一条 3 跳转发，验证端到端透传。
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
echo "==> 注册节点 a/b/c 并创建 3 跳转发 (10081 → a→b→c → master:7080)"
$CLI add-node --id a --name entry  --addr 127.0.0.1:7444 >/dev/null
$CLI add-node --id b --name relay  --addr 127.0.0.1:7445 >/dev/null
$CLI add-node --id c --name exit   --addr 127.0.0.1:7446 >/dev/null
$CLI add-forward --name demo3 --listen 10081 --path a,b,c --target 127.0.0.1:7080 >/dev/null

echo "==> 启动节点 c / b / a"
ZF_NODE_ID=c ZF_DATA_ADDR=0.0.0.0:7446 ./target/debug/zhuanfa-node > /tmp/zf-c.log 2>&1 & CPID=$!
ZF_NODE_ID=b ZF_DATA_ADDR=0.0.0.0:7445 ./target/debug/zhuanfa-node > /tmp/zf-b.log 2>&1 & BPID=$!
ZF_NODE_ID=a ZF_DATA_ADDR=0.0.0.0:7444 ./target/debug/zhuanfa-node > /tmp/zf-a.log 2>&1 & APID=$!
sleep 4

echo "==> 测试：curl 经 3 跳访问 /healthz"
RESULT=$(curl -s --max-time 5 localhost:10081/healthz)
echo "    返回: $RESULT  (期望 ok)"

echo "==> 转发列表"
$CLI list-forwards

echo "==> 清理"
kill $APID $BPID $CPID $MPID 2>/dev/null || true
[ "$RESULT" = "ok" ] && echo "✅ P1 演示通过" || echo "❌ 失败，查看 /tmp/zf-*.log"
