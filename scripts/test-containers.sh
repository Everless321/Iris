#!/usr/bin/env bash
# 容器测试：3 个节点容器走 enrollment 流程接入 host 上的 master。
# 期望流程：注册 → 拿 token → 启容器 → install 兑换证书 → mTLS 连 master → healthy
set -euo pipefail
cd "$(dirname "$0")/.."

API_HTTP="http://127.0.0.1:7080"
ADMIN_USER="admin"
ADMIN_PASS="admin12345"

echo "==> 等 master 起来..."
for i in $(seq 1 10); do
  curl -sf -o /dev/null "$API_HTTP/healthz" && break
  sleep 1
done

echo "==> 拿 admin JWT..."
TOKEN=$(curl -fsS -X POST "$API_HTTP/api/auth/login" \
  -H 'content-type: application/json' \
  -d "{\"username\":\"$ADMIN_USER\",\"password\":\"$ADMIN_PASS\"}" \
  | python3 -c "import json,sys;print(json.load(sys.stdin)['token'])")

# 清理旧的容器节点（如果之前跑过这个测试）
for id in docker-a docker-b1 docker-b2; do
  curl -s -X DELETE "$API_HTTP/api/nodes/$id" -H "Authorization: Bearer $TOKEN" >/dev/null || true
done

echo "==> 注册 3 个节点 (docker-a / docker-b1 / docker-b2)..."
register() {
  local id=$1 name=$2 addr=$3 weight=$4
  curl -fsS -X POST "$API_HTTP/api/nodes" \
    -H "Authorization: Bearer $TOKEN" -H 'content-type: application/json' \
    -d "{\"id\":\"$id\",\"name\":\"$name\",\"addr\":\"$addr\",\"weight\":$weight}" >/dev/null
}
register docker-a  "容器入口"      "host.docker.internal:17444" 1
register docker-b1 "容器出口-主"   "host.docker.internal:17445" 3
register docker-b2 "容器出口-备"   "host.docker.internal:17446" 1

echo "==> 生成 enrollment 令牌..."
new_token() {
  curl -fsS -X POST "$API_HTTP/api/nodes/$1/enrollment" \
    -H "Authorization: Bearer $TOKEN" \
    | python3 -c "import json,sys;print(json.load(sys.stdin)['token'])"
}
TA=$(new_token docker-a)
TB1=$(new_token docker-b1)
TB2=$(new_token docker-b2)
echo "    docker-a  token: ${TA:0:12}..."
echo "    docker-b1 token: ${TB1:0:12}..."
echo "    docker-b2 token: ${TB2:0:12}..."

