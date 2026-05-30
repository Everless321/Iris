#!/usr/bin/env bash
set -uo pipefail
cd "$(dirname "$0")/.."

LOG=/tmp/zf-udp-verify
mkdir -p $LOG
rm -rf $LOG/* /tmp/zf-udp-certs-*

ADMIN_USER=zfudp
ADMIN_PASS=zfudp_test_secret
API_HTTP=http://127.0.0.1:7080
API=$API_HTTP/api

cleanup() {
    pkill -f 'target/debug/zhuanfa-(master|node)' 2>/dev/null || true
    sleep 0.5
    rm -rf /tmp/zf-udp-certs-*
}
trap cleanup EXIT

echo "==> [1/7] 编译（debug）"
cargo build -q 2>&1 | tail -3

echo "==> [2/7] 启动 master"
rm -f data/zhuanfa.db*
ZF_ADMIN_USER=$ADMIN_USER ZF_ADMIN_PASS=$ADMIN_PASS RUST_LOG=info \
    ./target/debug/zhuanfa-master > $LOG/master.log 2>&1 &
for i in $(seq 1 20); do
    sleep 0.5
    curl -fsS $API_HTTP/healthz >/dev/null 2>&1 && break
done

echo "==> [3/7] admin login"
TOKEN=$(curl -fsS -X POST $API/auth/login \
    -H 'content-type: application/json' \
    -d "{\"username\":\"$ADMIN_USER\",\"password\":\"$ADMIN_PASS\"}" \
    | jq -r .token)
if [ -z "$TOKEN" ] || [ "$TOKEN" = "null" ]; then
    echo "login 失败"; tail -20 $LOG/master.log; exit 1
fi
AUTH="Authorization: Bearer $TOKEN"
echo "    token: ${TOKEN:0:32}..."

echo "==> [4/7] 注册 + enroll 节点 a/b/c"
PORT_A=7444; PORT_B=7445; PORT_C=7446
for entry in "a:entry:$PORT_A" "b:relay:$PORT_B" "c:exit:$PORT_C"; do
    id=$(echo $entry | cut -d: -f1)
    name=$(echo $entry | cut -d: -f2)
    port=$(echo $entry | cut -d: -f3)
    curl -fsS -X POST $API/nodes -H 'content-type: application/json' -H "$AUTH" \
        -d "{\"id\":\"$id\",\"name\":\"$name\",\"addr\":\"127.0.0.1:$port\",\"weight\":1}" >/dev/null
    ETOK=$(curl -fsS -X POST $API/nodes/$id/enrollment -H "$AUTH" | jq -r .token)
    RESP=$(curl -fsS -X POST $API/nodes/enroll -H 'content-type: application/json' \
        -d "{\"token\":\"$ETOK\"}")
    D=/tmp/zf-udp-certs-$id
    mkdir -p $D
    echo "$RESP" | jq -r .ca_pem    > $D/ca.pem
    echo "$RESP" | jq -r .cert_pem  > $D/client.pem
    echo "$RESP" | jq -r .key_pem   > $D/client-key.pem
    chmod 600 $D/*.pem
done

echo "==> [5/7] 创建 forwards: tcp_single / udp_single / udp_multi / dual"
# 必须先建 forward 再启 node：node 进程当前只在启动时读一次配置
curl -fsS -X POST $API/forwards -H 'content-type: application/json' -H "$AUTH" \
    -d '{"name":"tcp_single","listen_port":10181,"protocol":"tcp","path":["a"],"target":"127.0.0.1:7080"}' >/dev/null
curl -fsS -X POST $API/forwards -H 'content-type: application/json' -H "$AUTH" \
    -d '{"name":"udp_single","listen_port":10153,"protocol":"udp","path":["a"],"target":"8.8.8.8:53"}' >/dev/null
curl -fsS -X POST $API/forwards -H 'content-type: application/json' -H "$AUTH" \
    -d '{"name":"udp_multi","listen_port":10253,"protocol":"udp","path":["a","b","c"],"target":"8.8.8.8:53"}' >/dev/null
curl -fsS -X POST $API/forwards -H 'content-type: application/json' -H "$AUTH" \
    -d '{"name":"dual","listen_port":10353,"protocol":"tcp+udp","path":["a"],"target":"1.1.1.1:53"}' >/dev/null

echo "==> [6/7] 启动 node a/b/c"
ZF_NODE_ID=a ZF_DATA_ADDR=0.0.0.0:$PORT_A ZF_CERT_DIR=/tmp/zf-udp-certs-a \
    ZF_MASTER=https://127.0.0.1:7443 RUST_LOG=info \
    ./target/debug/zhuanfa-node > $LOG/node-a.log 2>&1 &
ZF_NODE_ID=b ZF_DATA_ADDR=0.0.0.0:$PORT_B ZF_CERT_DIR=/tmp/zf-udp-certs-b \
    ZF_MASTER=https://127.0.0.1:7443 RUST_LOG=info \
    ./target/debug/zhuanfa-node > $LOG/node-b.log 2>&1 &
ZF_NODE_ID=c ZF_DATA_ADDR=0.0.0.0:$PORT_C ZF_CERT_DIR=/tmp/zf-udp-certs-c \
    ZF_MASTER=https://127.0.0.1:7443 RUST_LOG=info \
    ./target/debug/zhuanfa-node > $LOG/node-c.log 2>&1 &
sleep 4

echo "==> [7/7] 验证"
PASS=0; FAIL=0
pf() { if [ "$1" = "P" ]; then echo "    [PASS] $2"; PASS=$((PASS+1)); else echo "    [FAIL] $2"; FAIL=$((FAIL+1)); fi; }

# T1: TCP single-hop 不破坏现有逻辑
RESP=$(curl -fsS --max-time 5 http://127.0.0.1:10181/healthz 2>/dev/null || true)
[ "$RESP" = "ok" ] && pf P "TCP single-hop curl /healthz" || pf F "TCP single-hop (got: '$RESP')"

# T2: UDP single-hop DNS
DIG1=$(dig @127.0.0.1 -p 10153 google.com +time=3 +tries=1 +short 2>/dev/null | tail -1)
echo "$DIG1" | grep -qE '^[0-9a-f.:]+$' && pf P "UDP single-hop dig google.com → $DIG1" || pf F "UDP single-hop dig (got: '$DIG1')"

# T3: UDP multi-hop a→b→c → 8.8.8.8:53
DIG2=$(dig @127.0.0.1 -p 10253 example.com +time=6 +tries=1 +short 2>/dev/null | tail -1)
echo "$DIG2" | grep -qE '^[0-9a-f.:]+$' && pf P "UDP multi-hop dig example.com → $DIG2" || pf F "UDP multi-hop dig (got: '$DIG2')"

# T4: 同端口 TCP+UDP — UDP 侧
DIG3=$(dig @127.0.0.1 -p 10353 cloudflare.com +time=3 +tries=1 +short 2>/dev/null | tail -1)
echo "$DIG3" | grep -qE '^[0-9a-f.:]+$' && pf P "dual UDP side → $DIG3" || pf F "dual UDP (got: '$DIG3')"

# T5: 同端口 TCP+UDP — TCP 侧（DoT 用 TCP 53）
DIG4=$(dig @127.0.0.1 -p 10353 cloudflare.com +tcp +time=3 +tries=1 +short 2>/dev/null | tail -1)
echo "$DIG4" | grep -qE '^[0-9a-f.:]+$' && pf P "dual TCP side → $DIG4" || pf F "dual TCP (got: '$DIG4')"

# T6: 非法 protocol 校验
HTTP=$(curl -s -o /dev/null -w "%{http_code}" -X POST $API/forwards -H 'content-type: application/json' -H "$AUTH" \
    -d '{"name":"bad","listen_port":10999,"protocol":"foo","path":["a"],"target":"8.8.8.8:53"}')
[ "$HTTP" = "400" ] && pf P "master 拒绝非法 protocol" || pf F "master 应拒绝非法 protocol (got HTTP $HTTP)"

# T7: protocol 归一化 — "udp+tcp" 应规范为 "tcp+udp"
NORM=$(curl -fsS -X POST $API/forwards -H 'content-type: application/json' -H "$AUTH" \
    -d '{"name":"norm","listen_port":10888,"protocol":"udp+tcp","path":["a"],"target":"8.8.8.8:53"}' | jq -r .protocol)
[ "$NORM" = "tcp+udp" ] && pf P "protocol 归一化 udp+tcp → tcp+udp" || pf F "归一化失败 (got: '$NORM')"

echo
echo "==> 结果: $PASS pass / $FAIL fail   (日志: $LOG/*.log)"
[ $FAIL -eq 0 ] && echo "✅ UDP 双协议验证通过" || echo "❌ 有失败用例"
exit $FAIL
