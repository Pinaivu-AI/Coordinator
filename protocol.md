# Protocol

The `pinaivu-protocol` crate (`coordinator/src/pinaivu-protocol/`)
defines every type that goes over the wire — both gossipsub broadcasts
and request-response messages — plus signing helpers. Both the
coordinator and the node depend on it; the node's git dep points at
`Pinaivu-AI/Coordinator` with `package = "pinaivu-protocol"` so
providers don't need to clone the coordinator code.

## Types

| Type | Where it travels | Signed by |
|---|---|---|
| `InferenceRequest` | gossipsub `/pinaivu/inference/any/1.0.0` | none (client identity carried in body) |
| `InferenceBid` | gossipsub `/pinaivu/bids/1.0.0` | none in v1 (libp2p mesh authenticates the sender) |
| `DispatchToken` | HTTP body (coordinator → client → node) | coordinator's enclave key |
| `ProofOfInference` | nested inside `CompletionAck` | each contributing node's key |
| `CompletionAck` | libp2p request-response `/pinaivu/completion/1.0.0` | primary node's key (covers the proof set) |
| `RecruitRequest` / `RecruitResponse` | libp2p request-response `/pinaivu/recruit/1.0.0` | primary's signed request; helper returns a signed proof |
| `RoutingReceipt` | stored in Postgres, served via `GET /v1/proofs/{id}` | coordinator's enclave key |

## Signing format

All signatures are Ed25519. Most types use serde-JSON canonical bytes
as the signed message (`canonical_bytes()` on each type).

**`RoutingReceipt` is different** — it's signed as BCS-encoded
`IntentMessage`:

```
sign(  bcs(IntentMessage {
         intent:        1,             // INTENT_ROUTING_RECEIPT
         timestamp_ms:  u64,
         payload:       ReceiptPayload {
            request_id:             vector<u8>,     // 16-byte UUID
            aggregated_output_hash: vector<u8>,     // 32-byte SHA-256
            payouts:                vector<Payout>, // {sui_address: address, amount: u64}
         },
       })
    )
```

This shape matches the on-chain `pinaivu::receipts::ReceiptPayload`
exactly so `enclave::verify_signature` accepts it.

### v1 limitation

The receipt signature only covers the **settlement subset** —
`(request_id, aggregated_output_hash, payouts)`. The receipt struct
also carries descriptive metadata (`client_id`, `primary_peer_id`,
`helper_peer_ids`, `proof_ids`, `bid_set_hash`) that the off-chain
explorer renders, but those fields are **not cryptographically
committed** in v1. A future tighter signature can add a
`full_receipt_hash` covered by the same payload.

## Topic names

```
gossipsub:
  /pinaivu/inference/any/1.0.0   broadcast inference requests
  /pinaivu/bids/1.0.0            broadcast bids
  /pinaivu/announce/1.0.0        (planned) periodic NodeCapabilities
  /pinaivu/reputation/1.0.0      (planned) reputation roots

request-response:
  /pinaivu/completion/1.0.0      node → coordinator: CompletionAck
  /pinaivu/recruit/1.0.0         node → node:        RecruitRequest

kademlia:
  /pinaivu/kad/1.0.0             DHT (isolated from public libp2p DHT)
```

## libp2p behaviour composition

`pinaivu_protocol::mesh::PinaivuBehaviour` is a single
`#[derive(NetworkBehaviour)]` struct used by both the coordinator and
every node so peer-id derivation, gossipsub mesh formation, and
request-response routing are identical on both sides.

```
PinaivuBehaviour {
    gossipsub:  pub/sub for marketplace topics
    kademlia:   peer routing
    identify:   protocol negotiation + observed addrs
    ping:       liveness
    completion: request_response::cbor for CompletionAck
    recruit:    request_response::cbor for RecruitRequest
}
```
