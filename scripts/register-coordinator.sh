#!/usr/bin/env bash
# Update on-chain PCRs and wait for the enclave to self-register on Sui.
#
# Usage (on EC2, after a fresh enclave deploy):
#   source /tmp/sui_vars.env          # PINAIVU_PACKAGE_ID, CONFIG_ID, CAP_ID, SUI_NETWORK
#   ./scripts/register-coordinator.sh
#
# Required env vars:
#   PINAIVU_PACKAGE_ID          Published package address (0x...)
#   PINAIVU_ENCLAVE_CONFIG_ID   EnclaveConfig<ENCLAVE> shared object id
#   PINAIVU_CAP_ID              Cap<ENCLAVE> owned object id (held by operator)
#   SUI_NETWORK                 mainnet | testnet | devnet  (default: mainnet)
#
# Optional env vars:
#   PCR_FILE                    Path to coordinator.pcrs  (default: ~/pinaivu-coordinator/out/coordinator.pcrs)
#   COORDINATOR_URL             Health endpoint base URL  (default: http://localhost:4000)
#   REGISTER_TIMEOUT_S          Seconds to wait for self-registration (default: 300)

set -euo pipefail

COORDINATOR_URL="${COORDINATOR_URL:-http://localhost:4000}"
PCR_FILE="${PCR_FILE:-$HOME/pinaivu-coordinator/out/coordinator.pcrs}"
NETWORK="${SUI_NETWORK:-mainnet}"
TIMEOUT="${REGISTER_TIMEOUT_S:-300}"

# ── Validate required vars ────────────────────────────────────────────────────
for var in PINAIVU_PACKAGE_ID PINAIVU_ENCLAVE_CONFIG_ID PINAIVU_CAP_ID; do
    if [ -z "${!var:-}" ]; then
        echo "ERROR: $var is not set" >&2
        exit 1
    fi
done

if [ ! -f "$PCR_FILE" ]; then
    echo "ERROR: PCR file not found: $PCR_FILE" >&2
    echo "Run 'make eif' first, or set PCR_FILE to the correct path." >&2
    exit 1
fi

if ! command -v sui &> /dev/null; then
    echo "ERROR: sui CLI not found in PATH." >&2
    echo "Install it on the EC2 instance before running this script." >&2
    exit 1
fi

# ── Configure network ─────────────────────────────────────────────────────────
sui client switch --env "$NETWORK" 2>/dev/null || \
    sui client new-env --alias "$NETWORK" \
        --rpc "https://fullnode.${NETWORK}.sui.io" 2>/dev/null || true
echo "Active network: $NETWORK"

# ── Read PCRs ─────────────────────────────────────────────────────────────────
PCR0=$(jq -r '.PCR0' "$PCR_FILE")
PCR1=$(jq -r '.PCR1' "$PCR_FILE")
PCR2=$(jq -r '.PCR2' "$PCR_FILE")
echo "PCR0 = $PCR0"
echo "PCR1 = $PCR1"
echo "PCR2 = $PCR2"

# ── Update on-chain PCRs ──────────────────────────────────────────────────────
echo ""
echo "Calling pinaivu::enclave::update_pcrs..."
TX=$(sui client call \
    --package "$PINAIVU_PACKAGE_ID" \
    --module enclave \
    --function update_pcrs \
    --type-args "${PINAIVU_PACKAGE_ID}::enclave::ENCLAVE" \
    --args \
        "$PINAIVU_ENCLAVE_CONFIG_ID" \
        "$PINAIVU_CAP_ID" \
        "$PCR0" "$PCR1" "$PCR2" \
    --gas-budget 50000000 \
    --json)

DIGEST=$(printf '%s' "$TX" | jq -r '.digest')
STATUS=$(printf '%s' "$TX" | jq -r '.effects.status.status')
echo "update_pcrs tx: $DIGEST  status: $STATUS"
if [ "$STATUS" != "success" ]; then
    echo "ERROR: update_pcrs failed" >&2
    printf '%s' "$TX" | jq .effects >&2
    exit 1
fi

# ── Wait for enclave to self-register ────────────────────────────────────────
echo ""
echo "Waiting up to ${TIMEOUT}s for enclave to self-register on Sui..."
INTERVAL=10
ELAPSED=0
while [ "$ELAPSED" -lt "$TIMEOUT" ]; do
    HEALTH=$(curl -sf "${COORDINATOR_URL}/enclave_health" 2>/dev/null || true)
    OBJ=$(printf '%s' "$HEALTH" | jq -r '.enclave_object_id // empty' 2>/dev/null || true)
    if [ -n "$OBJ" ] && [ "$OBJ" != "null" ]; then
        SUI_TX=$(printf '%s' "$HEALTH" | jq -r '.sui_tx_digest // ""')
        PUBKEY=$(printf '%s' "$HEALTH" | jq -r '.public_key_hex // ""')
        echo ""
        echo "Enclave registered successfully!"
        echo "  enclave_object_id = $OBJ"
        echo "  sui_tx_digest     = $SUI_TX"
        echo "  coordinator_pubkey = $PUBKEY"
        echo ""
        echo "ENCLAVE_OBJECT_ID=$OBJ"
        exit 0
    fi
    echo "  ${ELAPSED}s elapsed — enclave_object_id still null, retrying in ${INTERVAL}s..."
    sleep "$INTERVAL"
    ELAPSED=$((ELAPSED + INTERVAL))
done

echo ""
echo "WARNING: enclave_object_id still null after ${TIMEOUT}s."
echo "The enclave's background registration task may still be retrying."
echo "Check coordinator logs: ssh ec2 'tail -f /tmp/coordinator.log'"
echo ""
# Exit 0 so the deploy isn't blocked — a pending registration is non-fatal
# (inference works; only vault settlements are delayed until it lands).
exit 0
