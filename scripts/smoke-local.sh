#!/usr/bin/env bash
# Local end-to-end smoke test.
#
# Prerequisites (all running before you invoke this):
#   docker compose up          — Postgres + Redis
#   cargo run -p coordinator   — coordinator on :4000 (with .env.dev loaded)
#   pinaivu-node               — node on :5000, connected to coordinator
#   ollama serve               — Ollama with the model the node is configured for
#   cargo run (indexer)        — indexer on :3100 (with its .env loaded)
#
# Usage:
#   bash scripts/smoke-local.sh [model] [prompt]
#   bash scripts/smoke-local.sh llama3.2:1b "what is 2+2?"

set -euo pipefail

COORDINATOR="${COORDINATOR_URL:-http://localhost:4000}"
INDEXER="${INDEXER_URL:-http://localhost:3100}"
MODEL="${1:-llama3.2:1b}"
PROMPT="${2:-what is 2+2?}"
# Dummy 32-byte client pubkey for dev (not verified in dev mode)
CLIENT_PUBKEY="$(printf '01%.0s' {1..32})"

echo "=== Pinaivu local smoke ==="
echo "coordinator : $COORDINATOR"
echo "indexer     : $INDEXER"
echo "model       : $MODEL"
echo "prompt      : $PROMPT"
echo ""

# ── Step 1: auction + dispatch ────────────────────────────────────────────────
echo "[1/4] POST /v1/chat/completions ..."
RESP=$(curl -sf -X POST "$COORDINATOR/v1/chat/completions" \
  -H "content-type: application/json" \
  -d "{
    \"model\": \"$MODEL\",
    \"messages\": [{\"role\": \"user\", \"content\": \"$PROMPT\"}],
    \"client_pubkey_hex\": \"$CLIENT_PUBKEY\"
  }")

REQUEST_ID=$(echo "$RESP" | jq -r .request_id)
NODE_URL=$(echo "$RESP" | jq -r .node_url)
TOKEN=$(echo "$RESP" | jq -c .dispatch_token)

echo "    request_id : $REQUEST_ID"
echo "    node_url   : $NODE_URL"

# ── Step 2: inference on node ─────────────────────────────────────────────────
echo "[2/4] POST $NODE_URL/v1/inference ..."
INFER=$(curl -sf -X POST "$NODE_URL/v1/inference" \
  -H "content-type: application/json" \
  -d "{\"prompt\": \"$PROMPT\", \"dispatch_token\": $TOKEN}")

echo "    response   : $(echo "$INFER" | jq -r .content | head -c 120)"

# ── Step 3: fetch receipt from coordinator ────────────────────────────────────
echo "[3/4] Waiting for CompletionAck + receipt (up to 10s) ..."
for i in $(seq 1 10); do
  RECEIPT=$(curl -sf "$COORDINATOR/v1/proofs/$REQUEST_ID" 2>/dev/null || true)
  if [ -n "$RECEIPT" ] && echo "$RECEIPT" | jq -e .request_id > /dev/null 2>&1; then
    break
  fi
  sleep 1
done

if [ -z "$RECEIPT" ]; then
  echo "    ERROR: no receipt after 10s"
  exit 1
fi

echo "    primary    : $(echo "$RECEIPT" | jq -r .primary_peer_id)"
echo "    output_hash: $(echo "$RECEIPT" | jq -r .aggregated_output_hash)"
echo "    payouts    : $(echo "$RECEIPT" | jq -c .payouts)"

# ── Step 4: indexer serves the receipt ───────────────────────────────────────
echo "[4/4] GET $INDEXER/api/r/$REQUEST_ID ..."
IDX=$(curl -sf "$INDEXER/api/r/$REQUEST_ID" 2>/dev/null || true)
if [ -z "$IDX" ]; then
  echo "    WARNING: indexer returned nothing (may not be running)"
else
  PAYMENT_STATUS=$(echo "$IDX" | jq -r '.payments[0].status // "no payments"')
  echo "    payment[0] status: $PAYMENT_STATUS"
fi

echo ""
echo "=== smoke PASSED ==="
echo "    explorer url: $INDEXER/api/r/$REQUEST_ID"
