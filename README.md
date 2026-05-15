# Coordinator

Pinaivu coordinator — a Nautilus-style AWS Nitro Enclave component that
runs the libp2p inference auction on behalf of clients, issues signed
dispatch tokens, and tracks job completion. Part of the
[Pinaivu](https://pinaivu.ai) decentralised AI inference network.

## Status

Early development. The control plane is wired end to end against an
in-process mock mesh: the coordinator boots, generates an enclave
keypair, serves attestation, runs an auction, signs a verifiable
dispatch token, and returns it to the client. The libp2p mesh, apalis
worker, settlement integration, and Nitro EIF build are next.

## Architecture

```
client (1) ── POST /v1/chat/completions ─▶ coordinator (Nitro Enclave)
client ◀──── { node_url, dispatch_token } ──┤  + parent socat forwarders
client (2) ── HTTPS w/ dispatch_token ─────▶ node_1
node_1 ◀── recruits node_2 over libp2p ───▶ node_2
node_1 ── streams response direct ─▶ client
node_1 ── completion ack + proofs ─▶ coordinator
```

The coordinator is never in the response data path. It signs
attestations, dispatch tokens, and routing receipts; everything else
flows peer-to-peer. See
`/home/ash/pinaivu_ai_items/architecture-overview.md` for the full
trust model and helper-recruitment details.

## Crate layout

| Path | Role |
|---|---|
| `src/coordinator/` | Main binary + library — axum HTTP, marketplace, persistence wiring |
| `src/nautilus-enclave/` | Enclave keypair + NSM attestation (mock by default, real behind `--features aws`) |
| `src/aws/` | NSM ioctl + platform init |
| `src/init/` | Nitro Enclave init process |
| `src/system/` | Low-level system utilities |

Inside `src/coordinator/src/` modules are grouped by concern:
`app/`, `observability/`, `protocol/`, `mesh/`, `marketplace/`,
`reputation/`, `settlement/`, `jobs/`, `persistence/`, `api/`.

## What works today

- Coordinator boots, generates an Ed25519 enclave key, binds an axum
  HTTP server (TCP for dev, VSOCK for prod) with graceful shutdown.
- Endpoints:
  - `GET  /health` — liveness
  - `GET  /enclave_health` — `{ public_key_hex, uptime_ms }`
  - `GET  /get_attestation` — NSM `AttestationDoc` (mock impl binds
    pubkey to PCRs; real impl behind the `aws` feature)
  - `POST /v1/chat/completions` — runs the auction, signs a
    `DispatchToken`, returns `{ request_id, node_url, dispatch_token }`
- Protocol artefacts (`ProofOfInference`, `DispatchToken`,
  `RoutingReceipt`) have real canonical-bytes / sign / verify with
  Ed25519. Every field is signature-covered; tamper-on-any-field is
  detected.
- Auction engine: 200 ms bid-collection window, composite
  price/latency/reputation scoring per whitepaper §12.3, tie-break on
  lowest price.
- `Mesh` trait abstracts the marketplace network so the auction runs
  against an `InMemoryMesh` in tests and (soon) a libp2p swarm in
  production.

## What's stubbed

- libp2p `Swarm` is not yet constructed (only the `Mesh` trait + mock).
- Apalis worker, Postgres + Redis persistence, settlement-adapter
  invocation, reputation gossip, multi-helper proof aggregation are
  all module-shaped but bodies are `TODO`.
- `nautilus-enclave/nsm` `aws`-feature path is `unimplemented!()`;
  mock path returns deterministic PCRs for dev.
- `Containerfile` is a skeleton — does not yet produce a real `.eif`.

## Running

Local dev (no enclave, mock NSM):

```bash
cargo run -p coordinator
```

By default binds `127.0.0.1:4000`. Override with `PINAIVU_BIND`:

```bash
PINAIVU_BIND=0.0.0.0:8080 cargo run -p coordinator
```

Probe the endpoints:

```bash
curl http://127.0.0.1:4000/health
# ok

curl http://127.0.0.1:4000/enclave_health | jq
# { "public_key_hex": "...", "uptime_ms": 1234 }

curl http://127.0.0.1:4000/get_attestation | jq
# { "pcr0": "...", "pcr1": "...", "pcr2": "...",
#   "public_key": "...", "timestamp_ms": 0, "raw_cbor_hex": "" }
```

## Tests

```bash
cargo test --workspace
```

| Suite | Count |
|---|---|
| Protocol crypto (sign / verify / tamper) | 16 |
| Marketplace auction (scoring + window) | 4 |
| nautilus-enclave (keypair + NSM mock) | 4 |
| HTTP smoke (health + attestation) | 3 |
| Auction integration (mesh → token → verify) | 3 |

## Building the enclave image

Not yet wired. Final command will be `make eif`; produces
`coordinator.eif` + `coordinator.pcrs` via a stagex multi-stage build
declared in `Containerfile`.

## Parent-host forwarders

`parent_forwarder.sh` runs on the EC2 host (not inside the enclave)
and bridges:

- Inbound: `TCP:443 ↔ VSOCK:4000` (client API)
- Outbound: VSOCK → Postgres / Redis / libp2p bootstrap peers / RPCs

Skeleton only at this point.

## Layout convention

Every module folder owns one concern. Public types are re-exported
from the folder's `mod.rs` so callers write
`crate::protocol::ProofOfInference` rather than
`crate::protocol::proof::ProofOfInference`. No loose source files at
the root of `src/coordinator/src/`.
