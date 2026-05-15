#!/usr/bin/env bash
# Parent-host socat forwarders. Runs on the EC2 host alongside the
# enclave. Bridges the enclave's VSOCK endpoints to/from TCP services
# the enclave needs but cannot reach directly.
#
# Skeleton — port numbers and service list finalised as services come
# online. Run as a systemd unit on the host in production.

set -euo pipefail

ENCLAVE_CID="${ENCLAVE_CID:-16}"

# Inbound: HTTPS clients → enclave HTTP API on VSOCK:4000
# socat TCP-LISTEN:4000,reuseaddr,fork VSOCK-CONNECT:${ENCLAVE_CID}:4000 &

# Outbound (enclave VSOCK → external TCP), assigned VSOCK ports:
#   8101 → Postgres
#   8102 → Redis
#   8103 → libp2p bootstrap peer
#   8104 → settlement RPC (Sui)
#   8105 → settlement RPC (EVM)
#
# socat VSOCK-LISTEN:8101,reuseaddr,fork TCP:postgres.internal:5432 &
# socat VSOCK-LISTEN:8102,reuseaddr,fork TCP:redis.internal:6379 &
# ...

echo "parent_forwarder skeleton — not yet wired"
wait
