# Pinaivu Docs

Index of architectural docs for the Pinaivu marketplace.

| Doc | Purpose |
|---|---|
| [architecture.md](./architecture.md) | Components, data flow, where each repo lives |
| [protocol.md](./protocol.md) | Wire-format types, signing, libp2p protocols |
| [onchain.md](./onchain.md) | Sui Move contracts: enclave, receipts, vault |
| [deployment.md](./deployment.md) | Building the EIF, deploying to EC2, env vars |
| [smoke-test.md](./smoke-test.md) | Reproducible E2E run incl. on-chain settlement |
| [glossary.md](./glossary.md) | Terms used throughout |
| [incident-2026-05-20-enclave-boot-hang.md](./incident-2026-05-20-enclave-boot-hang.md) | Day-1 boot-hang post-mortem (IPv6, SNI, sslmode) |

## Repo map

```
~/projects/pinaivu/
├── coordinator/          ← Pinaivu-AI/Coordinator    (Rust enclave + TS sidecar)
│   └── src/
│       ├── coordinator/        # Rust crate: HTTP API, libp2p, signing
│       ├── coordinator/scripts # TS sidecar (Express, signs Sui PTBs)
│       ├── pinaivu-protocol/   # shared wire-format crate
│       ├── nautilus-enclave/   # NSM attestation
│       ├── aws/, init/, system/# enclave init / kernel-side helpers
├── node/                 ← Pinaivu-AI/Node          (Rust GPU-node binary)
├── contracts/            ← Pinaivu-AI/contracts     (Sui Move package)
└── docs/                 ← this directory
```

Per-component READMEs live in each repo. This `docs/` folder is the
cross-component reference — go here when you want to understand how
the pieces fit together rather than how any one piece works internally.