echo "==> 写 docker-compose.nodes.yml 并启动容器..."
cat > docker-compose.nodes.yml <<EOF
# 3 个节点容器，每个走自己的 enrollment 令牌兑换证书后启动
services:
  node-a:
    image: zhuanfa:dev
    container_name: zf-node-a
    extra_hosts:
      - "host.docker.internal:host-gateway"
    environment:
      ZF_MASTER_HTTP: "http://host.docker.internal:7080"
      ZF_MASTER_GRPC: "https://host.docker.internal:7443"
      ZF_NODE_ID: "docker-a"
      ZF_DATA_ADDR: "0.0.0.0:7444"
      ZF_TOKEN: "$TA"
      RUST_LOG: info
    ports:
      - "17444:7444"
    command:
      - bash
      - -c
      - |
        set -e
        mkdir -p /data/certs
        RESP=\$\$(curl -fsS -X POST "\$\$ZF_MASTER_HTTP/api/nodes/enroll" \\
          -H 'content-type: application/json' \\
          -d "{\\"token\\":\\"\$\$ZF_TOKEN\\"}")
        echo "\$\$RESP" | jq -r .ca_pem    > /data/certs/ca.pem
        echo "\$\$RESP" | jq -r .cert_pem  > /data/certs/client.pem
        echo "\$\$RESP" | jq -r .key_pem   > /data/certs/client-key.pem
        chmod 600 /data/certs/*.pem
        echo "==> enrollment ok, starting node"
        export ZF_MASTER="\$\$ZF_MASTER_GRPC"
        exec zhuanfa-node

  node-b1:
    image: zhuanfa:dev
    container_name: zf-node-b1
    extra_hosts:
      - "host.docker.internal:host-gateway"
    environment:
      ZF_MASTER_HTTP: "http://host.docker.internal:7080"
      ZF_MASTER_GRPC: "https://host.docker.internal:7443"
      ZF_NODE_ID: "docker-b1"
      ZF_DATA_ADDR: "0.0.0.0:7444"
      ZF_TOKEN: "$TB1"
      RUST_LOG: info
    ports:
      - "17445:7444"
    command:
      - bash
      - -c
      - |
        set -e
        mkdir -p /data/certs
        RESP=\$\$(curl -fsS -X POST "\$\$ZF_MASTER_HTTP/api/nodes/enroll" \\
          -H 'content-type: application/json' \\
          -d "{\\"token\\":\\"\$\$ZF_TOKEN\\"}")
        echo "\$\$RESP" | jq -r .ca_pem    > /data/certs/ca.pem
        echo "\$\$RESP" | jq -r .cert_pem  > /data/certs/client.pem
        echo "\$\$RESP" | jq -r .key_pem   > /data/certs/client-key.pem
        chmod 600 /data/certs/*.pem
        export ZF_MASTER="\$\$ZF_MASTER_GRPC"
        exec zhuanfa-node

  node-b2:
    image: zhuanfa:dev
    container_name: zf-node-b2
    extra_hosts:
      - "host.docker.internal:host-gateway"
    environment:
      ZF_MASTER_HTTP: "http://host.docker.internal:7080"
      ZF_MASTER_GRPC: "https://host.docker.internal:7443"
      ZF_NODE_ID: "docker-b2"
      ZF_DATA_ADDR: "0.0.0.0:7444"
      ZF_TOKEN: "$TB2"
      RUST_LOG: info
    ports:
      - "17446:7444"
    command:
      - bash
      - -c
      - |
        set -e
        mkdir -p /data/certs
        RESP=\$\$(curl -fsS -X POST "\$\$ZF_MASTER_HTTP/api/nodes/enroll" \\
          -H 'content-type: application/json' \\
          -d "{\\"token\\":\\"\$\$ZF_TOKEN\\"}")
        echo "\$\$RESP" | jq -r .ca_pem    > /data/certs/ca.pem
        echo "\$\$RESP" | jq -r .cert_pem  > /data/certs/client.pem
        echo "\$\$RESP" | jq -r .key_pem   > /data/certs/client-key.pem
        chmod 600 /data/certs/*.pem
        export ZF_MASTER="\$\$ZF_MASTER_GRPC"
        exec zhuanfa-node
EOF

# 清掉旧容器
docker rm -f zf-node-a zf-node-b1 zf-node-b2 >/dev/null 2>&1 || true

docker compose -f docker-compose.nodes.yml up -d
sleep 8

echo ""
echo "==> 容器状态:"
docker ps --filter "name=zf-node-" --format "  {{.Names}}  {{.Status}}  {{.Ports}}"

echo ""
echo "==> 节点 a 日志 (enrollment + mTLS 连接):"
docker logs zf-node-a 2>&1 | tail -10

echo ""
echo "==> 节点 b1 日志:"
docker logs zf-node-b1 2>&1 | tail -6

echo ""
echo "==> master 视角节点健康状态:"
curl -fsS "$API_HTTP/api/nodes" -H "Authorization: Bearer $TOKEN" \
  | python3 -c "
import json, sys
for n in json.load(sys.stdin):
    if n['id'].startswith('docker-'):
        h = n['health']
        rtt = n['latency_ms']
        lat = f'{rtt}ms' if rtt is not None else '-'
        print(f\"  {n['id']:12} {n['name']:14} health={h:10} latency={lat}\")
"

echo ""
echo "==============================================================="
echo "✅ 3 个容器节点跑起来了"
echo "==============================================================="
echo ""
echo "查看："
echo "  docker logs -f zf-node-a       # 单容器实时日志"
echo "  docker ps --filter name=zf-node-"
echo "  浏览器: http://127.0.0.1:7080 → 节点页能看到 docker-a/b1/b2"
echo ""
echo "停掉："
echo "  docker compose -f docker-compose.nodes.yml down"
