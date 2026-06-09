# Deployment

How the coordinator gets built, attested, and run.

## Build (CI on every push)

The Containerfile is a stagex multi-stage build:

1. Pinned stagex base images (musl toolchain, rust, busybox, socat).
2. `node:22-alpine` build stage → the musl-linked `node` binary plus
   `libstdc++` / `libgcc_s` runtime libs.
3. `npm ci --omit=dev` against `src/coordinator/scripts/package.json`
   to produce the pinned `node_modules` for the TS sidecar.
4. `cargo build --release --target x86_64-unknown-linux-musl`
   for `init`, `coordinator`, and the support crates.
5. Initramfs assembly: kernel module, busybox, socat, Rust binaries,
   node binary + libs + scripts dir.
6. `eif_build` produces `coordinator.eif` + `coordinator.pcrs`.

The build is reproducible: rebuilding the same source tree produces
the same `coordinator.pcrs` byte-for-byte. Those PCRs are what the
on-chain `EnclaveConfig<ENCLAVE>` is updated to via `update_pcrs`
before the first `register_enclave` will succeed.

## EC2 deployment

Triggered by every push to `main` of the coordinator repo. Workflow:
`.github/workflows/deploy.yml`.

```
GitHub Actions
  ├─ scp source to EC2
  ├─ write ~/.env.runtime from PINAIVU_ENV_FILE secret
  ├─ install docker, nitro-cli, socat, jq, node, npm on EC2 (idempotent)
  ├─ docker build → coordinator.eif + coordinator.pcrs in ./out
  ├─ stop previous bridges, pkill any leftover manual socats
  ├─ terminate previous enclave, compact memory, restart allocator
  │                        (NOT just `start` — restart so memory_mib changes apply)
  ├─ nitro-cli run-enclave --cpu-count 2 --memory 4096 --eif-path …
  ├─ register systemd units for the host-side socat bridges:
  │       pinaivu-bridge-http     TCP:4000 → VSOCK:CID:4000
  │       pinaivu-bridge-libp2p   TCP:4001 → VSOCK:CID:4001
  │       pinaivu-logs            VSOCK:5000 → /tmp/coordinator.log
  │       pinaivu-outbound-postgres VSOCK:8101 → TCP:supabase
  │       pinaivu-outbound-redis    VSOCK:8102 → TCP:upstash
  ├─ awk-1 ~/.env.runtime ~/.env.runtime.dynamic | socat - VSOCK:CID:7000
  │                       # concatenation must be newline-terminated:
  │                       # if ~/.env.runtime has no trailing newline,
  │                       # cat A B glues the last line of A onto the
  │                       # first of B and the value of SIDECAR_SECRET
  │                       # ends up containing PINAIVU_ENCLAVE_OBJECT_ID
  ├─ wait for /health, then re-check uptime after 60s
  │                       (catches coordinators that pass /health then die)
  └─ register coordinator on Sui via scripts/register-coordinator.sh:
     ├─ awk-parse out/coordinator.pcrs (plain `<hex>  PCR<N>` lines)
     ├─ sui client call enclave::update_pcrs
     ├─ curl http://localhost:4000/get_attestation
     ├─ node scripts/register/hex-to-vector.mjs   # → "[1u8,2u8,...]"
     ├─ sui client ptb
     │      load_nitro_attestation(att) | register_enclave(config,cap,doc)
     ├─ append PINAIVU_ENCLAVE_OBJECT_ID=<new> to ~/.env.runtime.dynamic
     └─ POST /v1/admin/set-enclave-id  (X-Sidecar-Secret)
            → coordinator forwards to sidecar over loopback
            → sidecar's activeEnclaveObjectId gets set in-process
```

## Inside the enclave at boot

Reading `src/init/src/main.rs`:

```
init (PID 1) runs:
  1. Mount /proc, /sys, /dev, /tmp, cgroups
  2. NSM heartbeat (signals nitro-cli the enclave is alive)
  3. Insert nsm.ko, seed kernel entropy
  4. Bring up loopback
  5. vsock_accept(7000) — blocks until parent pushes the env file
  6. Apply env defaults (PINAIVU_BIND, PINAIVU_LIBP2P_LISTEN, ...)
  7. Generate SIDECAR_SECRET if not set (random 32 bytes from NSM RNG)
  8. Write /etc/hosts entries mapping the real Postgres/Redis/Sui RPC
     hostnames to 127.0.0.1 (TLS SNI must see the real name; bridge
     forwards bytes).
  9. socat bridges:
       TCP-LISTEN:5432 → VSOCK:3:8101   Postgres outbound
       TCP-LISTEN:6379 → VSOCK:3:8102   Redis outbound
       TCP-LISTEN:443  → VSOCK:3:8103   Sui RPC outbound (HTTPS)
       VSOCK-LISTEN:4000 → TCP:127.0.0.1:4000   HTTP inbound
       VSOCK-LISTEN:4001 → TCP:127.0.0.1:4001   libp2p inbound
 10. Truncate /tmp/coordinator.log; open it append-only for both
     sidecar and coordinator (O_APPEND on both writers so atomic
     line-sized appends don't race-corrupt each other's bytes).
 11. Spawn TS sidecar:
       /usr/local/bin/node /scripts/node_modules/tsx/dist/cli.mjs
                          /scripts/sidecar-server.ts
       Sidecar listens on 127.0.0.1:8200 (loopback only).
       Sidecar reads PINAIVU_ENCLAVE_OBJECT_ID at startup so it can
       settle without an HTTP push, when the dynamic env file already
       holds an id from a prior deploy.
 12. Spawn coordinator binary; stdout+stderr → /tmp/coordinator.log
 13. Native Rust thread (log_forwarder) re-opens the file every ~100 ms,
     tracks its own position, and writes new bytes to VSOCK:5000 via
     libc::send + MSG_NOSIGNAL. Replaces the earlier
     `socat EXEC:tail -f` chain that suffered from libc block-buffering
     and never surfaced lines past the first burst.
 14. wait(coordinator)
 15. reboot()   # exits the enclave on any process exit
```

