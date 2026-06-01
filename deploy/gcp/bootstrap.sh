#!/usr/bin/env bash
# iris-node bootstrap: read GCP metadata, enroll, write env, start service.
# Triggered by iris-bootstrap.service on first boot.
set -euo pipefail

MD="http://metadata.google.internal/computeMetadata/v1/instance"
HDR="Metadata-Flavor: Google"
IRIS_DIR=/opt/iris
LOG=/var/log/iris-bootstrap.log

log() { echo "[$(date -Is)] $*" | tee -a "$LOG"; }

meta() {
  curl -fsS -H "$HDR" "$MD/attributes/$1" 2>/dev/null || true
}

mkdir -p "$IRIS_DIR"
cd "$IRIS_DIR"

if [[ -f "$IRIS_DIR/.bootstrapped" ]]; then
  log "already bootstrapped, skipping"
  exit 0
fi

log "reading GCP metadata"
MASTER=$(meta iris-master)
TOKEN=$(meta iris-enroll-token)
NODE_ID=$(meta iris-node-id)
DATA_ADDR=$(meta iris-data-addr)

if [[ -z "$MASTER" || -z "$TOKEN" || -z "$NODE_ID" ]]; then
  log "ERROR: missing required metadata (iris-master / iris-enroll-token / iris-node-id)"
  exit 1
fi

if [[ "$MASTER" == *:* ]]; then
  MASTER_HOST="${MASTER%:*}"
else
  MASTER_HOST="$MASTER"
fi

if [[ -z "$DATA_ADDR" || "$DATA_ADDR" == "auto" ]]; then
  EXTIP=$(curl -fsS -H "$HDR" "$MD/network-interfaces/0/access-configs/0/external-ip")
  DATA_ADDR="${EXTIP}:7444"
  log "auto data-addr = $DATA_ADDR"
fi

MASTER_HTTP="http://${MASTER_HOST}:7080"
GRPC_ADDR="https://${MASTER_HOST}:7443"
log "master_host=$MASTER_HOST http=$MASTER_HTTP grpc=$GRPC_ADDR node=$NODE_ID data=$DATA_ADDR"

log "calling enroll API"
RESP=$(curl -fsS -X POST "$MASTER_HTTP/api/nodes/enroll" \
  -H 'content-type: application/json' \
  -d "{\"token\":\"$TOKEN\"}")

echo "$RESP" | jq -r .ca_pem   > "$IRIS_DIR/ca.pem"
echo "$RESP" | jq -r .cert_pem > "$IRIS_DIR/client.pem"
echo "$RESP" | jq -r .key_pem  > "$IRIS_DIR/client-key.pem"

REAL_NODE_ID=$(echo "$RESP" | jq -r .node_id)

cat > "$IRIS_DIR/node.env" <<EOF
IRIS_NODE_ID=$REAL_NODE_ID
IRIS_MASTER=$GRPC_ADDR
IRIS_CERT_DIR=$IRIS_DIR
IRIS_DATA_ADDR=0.0.0.0:7444
RUST_LOG=info
EOF

chmod 600 "$IRIS_DIR"/{ca.pem,client.pem,client-key.pem,node.env}
chmod 700 "$IRIS_DIR"
touch "$IRIS_DIR/.bootstrapped"

log "enroll OK, node_id=$REAL_NODE_ID, starting iris-node.service"
systemctl daemon-reload
systemctl enable iris-node.service
systemctl start --no-block iris-node.service
log "done"
