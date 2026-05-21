#!/usr/bin/env bash
# Update on-chain PCRs and register this enclave on Sui.
#
# Runs on the EC2 host (not inside the enclave). Uses the sui CLI for
# both calls so we don't depend on the in-enclave sidecar to register
# itself — that path was opaque to deploy-time diagnostics.
#
# Usage:
#   source ~/.env.runtime
#   ./scripts/register-coordinator.sh
#
# Required env vars (loaded from ~/.env.runtime):
#   PINAIVU_PACKAGE_ID
#   PINAIVU_ENCLAVE_CONFIG_ID
#   PINAIVU_CAP_ID
#   SUI_NETWORK                 mainnet | testnet | devnet
#
# Optional:
#   PCR_FILE                    default: ~/pinaivu-coordinator/out/coordinator.pcrs
#   COORDINATOR_URL             default: http://localhost:4000
#   GAS_BUDGET                  default: 200000000

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
COORDINATOR_URL="${COORDINATOR_URL:-http://localhost:4000}"
PCR_FILE="${PCR_FILE:-$HOME/pinaivu-coordinator/out/coordinator.pcrs}"
NETWORK="${SUI_NETWORK:-mainnet}"
GAS_BUDGET="${GAS_BUDGET:-200000000}"

# ── Validate ────────────────────────────────────────────────────────────────
for var in PINAIVU_PACKAGE_ID PINAIVU_ENCLAVE_CONFIG_ID PINAIVU_CAP_ID; do
    if [ -z "${!var:-}" ]; then
        echo "ERROR: $var is not set" >&2
        exit 1
    fi
done

if [ ! -f "$PCR_FILE" ]; then
    echo "ERROR: PCR file not found: $PCR_FILE" >&2
    exit 1
fi

for cmd in sui jq curl node; do
    if ! command -v "$cmd" &>/dev/null; then
        echo "ERROR: $cmd not found in PATH" >&2
        exit 1
    fi
done

# ── Pick network ────────────────────────────────────────────────────────────
sui client switch --env "$NETWORK" 2>/dev/null || \
    sui client new-env --alias "$NETWORK" \
        --rpc "https://fullnode.${NETWORK}.sui.io" 2>/dev/null || true
echo "Active network: $NETWORK"

# ── Read PCRs (plain text format from eif_build) ────────────────────────────
PCR0=$(awk '$2=="PCR0"{print $1}' "$PCR_FILE")
PCR1=$(awk '$2=="PCR1"{print $1}' "$PCR_FILE")
PCR2=$(awk '$2=="PCR2"{print $1}' "$PCR_FILE")
if [ -z "$PCR0" ] || [ -z "$PCR1" ] || [ -z "$PCR2" ]; then
    echo "ERROR: failed to parse PCRs from $PCR_FILE" >&2
    head -5 "$PCR_FILE" >&2
    exit 1
fi
echo "PCR0 = $PCR0"
echo "PCR1 = $PCR1"
echo "PCR2 = $PCR2"

# ── update_pcrs ─────────────────────────────────────────────────────────────
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
        "0x$PCR0" "0x$PCR1" "0x$PCR2" \
    --gas-budget "$GAS_BUDGET" \
    --json)
DIGEST=$(printf '%s' "$TX" | jq -r '.digest')
STATUS=$(printf '%s' "$TX" | jq -r '.effects.status.status')
echo "update_pcrs tx: $DIGEST  status: $STATUS"
if [ "$STATUS" != "success" ]; then
    echo "ERROR: update_pcrs failed" >&2
    printf '%s' "$TX" | jq .effects >&2
    exit 1
fi

# ── Fetch the live attestation from the coordinator ────────────────────────
echo ""
echo "Fetching NSM attestation from ${COORDINATOR_URL}/get_attestation..."
ATT_RESP=$(curl -sf --connect-timeout 5 "${COORDINATOR_URL}/get_attestation")
ATT_HEX=$(printf '%s' "$ATT_RESP" | jq -r '.raw_cbor_hex // empty')
if [ -z "$ATT_HEX" ] || [ "$ATT_HEX" = "null" ]; then
    echo "ERROR: /get_attestation did not return raw_cbor_hex" >&2
    printf '%s' "$ATT_RESP" | head -c 500 >&2
    exit 1