## Required env (pushed via VSOCK:7000)

| Variable | What | Set by |
|---|---|---|
| `DATABASE_URL` | Supabase Postgres URL — must end in `?sslmode=require` and use the **Session pooler** host (free-tier direct connect is IPv6-only) | `PINAIVU_ENV_FILE` |
| `REDIS_URL` | Upstash Redis URL — use `rediss://` + auth, the dumb VSOCK bridge doesn't terminate TLS | `PINAIVU_ENV_FILE` |
| `POSTGRES_BRIDGE_HOST` / `_PORT` | Real upstream host/port — init writes a 127.0.0.1 alias in `/etc/hosts` so TLS SNI sees the real name | `PINAIVU_ENV_FILE` |
| `REDIS_BRIDGE_HOST` / `_PORT` | Same trick for Redis | `PINAIVU_ENV_FILE` |
| `SUI_RPC_URL` | Sui fullnode URL (e.g. `https://fullnode.testnet.sui.io`); init also adds an `/etc/hosts` alias for this host | `PINAIVU_ENV_FILE` |
| `SUI_NETWORK` | `mainnet` / `testnet` / `devnet` | `PINAIVU_ENV_FILE` |
| `OPERATOR_PRIVATE_KEY` | Sui secret key (`suiprivkey1…`) used by sidecar to sign PTBs | `PINAIVU_ENV_FILE` |
| `PINAIVU_PACKAGE_ID` | Published address of `pinaivu` contracts | `PINAIVU_ENV_FILE` |
| `PINAIVU_ENCLAVE_CONFIG_ID` | Shared-object id of `EnclaveConfig<ENCLAVE>` | `PINAIVU_ENV_FILE` |
| `PINAIVU_CAP_ID` | Owned-object id of `Cap<ENCLAVE>` (held by operator) | `PINAIVU_ENV_FILE` |
| `PINAIVU_VAULT_ID` | Shared-object id of `Vault<SUI>` | `PINAIVU_ENV_FILE` |
| `PINAIVU_ENCLAVE_OBJECT_ID` | Set by `register-coordinator.sh` in `~/.env.runtime.dynamic` after a successful `register_enclave`; the next boot's sidecar reads it directly | dynamic env (post-register) |
| `SIDECAR_URL` | Default `http://127.0.0.1:8200` | init |
| `SIDECAR_SECRET` | Hex 32 bytes — used to authenticate coordinator↔sidecar and the `/v1/admin/*` endpoints; auto-generated only if not pushed | `PINAIVU_ENV_FILE` |
| `PINAIVU_BIND` | Default `127.0.0.1:4000` | init |
| `PINAIVU_LIBP2P_LISTEN` | Default `/ip4/0.0.0.0/tcp/4001` | init |

### `~/.env.runtime` vs `~/.env.runtime.dynamic`

- `~/.env.runtime` is **overwritten on every deploy** from the
  `PINAIVU_ENV_FILE` GH Actions secret. Holds static config.
- `~/.env.runtime.dynamic` is written by post-boot scripts running on
  the host (e.g. `register-coordinator.sh`). Holds values discovered
  after enclave start — currently just `PINAIVU_ENCLAVE_OBJECT_ID`.
  Survives across deploys.
- The VSOCK:7000 push concatenates both with `awk 1` so every line
  ends in `\n` and the last line of `.env.runtime` cannot glue itself
  onto the first line of `.env.runtime.dynamic`. (Earlier `cat A B`
  did exactly that, producing a 156-char `SIDECAR_SECRET` value that
  broke the admin endpoint's auth comparison.)

## Local dev

Running the coordinator outside an enclave still works (mock NSM
attestation):

```bash
cd ~/projects/pinaivu/coordinator
DATABASE_URL=postgres://... REDIS_URL=redis://localhost:6379 \
SIDECAR_SECRET=local-dev \
cargo run -p coordinator
```

Skip the sidecar in dev — registration just logs a warning and the
HTTP server keeps serving. Inference flow doesn't depend on it.

## Smoke testing prod

```bash
EC2_IP=<your IP>
curl -s http://$EC2_IP:4000/health           # → "ok"
curl -s http://$EC2_IP:4000/enclave_health   # → { pubkey, peer_id, uptime_ms }
```

For the full inference round-trip with a node, see the node repo's
README.
