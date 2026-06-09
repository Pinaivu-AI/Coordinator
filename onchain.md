# On-chain (Sui Move)

Source: `Pinaivu-AI/contracts` (local at `~/projects/pinaivu/contracts/`).

## Three modules

```
pinaivu::enclave   ← root of trust: register PCRs + attested pubkey
       │
       │ verify_signature<ENCLAVE, P>()
       ▼
pinaivu::receipts  ← typed payload shape that matches Rust's BCS encoding
       │
       │ verify_completion_receipt(...)
       ▼
pinaivu::vault     ← treasury + settle(payee, amount, signature)
```

## `pinaivu::enclave`

- `EnclaveConfig<ENCLAVE>` — shared object holding the expected PCR
  triple. The admin holding `Cap<ENCLAVE>` rotates PCRs after every
  reproducible build via `update_pcrs`.
- `Enclave<ENCLAVE>` — shared object created by `register_enclave`
  after a fresh NSM attestation document is verified against the
  config's PCRs. Stores the Ed25519 pubkey extracted from the document.
- `verify_signature<ENCLAVE, P: drop>(enclave, intent, timestamp_ms,
  payload, signature)` — wraps the payload in an `IntentMessage`,
  BCS-encodes it, and verifies with Sui's `ed25519` precompile.

## `pinaivu::receipts`

- `Payout { sui_address: address, amount: u64 }`
- `ReceiptPayload { request_id: vector<u8>, aggregated_output_hash:
  vector<u8>, payouts: vector<Payout> }`
- `verify_completion_receipt(enclave, timestamp_ms, request_id,
  hash, payouts, &signature)` — convenience wrapper that calls
  `enclave::verify_signature::<ENCLAVE, ReceiptPayload>(...)` with the
  reserved `INTENT_ROUTING_RECEIPT = 1` intent scope.

## `pinaivu::vault`

Treasury model — Pinaivu pre-funds a shared `Balance<T>` per supported
coin type. Clients **do not** deposit per request.

```
Vault<phantom T> {
    treasury: Balance<T>,
    settled:  Table<(request_id || payee), bool>,  // dedupe
}

public fun new_vault<T>(ctx)
public fun top_up<T>(vault, payment)          // Pinaivu funds the pool
public fun settle<T>(
    vault, enclave,
    request_id, payee, amount,
    timestamp_ms, aggregated_output_hash, payouts,
    signature,
    ctx,
)
public fun treasury_balance<T>(vault): u64
```

`settle` aborts unless:
1. `(request_id, payee)` hasn't been settled before
2. `verify_completion_receipt(...)` returns true (signature valid)
3. `(payee, amount)` appears in the `payouts` list
4. Treasury has enough balance left

Emits `TreasuryToppedUp` + `Settled` events for the off-chain explorer
to index.

## Why settle is safe even if the coordinator operator key leaks

The operator key (held by the in-enclave sidecar) signs and submits
the `settle()` PTB but does NOT authorise the disbursement. The
disbursement is authorised by the **separate enclave-attested
Ed25519 key** (registered via `register_enclave`) which signs the
`ReceiptPayload` BCS bytes. The sidecar cannot forge that signature;
only the coordinator running attested code can.

A compromised operator key can pay gas to submit valid receipts
faster, or DOS the vault by submitting bogus PTBs (they all abort
without moving money). It cannot drain funds.

## Deploy outline

```bash
# 1. Publish
cd ~/projects/pinaivu/contracts && sui client publish --gas-budget 200000000
# → PackageId, Cap<ENCLAVE>, EnclaveConfig<ENCLAVE> object ids

# 2. Set real PCRs (reproducible build emits coordinator.pcrs)
sui client call --package $PKG --module enclave --function update_pcrs \
  --type-args "$PKG::enclave::ENCLAVE" \
  --args $CONFIG $CAP <pcr0_hex> <pcr1_hex> <pcr2_hex>

# 3. One vault per coin type (SUI shown)
sui client call --package $PKG --module vault --function new_vault \
  --type-args "0x2::sui::SUI"

# 4. Top up
sui client call --package $PKG --module vault --function top_up \
  --type-args "0x2::sui::SUI" --args $VAULT_ID $FUNDING_COIN_ID
```

## Coordinator registration (host-driven)

Registration is **authoritative on the EC2 host**, not inside the
enclave. The in-enclave path still runs in parallel (capped at 5
retries) for redundancy, but its calls go through the sidecar's
HTTPS bridge to Sui RPC and tend to lose a race against
`update_pcrs`: when an enclave's NSM attestation contains PCRs that
differ from `EnclaveConfig.pcrs` on-chain, `register_enclave` aborts
`EInvalidPcrs`. The host-side path runs `update_pcrs` first and
therefore always converges.

`scripts/register-coordinator.sh` on the host:

1. Parse `out/coordinator.pcrs` (plain `<hex>  PCR<N>` lines, not JSON).
2. `sui client call enclave::update_pcrs` so the on-chain
   `EnclaveConfig<ENCLAVE>` matches this build's PCRs.
3. `curl http://localhost:4000/get_attestation` — the live enclave's
   NSM document, binding its Ed25519 pubkey to the same PCRs.
4. `node scripts/register/hex-to-vector.mjs` converts the 9 KB hex
   blob into a Sui CLI PTB vector literal (`[0u8,1u8,…]`).
5. `sui client ptb` chains
   `0x2::nitro_attestation::load_nitro_attestation` →
   `pinaivu::enclave::register_enclave<ENCLAVE>(config, cap, doc)`
   in a single transaction.
6. Extract the new `Enclave<ENCLAVE>` shared object id from the tx's
   `objectChanges`.
7. Append (or update) `PINAIVU_ENCLAVE_OBJECT_ID=<id>` in
   `~/.env.runtime.dynamic` so the next enclave boot picks it up via
   the VSOCK:7000 config push.
8. `POST /v1/admin/set-enclave-id` to the **currently running**
   coordinator with `X-Sidecar-Secret`. Coordinator updates its
   `/enclave_health` cache and forwards over loopback to the sidecar's
   `PUT /sui/set-enclave-id`, so `activeEnclaveObjectId` is live for
   `vault::settle` calls **in the same deploy** that registered.
