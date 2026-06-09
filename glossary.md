# Glossary

| Term | Meaning |
|---|---|
| **Coordinator** | Rust service running inside the Nitro Enclave on EC2. Brokers inference jobs: runs the auction, issues dispatch tokens, signs routing receipts. |
| **Node** | Rust binary run by GPU providers. Joins the libp2p mesh, bids on inference requests, runs Ollama, signs a `ProofOfInference` per completed job. |
| **Enclave** | AWS Nitro Enclave — an isolated VM with no external storage and only a VSOCK channel to its parent. Used here to make the coordinator's signing key tamper-resistant. |
| **NSM** | Nitro Security Module — a virtual TPM-like device that produces COSE_Sign1 attestation documents binding a Ed25519 pubkey to the running PCRs. |
| **PCR** | Platform Configuration Register — SHA-384 digest measuring code identity. PCR0 = enclave image, PCR1 = kernel, PCR2 = application. |
| **EIF** | Enclave Image Format — the bootable artifact `nitro-cli` runs. Built from a kernel + initramfs by `eif_build`. |
| **Sidecar** | Long-lived TypeScript Express server colocated with the coordinator inside the enclave. Holds the Sui operator private key and signs PTBs on the coordinator's behalf. |
| **Operator key** | Sui Ed25519 keypair that pays gas + holds `Cap<ENCLAVE>`. Lives in the sidecar's memory only. **Not** what authorises payouts — that's the enclave key. |
| **Enclave key** | The Ed25519 keypair the coordinator generates at boot. Bound into the NSM attestation. Registered on-chain as the canonical signer for Pinaivu receipts. |
| **`Enclave<ENCLAVE>`** | On-chain shared object storing the enclave's registered pubkey. Created by `pinaivu::enclave::register_enclave`. |
| **`Cap<ENCLAVE>`** | On-chain owned object proving admin rights over `EnclaveConfig<ENCLAVE>`. Held by the operator address. |
| **Vault** | On-chain shared object holding the Pinaivu treasury per coin type. `settle()` is the only path that moves funds out, gated by a coordinator-signed receipt. |
| **`RoutingReceipt`** | Signed audit artefact for one completed inference job. Holders of `(receipt, coordinator_pubkey)` can verify offline; the on-chain vault verifies the same signature when disbursing. |
| **`CompletionAck`** | What a node sends back to the coordinator after finishing a job. Carries `Vec<ProofOfInference>` — one per contributing node. |
| **`ProofOfInference`** | A node-signed attestation: "I (with this Ed25519 pubkey) processed input with this hash, produced output with this hash, in N ms." |
| **`DispatchToken`** | What the coordinator returns to a client after auction. The client hands it to the node to authorise the work; the node verifies it was issued by the registered enclave. |
| **Apalis** | Rust async job-queue library, Postgres-backed. Used for the deadline watcher and (soon) settlement worker. |
| **VSOCK** | Linux virtio socket used between the host and an enclave. The enclave has no other network. |
| **NanoX** | Payment unit — 1 X = 10⁹ NanoX (per whitepaper §6.1). All bid prices and payout amounts are in NanoX. |
| **IntentMessage** | BCS-encoded envelope `{ intent: u8, timestamp_ms: u64, payload }` used for every coordinator signature. The `intent` byte scopes the signature so one type's signature cannot be replayed as another. |
| **`.env.runtime.dynamic`** | Host-side file holding post-boot discovered values (currently just `PINAIVU_ENCLAVE_OBJECT_ID`). Concatenated alongside `~/.env.runtime` and pushed into the enclave via VSOCK:7000 at startup. Survives deploys; `.env.runtime` itself is overwritten on every deploy. |
| **Admin endpoint** | `POST /v1/admin/set-enclave-id` and `GET /v1/admin/settlements/{request_id}`. Authenticated with `X-Sidecar-Secret`. The deploy host uses the first to push a freshly-registered enclave id into the running sidecar; the second exposes payment-row status (`pending`/`submitted`/`confirmed`/`failed`) without needing a Postgres shell. |
| **log_forwarder** | Native Rust thread in `init` that polls `/tmp/coordinator.log` and streams new bytes to VSOCK:5000 via `libc::send(MSG_NOSIGNAL)`. Replaces a `socat EXEC:tail -f` chain that block-buffered after the first burst. |