fi
echo "Got attestation (${#ATT_HEX} hex chars)"

ATT_VEC=$(node "$SCRIPT_DIR/register/hex-to-vector.mjs" "$ATT_HEX")
echo "Encoded to PTB vector ($((${#ATT_VEC} / 1024)) KB)"

# ── register_enclave (PTB: load_nitro_attestation + register_enclave) ──────
echo ""
echo "Calling pinaivu::enclave::register_enclave..."
REG_TX=$(sui client ptb \
    --assign v "vector$ATT_VEC" \
    --move-call "0x2::nitro_attestation::load_nitro_attestation" v @0x6 \
    --assign doc \
    --move-call "${PINAIVU_PACKAGE_ID}::enclave::register_enclave<${PINAIVU_PACKAGE_ID}::enclave::ENCLAVE>" \
        @"$PINAIVU_ENCLAVE_CONFIG_ID" \
        @"$PINAIVU_CAP_ID" \
        doc \
    --gas-budget "$GAS_BUDGET" \
    --json)

REG_DIGEST=$(printf '%s' "$REG_TX" | jq -r '.digest')
REG_STATUS=$(printf '%s' "$REG_TX" | jq -r '.effects.status.status')
echo "register_enclave tx: $REG_DIGEST  status: $REG_STATUS"
if [ "$REG_STATUS" != "success" ]; then
    echo "ERROR: register_enclave failed" >&2
    printf '%s' "$REG_TX" | jq .effects >&2
    exit 1
fi

ENCLAVE_OBJECT_ID=$(printf '%s' "$REG_TX" | jq -r \
    '.objectChanges[]? | select(.type=="created" and (.objectType // "" | contains("::enclave::Enclave"))) | .objectId' | head -1)

if [ -z "$ENCLAVE_OBJECT_ID" ] || [ "$ENCLAVE_OBJECT_ID" = "null" ]; then
    echo "WARNING: enclave object id not found in tx output; check Sui explorer" >&2
else
    echo ""
    echo "Enclave registered:"
    echo "  enclave_object_id = $ENCLAVE_OBJECT_ID"
    echo "  tx_digest         = $REG_DIGEST"
    echo "ENCLAVE_OBJECT_ID=$ENCLAVE_OBJECT_ID"

    # Persist the id so the next enclave boot pushes it to the sidecar via
    # VSOCK:7000. ~/.env.runtime is rewritten from the GH secret each
    # deploy, so we use a sibling file that the deploy concatenates at
    # config-push time.
    DYNAMIC_ENV="$HOME/.env.runtime.dynamic"
    touch "$DYNAMIC_ENV"
    chmod 600 "$DYNAMIC_ENV"
    if grep -q '^PINAIVU_ENCLAVE_OBJECT_ID=' "$DYNAMIC_ENV"; then
        sed -i "s|^PINAIVU_ENCLAVE_OBJECT_ID=.*|PINAIVU_ENCLAVE_OBJECT_ID=$ENCLAVE_OBJECT_ID|" "$DYNAMIC_ENV"
    else
        echo "PINAIVU_ENCLAVE_OBJECT_ID=$ENCLAVE_OBJECT_ID" >> "$DYNAMIC_ENV"
    fi
    echo "persisted to $DYNAMIC_ENV — next deploy's sidecar will pick it up on boot"

    # Push the id into the running sidecar via the coordinator's admin
    # proxy so the current deploy can settle without waiting for a reboot.
    if [ -n "${SIDECAR_SECRET:-}" ]; then
        echo ""
        echo "Pushing enclave_object_id to running coordinator..."
        PUSH_RESP=$(curl -sf -X POST "${COORDINATOR_URL}/v1/admin/set-enclave-id" \
            -H 'content-type: application/json' \
            -H "x-sidecar-secret: ${SIDECAR_SECRET}" \
            -d "{\"enclave_object_id\":\"$ENCLAVE_OBJECT_ID\",\"tx_digest\":\"$REG_DIGEST\"}" \
            || true)
        if [ -n "$PUSH_RESP" ]; then
            echo "  response: $PUSH_RESP"
        else
            echo "  WARNING: admin push failed; sidecar will pick up id on next boot only" >&2
        fi
    else
        echo "SIDECAR_SECRET not set — skipping live admin push (next boot will pick up id)"
    fi
fi
