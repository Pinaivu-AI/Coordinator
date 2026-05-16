#!/usr/bin/env bash
# Run on the EC2 host alongside the enclave.
# Bridges VSOCK ↔ TCP for every service the coordinator needs.
#
# Usage:
#   ENCLAVE_CID=$(sudo nitro-cli describe-enclaves | jq -r '.[0].EnclaveCID')
#   ./parent_forwarder.sh
#
# Or source an env file first:
#   set -a; source .env.runtime; set +a
#   ./parent_forwarder.sh

set -euo pipefail

# ── Helpers ──────────────────────────────────────────────────────────────────
extract_host() { printf '%s' "$1" | sed -nE 's#^[^:]+://([^@]+@)?([^:/]+).*#\2#p'; }
extract_port() {
    local p
    p=$(printf '%s' "$1" | sed -nE 's#.*:([0-9]+)(/.*)?$#\1#p')
    printf '%s' "${p:-5432}"
}

# ── Discover enclave CID ──────────────────────────────────────────────────────
if [ -z "${ENCLAVE_CID:-}" ]; then
    ENCLAVE_CID=$(sudo nitro-cli describe-enclaves | jq -r '.[0].EnclaveCID // empty')
fi
if [ -z "${ENCLAVE_CID:-}" ]; then
    echo "ERROR: no running enclave found. Run: make run"
    exit 1
fi
echo "Enclave CID: ${ENCLAVE_CID}"

# ── Push config to enclave (VSOCK:7000) ──────────────────────────────────────
# The enclave init process listens on VSOCK:7000 for one connection.
# We send KEY=VALUE lines from ENV_FILE (default: .env.runtime) then close.
# This is the only moment secrets (DATABASE_URL, REDIS_URL, …) enter the enclave;
# they never appear in the EIF image or PCR measurements.
ENV_FILE="${ENV_FILE:-.env.runtime}"
if [ -f "${ENV_FILE}" ]; then
    echo "Pushing config from ${ENV_FILE} → VSOCK:${ENCLAVE_CID}:7000"
    socat - "VSOCK-CONNECT:${ENCLAVE_CID}:7000" < "${ENV_FILE}"
else
    echo "WARNING: ${ENV_FILE} not found — enclave will use built-in defaults"
fi

# ── Inbound: external TCP → enclave VSOCK ────────────────────────────────────
# HTTP API (clients → coordinator port 4000)
echo "TCP:4000 → VSOCK:${ENCLAVE_CID}:4000  (HTTP API)"
socat TCP-LISTEN:4000,reuseaddr,fork \
    VSOCK-CONNECT:"${ENCLAVE_CID}":4000 &

# libp2p marketplace mesh (nodes → coordinator port 4001)
echo "TCP:4001 → VSOCK:${ENCLAVE_CID}:4001  (libp2p)"
socat TCP-LISTEN:4001,reuseaddr,fork \
    VSOCK-CONNECT:"${ENCLAVE_CID}":4001 &

# ── Outbound: enclave VSOCK → external TCP ───────────────────────────────────
# Postgres — enclave reaches VSOCK:8101, host forwards to Postgres
if [ -n "${DATABASE_URL:-}" ]; then
    PG_HOST=$(extract_host "${DATABASE_URL}")
    PG_PORT=$(extract_port "${DATABASE_URL}")
    echo "VSOCK:8101 → ${PG_HOST}:${PG_PORT}  (Postgres)"
    socat VSOCK-LISTEN:8101,reuseaddr,fork \
        TCP:"${PG_HOST}":"${PG_PORT}" &
fi

# Redis — enclave reaches VSOCK:8102, host forwards to Redis
if [ -n "${REDIS_URL:-}" ]; then
    REDIS_HOST=$(extract_host "${REDIS_URL}")
    REDIS_PORT=$(extract_port "${REDIS_URL}")
    echo "VSOCK:8102 → ${REDIS_HOST}:${REDIS_PORT}  (Redis)"
    socat VSOCK-LISTEN:8102,reuseaddr,fork \
        TCP:"${REDIS_HOST}":"${REDIS_PORT}" &
fi

# ── Log collection ────────────────────────────────────────────────────────────
echo "VSOCK:5000 → enclave.log  (coordinator logs)"
socat VSOCK-LISTEN:5000,reuseaddr,fork \
    OPEN:enclave.log,creat,append &

echo ""
echo "All bridges active."
echo "Test: curl http://localhost:4000/health"
echo "Logs: tail -f enclave.log"
echo ""

wait
