# Coordinator

Pinaivu coordinator — a Nautilus-style AWS Nitro Enclave component that
brokers private LLM inference between clients and an open mesh of GPU
node operators. It runs the libp2p auction, issues signed dispatch
tokens, verifies job completion, and settles payouts on Sui. Part of
[Pinaivu](https://pinaivu.com)'s decentralized inference network.

## Where this fits

The coordinator is the off-chain-but-verifiable piece of Pinaivu's trust
model: a single instance today, but its code runs inside an attested
Nitro Enclave and every artefact it signs is checkable against an
on-chain record it cannot quietly alter. It is not the decentralized
layer (that's the libp2p GPU mesh and Walrus storage) and it is not the
on-chain layer (that's the Sui Move contracts in
[`Pinaivu-AI/contracts`](https://github.com/Pinaivu-AI/contracts)). See
the [decentralization & verifiability
model](https://docs.pinaivu.com/architecture/decentralization) for the
full breakdown, and [why Sui](https://docs.pinaivu.com/architecture/why-sui)
for why the on-chain layer is Move + Nautilus + Walrus specifically.

The coordinator is never in the response data path. It signs
attestations, dispatch tokens, and routing receipts; the actual
inference response streams directly from the winning node to the
client.

```
client (1) ── POST /v1/chat/completions ─▶ coordinator (Nitro Enclave)
client ◀──── { node_url, dispatch_token, session_id } ──┤
client (2) ── HTTPS w/ dispatch_token ─────▶ node_1
node_1 ◀── recruits node_2 over libp2p ───▶ node_2
node_1 ── streams response direct ─▶ client
node_1 ── completion ack + proofs ─▶ coordinator
coordinator ── settle() PTB ─▶ Sui vault contract
```

## Crate layout

| Path | Role |
|---|---|
| `src/coordinator/` | Main binary + library — axum HTTP, marketplace, persistence, on-chain registration |
| `src/pinaivu-protocol/` | Shared wire types (`ProofOfInference`, `DispatchToken`, `RoutingReceipt`, mesh behaviour/topics) — also depended on by the node |
| `src/nautilus-enclave/` | Enclave keypair + NSM attestation (mock by default, real behind `--features aws`) |
| `src/aws/` | NSM ioctl + platform init |
| `src/init/` | Nitro Enclave init process, spawns the TS sidecar that holds the Sui operator key |
| `src/system/` | Low-level system utilities |

Inside `src/coordinator/src/` modules are grouped by concern: `api/`,
`app/`, `jobs/`, `marketplace/`, `mesh/`, `observability/`, `onchain/`,
`payments/`, `persistence/`, `receipts/`, `reputation/`, `settlement/`.

## What this does

- Boots, generates an Ed25519 enclave key, binds an axum HTTP server
  (TCP for dev, VSOCK for prod) with graceful shutdown.
- Runs a real libp2p mesh (gossipsub auctions, request-response for
  dispatch and completion acks) — not a mock. Nodes bid in an open
  200ms window; the coordinator scores bids on price, latency,
  reputation, and Walrus-backed session cache warmth, then dispatches
  to the winner.
- Tracks every accepted job as an apalis job backed by Postgres, with
  a deadline watcher that triggers refunds on timeout.
- Verifies completion acks (primary + any helper proofs), writes a
  signed `RoutingReceipt` to Postgres, and computes per-node payouts
  from the proof set.
- Registers its NSM attestation on Sui at startup via an in-enclave
  TS sidecar holding the operator key, so its attested pubkey is
  checkable on-chain (`pinaivu::enclave::register_enclave`).
- Settles payouts by submitting `vault::settle()` PTBs signed against
  the registered enclave key — the operator key only pays gas, it
  cannot authorize a payout on its own.
- Exposes routing receipts and session debug info via `GET
  /v1/proofs/{request_id}` and `GET /v1/admin/sessions/{session_id}`.

## Endpoints

| Endpoint | Purpose |
|---|---|
| `POST /v1/chat/completions` | Runs the auction, returns `{ request_id, node_url, dispatch_token, session_id }` |
| `GET /v1/proofs/{request_id}` | Signed routing receipt + bundled proofs |
| `GET /v1/nodes` | Current peer registry snapshot |
| `GET /v1/admin/sessions/{session_id}` | Session turns + warm-node cache rows (debug) |
| `GET /health`, `GET /metrics` | Liveness + Prometheus |
| `GET /enclave_health` | Coordinator pubkey, uptime, registered Sui enclave object id |
| `GET /get_attestation` | NSM attestation document |

## Running

Local dev (no enclave, mock NSM) requires `DATABASE_URL` and
`REDIS_URL` — there is no in-memory fallback:

```bash
DATABASE_URL=postgres://... REDIS_URL=redis://... cargo run -p coordinator
```

By default binds `127.0.0.1:4000`. Override with `PINAIVU_BIND`.

```bash
curl http://127.0.0.1:4000/health
curl http://127.0.0.1:4000/enclave_health | jq
curl http://127.0.0.1:4000/get_attestation | jq
```

## Tests

```bash
cargo test --workspace
```

## Building the enclave image

```bash
make eif
```

Produces `coordinator.eif` + `coordinator.pcrs` via a stagex
multi-stage build. Rebuilding from the same source produces identical
PCRs (reproducible build), which is what `pinaivu::enclave::update_pcrs`
checks against on-chain before registration succeeds.

## Parent-host forwarders

`parent_forwarder.sh` runs on the EC2 host (not inside the enclave) and
bridges inbound client traffic (`TCP:443 ↔ VSOCK:4000`) and outbound
traffic to Postgres, Redis, libp2p bootstrap peers, and Sui RPC.

## Layout convention

Every module folder owns one concern. Public types are re-exported from
the folder's `mod.rs` so callers write `crate::protocol::ProofOfInference`
rather than `crate::protocol::proof::ProofOfInference`. No loose source
files at the root of `src/coordinator/src/`.
