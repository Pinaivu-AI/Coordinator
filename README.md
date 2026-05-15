# Coordinator

Pinaivu coordinator — a Nautilus-style AWS Nitro Enclave component that runs
the libp2p inference auction on behalf of clients, issues signed dispatch
tokens, and tracks job completion. Part of the [Pinaivu](https://pinaivu.ai)
decentralised AI inference network.

Status: **skeletal scaffold**. Not yet functional.

## Architecture

```
client (1) ── POST /v1/chat/completions ──▶  coordinator (Nitro Enclave)
client ◀───── { node_url, dispatch_token } ──┤  + parent socat forwarders
client (2) ── HTTPS w/ dispatch_token ──▶  node_1
node_1 ◀── recruits node_2 over libp2p ─▶  node_2
node_1 ── streams response direct ──▶  client
node_1 ── completion ack + proofs ──▶  coordinator
```

The coordinator is **not** in the response data path. It only sees the
auction, the dispatch decision, and the completion ack.

## Crate layout

| Path | Role |
|---|---|
| `src/coordinator/` | Main binary — axum HTTP + libp2p mesh + apalis monitor |
| `src/init/` | Nitro Enclave init process |
| `src/aws/` | NSM driver bindings + platform init |
| `src/system/` | Low-level system utilities |
| `src/nautilus-enclave/` | NSM attestation + crypto primitives |

## Build

```bash
cargo check                # local dev (no enclave)
make eif                   # reproducible enclave image (coming soon)
```
