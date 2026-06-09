# Architecture

Pinaivu is a peer-to-peer decentralised AI inference marketplace.
Clients send chat completions to a TEE-attested **coordinator** which
auctions the job to GPU **nodes** on a libp2p mesh, returns a dispatch
token, and audits the result via a signed routing receipt that
disburses payment from an on-chain **vault**.

## Components

```
                                ┌─────────────────────────────────────┐
                                │  Sui mainnet                        │
                                │  pinaivu::{enclave, receipts, vault}│
                                └────────────┬───────┬────────────────┘
                                             │       │
                              register/      │       │  settle()
                              update_pcrs    │       │  (sig must verify
                                             │       │   under registered
                                             │       │   enclave key)
                  ┌──────────────────────────┴───────┴───────────────────┐
                  │  EC2 host                                            │
                  │  ┌──────────────────────────────────────────────┐   │
                  │  │  Nitro Enclave  (NSM-attested)               │   │
                  │  │  ┌────────────────┐    HTTP    ┌─────────┐  │   │
                  │  │  │ coordinator    │◀──loopback▶│ sidecar │  │   │
                  │  │  │ (Rust)         │            │ (TS)    │  │   │
                  │  │  │  - HTTP API    │            │         │  │   │
                  │  │  │  - libp2p mesh │            │ holds   │  │   │
                  │  │  │  - apalis      │            │ operator│  │   │
                  │  │  │  - signs       │            │ priv key│  │   │
                  │  │  │    receipts    │            │ signs + │  │   │
                  │  │  └────────────────┘            │ submits │  │   │
                  │  │                                │ PTBs    │  │   │
                  │  │                                └─────────┘  │   │
                  │  └────────┬────────────────────┬─────────────────┘   │
                  │           │ VSOCK              │ VSOCK              │
                  │  ┌────────┴────────┐  ┌────────┴────────┐           │
                  │  │ socat bridges   │  │ socat bridges   │           │
                  │  │ (host systemd)  │  │ outbound        │           │
                  │  │ TCP:4000→VSOCK  │  │ VSOCK→Postgres  │           │
                  │  │ TCP:4001→VSOCK  │  │ VSOCK→Redis     │           │
                  │  │ VSOCK→logs file │  │ VSOCK→Sui RPC*  │           │
                  │  └─────────────────┘  └─────────────────┘           │
                  └──────────────┬─────────────────┬───────────────────┘
                                 │                 │
                                 ▼                 ▼
                       libp2p mesh (gossipsub +  Supabase Postgres
                       request-response)         Upstash Redis
                       │                         (receipts, jobs, payments)
                       │
            ┌──────────┴──────────┐
            │                     │
        ┌───┴───┐            ┌────┴────┐
        │ node  │            │  node   │   each runs Ollama,
        │ (Rust)│            │  (Rust) │   bids on inference
        └───────┘            └─────────┘   requests, sends signed
                                            CompletionAck back
```

`*` Sui RPC reach from the sidecar is currently direct from inside the
enclave to public endpoints via the parent's TCP bridge.

## Request lifecycle

```
1. client → coordinator      POST /v1/chat/completions
                              { model, messages, client_pubkey_hex }
2. coordinator               publishes InferenceRequest on gossipsub
                              "/pinaivu/inference/any/1.0.0"
3. nodes                     receive request, bid via "/pinaivu/bids/1.0.0"
                              InferenceBid { peer_id, price_per_1k,
                                             latency, reputation,
                                             http_endpoint,
                                             payout_address }
4. coordinator               ranks bids, picks winner, signs DispatchToken
5. coordinator → client      { request_id, node_url, dispatch_token }
6. client → node             POST {node_url}/v1/inference
                              { prompt, dispatch_token }
7. node                      verifies dispatch_token, runs Ollama,
                              returns streaming response to client,
                              builds + signs ProofOfInference
8. node → coordinator        libp2p request-response on
                              "/pinaivu/completion/1.0.0"
                              CompletionAck { request_id, proofs[],
                                              aggregated_output_hash,
                                              signature }
9. coordinator               verifies primary sig + every embedded proof,
                              signs RoutingReceipt (BCS IntentMessage),
                              stores in Postgres
10. apalis worker            (later) computes payouts from proofs,
                              asks sidecar to submit vault::settle PTB(s)
11. client → coordinator     GET /v1/proofs/{request_id} → receipt
```

## Trust model

| Component | What it proves |
|---|---|
| **NSM attestation** | The coordinator binary running in the enclave matches the published `coordinator.pcrs`. Document is signed by AWS' Nitro root. |
| **`enclave::register_enclave`** | The Ed25519 key embedded in the NSM document is now the on-chain "Pinaivu coordinator" key. Anyone can verify a receipt signature against this key. |
| **`RoutingReceipt` signature** | The coordinator (running attested code) authorised payments to these `(payee, amount)` pairs for this `request_id`. |
| **`vault::settle`** | Treasury funds can only move when a coordinator signature for the exact `(payee, amount)` is presented. Sui RPC enforces this on-chain. |
| **`ProofOfInference` signature** | A specific node (identified by Ed25519 pubkey ↔ libp2p PeerId) processed this prompt chunk and produced this output. Self-verifiable offline. |

## Out-of-scope today

- Multi-node recruitment (`/pinaivu/recruit/1.0.0` protocol shipped, orchestration engine handled separately).
- Tensor / pipeline parallelism, speculative decoding.
- Reputation gossip authoring (coordinator consumes; nodes author later).
- Streaming partial outputs across the recruit channel.
