# End-to-end smoke test

The first fully working E2E run (client → auction → node → inference →
completion ack → routing receipt → vault settlement on Sui testnet)
landed at `tx 5ePLsmVqcCAzFmpLz9XAQaKpsDFWbjWHJtSfkNoLUwMe`.

This doc walks through reproducing it.

## Prerequisites

| Piece | Where | Required |
|---|---|---|
| Coordinator running in Nitro Enclave on EC2 | `13.206.80.190:4000` | `curl /health` returns `ok` |
| `/enclave_health` returns non-null `enclave_object_id` | same | means `register_enclave` already landed on-chain |
| Vault on Sui testnet has positive `treasury_balance<SUI>` | `pinaivu::vault` | `top_up` was called once after publish |
| Local Ollama running | `127.0.0.1:11434` | `curl /api/tags` lists `llama3.2:1b` (or similar) |
| Local `pinaivu-node` built | `~/projects/pinaivu/node/target/release/pinaivu-node` | `cargo build --release` |

## Step 1 — get the live coordinator peer_id

```bash
curl -s http://13.206.80.190:4000/enclave_health | jq -r .peer_id
# e.g. 12D3KooWMSsNbsLWeLjdNRdFAhyGd488HGTqQpkQGREvUN7GN5zG
```

Every deploy generates a fresh enclave key → fresh peer_id, so a stale
value from yesterday won't dial through.

## Step 2 — start the node

```bash
PEER=$(curl -s http://13.206.80.190:4000/enclave_health | jq -r .peer_id)
pkill -f pinaivu-node 2>/dev/null

nohup ~/projects/pinaivu/node/target/release/pinaivu-node \
  --coordinator-addr  /ip4/13.206.80.190/tcp/4001/p2p/$PEER \
  --coordinator-http  http://13.206.80.190:4000 \
  --listen            127.0.0.1:5000 \
  --advertise-url     http://127.0.0.1:5000 \
  --model             llama3.2:1b \
  --payout-address    0x5325e6c12ea21fde28bbdec080614b3d9d18064aba3a4ff13d4001075f2245d2 \
  > /tmp/pinaivu-node.log 2>&1 &
disown

# Wait for "connection established peer=<peer_id>" in /tmp/pinaivu-node.log
tail -5 /tmp/pinaivu-node.log
```

`--advertise-url 127.0.0.1:5000` is fine when the client is on the
same machine. For a separate client, expose 5000 with ngrok / a public
IP and pass that URL here so the coordinator's response gives the
client a reachable `node_url`.

## Step 3 — client submits the request

```bash
RESP=$(curl -s -X POST http://13.206.80.190:4000/v1/chat/completions \
  -H 'content-type: application/json' \
  -d '{
    "model": "llama3.2:1b",
    "messages": [{"role":"user","content":"say hi"}],
    "client_pubkey_hex": "'$(printf '01%.0s' {1..32})'"
  }')
echo "$RESP" | jq '{request_id, node_url}'

REQ=$(echo "$RESP" | jq -r .request_id)
TOKEN=$(echo "$RESP" | jq -c .dispatch_token)
NODE_URL=$(echo "$RESP" | jq -r .node_url)
```

What this exercises:
- coordinator publishes `InferenceRequest` on the
  `/pinaivu/inference/any/1.0.0` gossipsub topic
- the dialed node receives + bids on `/pinaivu/bids/1.0.0`
- coordinator runs the 200 ms auction window, picks our node, signs the
  `DispatchToken` with its enclave key

## Step 4 — client posts the prompt to the node

```bash
curl -s -X POST "$NODE_URL/v1/inference" \
  -H 'content-type: application/json' \
  -d "{\"prompt\":\"say hi\",\"dispatch_token\":$TOKEN}" | jq .
```

Expected:

```json
{
  "request_id": "<uuid>",
  "content": "Hi. How can I assist you today?",
  "output_tokens": 8,
  "latency_ms": 1093
}
```

What just happened:
- node verifies the dispatch token's Ed25519 signature against the
  cached `coordinator_pubkey` (fetched from `/enclave_health` at node
  startup)
- node calls Ollama, captures the reply
- node builds a `ProofOfInference`, signs it with its own libp2p key,
  wraps it in a `CompletionAck`, signs that with the same key, sends
  to coordinator on `/pinaivu/completion/1.0.0`
- coordinator verifies all signatures, signs a `RoutingReceipt` (BCS
  IntentMessage), stores it in `routing_receipts`, inserts a row into
  `payments` (status `pending`), and enqueues a `SettlementJob`
  in apalis

## Step 5 — wait for settlement

```bash
sleep 12   # apalis poll + sidecar PTB + Sui finality

SECRET=$(grep -E '^SIDECAR_SECRET=' ~/projects/pinaivu/coordinator/.env | cut -d= -f2)
curl -s "http://13.206.80.190:4000/v1/admin/settlements/$REQ" \
  -H "x-sidecar-secret: $SECRET" | jq .
```

Expected (status `submitted` or `confirmed`):

```json
{
  "request_id": "<uuid>",
  "payments": [{
    "status": "submitted",
    "tx_digest": "5ePLsmVqcCAzFmpLz9XAQaKpsDFWbjWHJtSfkNoLUwMe",
    "submitted_at": "2026-05-21T18:58:23.134765+00:00",
    "amount_nanox": 1000000,
    "payee_sui_address": "0x5325…5d2"
  }]
}
```

## Step 6 — verify on Sui

```bash
PKG=0x60829f1e25091670a091a0c2acc1734d47897a43e9c3c88e44a54b8b22697014

curl -s -X POST https://fullnode.testnet.sui.io \
  -H 'content-type: application/json' \
  -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"suix_queryEvents\",
       \"params\":[{\"MoveEventType\":\"$PKG::vault::Settled\"},null,3,true]}" \
  | jq '.result.data[] | {tx: .id.txDigest, parsedJson}'
```

The latest event should contain:
- `request_id` = the 16 raw bytes of your UUID
- `payee` = your payout address
- `amount` = `1000000`
- `tx` matches the `tx_digest` from step 5

## What can go wrong (and what each signal means)

| Symptom | Diagnose with | Likely cause |
|---|---|---|
| Coordinator boot stalls at `CHK 05.1` | `tail /tmp/coordinator.log` on EC2 | Postgres connect: IPv6-only Supabase, missing `sslmode=require`, unescaped `@` in password (`%40`) |
| `Bridges active … socat … Network is unreachable` | `journalctl -u pinaivu-outbound-postgres` | socat resolving AAAA on IPv4-only EC2 — use `TCP4:`, not `TCP:` |
| Admin endpoint returns 401 with matching secrets | grep `admin secret mismatch` in coordinator log | env file concatenated by `cat` without trailing newline → use `awk 1` |
| `CommandArgumentError { kind: InvalidUsageOfPureArg }` | sidecar log | passing typed `vector<Payout>` as `tx.pure.vector("u8", …)`; build via `receipts::new_payout` + `makeMoveVec` |
| `MoveAbort code 1 in vault::settle` | sidecar log | `EInvalidReceipt`: signed bytes diverge from on-chain BCS — most often UUID encoded as 36-byte string vs 16 raw bytes |
| `MoveAbort code 2 in vault::settle` | sidecar log | `EPayeeNotInReceipt`: settle's `(payee, amount)` not present in `payouts` |
| `MoveAbort code 3 in vault::settle` | sidecar log | `EInsufficientTreasury`: top up the vault |
