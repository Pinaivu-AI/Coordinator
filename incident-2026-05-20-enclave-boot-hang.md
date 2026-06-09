# Incident: enclave boots, but coordinator hangs at Postgres connect (2026-05-20)

## Outcome

After ~6 failed deploys, the coordinator booted successfully on EC2 inside
the Nitro Enclave:

```
Health check passed
--- /enclave_health ---
{
  "public_key_hex": "72c3618060f158c4c899514357b83d5cd734268b222a4b093cf201c889874b32",
  "peer_id": "12D3KooWHYMSbkpCP4BJwFor1sn9ZC1NwGxoLCWVrWFNunGyPQQq",
  "uptime_ms": 12394,
  ...
}
Uptime at first probe: 12420 ms
Uptime after 60s:       72429 ms
```

## Symptom

Every deploy passed the build + EIF + enclave-launch + bridge-setup
phases, but the health probe failed for 130s with the coordinator log
truncated to a single line:

```
2026-05-20T20:03:24Z  INFO coordinator: enclave key generated
```

then `exit status: 1`. No error, no panic, no retry warning — just dead silence.

## Root causes (three stacked bugs)

### 1. Diagnostic surface was inadequate

`tracing::warn!` and `?`-propagated errors were going through paths that
either buffered or never made it to `/tmp/coordinator.log`. Until we
added unbuffered `eprintln!` checkpoints in `main.rs` and `pg::connect`,
every failed deploy looked identical.

**Fix:** wrap `main()` to print any `Err` via `eprintln!("FATAL: …")` and
sprinkle `CHK NN` checkpoints around every blocking call. Stderr is
unbuffered in Rust — these always survive process exit.

### 2. Outbound socat bridges were trying IPv6

The host-side `pinaivu-outbound-postgres.service` unit was running:

```
ExecStart=/usr/bin/socat VSOCK-LISTEN:8101,fork,reuseaddr TCP:db.<…>.supabase.co:5432
```

socat resolves the hostname, gets an AAAA record back, tries to connect
via IPv6, fails (EC2 has no IPv6 routing), and retries forever:

```
socat[34820] E connect(5, AF=10 [2406:da1a:082a:9d02:…]:5432, 28): Network is unreachable
```

**Fix:** `TCP:` → `TCP4:` in all three outbound socat bridges in
`.github/workflows/deploy.yml` (postgres, redis, sui).

### 3. Free-tier Supabase direct connect is IPv6-only

After forcing TCP4 we got a new error:

```
socat[42406] E getaddrinfo("db.<…>.supabase.co", "NULL", ...): Name or service not known
```

= the direct-connect hostname has **no A record**. Free-tier Supabase
moved to IPv6-only on the direct endpoint; IPv4 access requires the
**Session Pooler** (Supavisor) endpoint with a different host, username,
and URL format.

**Fix:** swap the connection string to
`postgresql://postgres.<project_ref>:<pw>@aws-1-<region>.pooler.supabase.com:5432/postgres`.
Use session pooler (port 5432), not transaction pooler (6543) — sqlx
relies on prepared statements which break under transaction-mode pooling.

## TLS SNI subtlety (not a bug, but easy to get wrong)

The VSOCK bridge is a dumb byte pipe. It does not terminate TLS. So when
the coordinator connects to upstream Postgres/Redis through it, the
client's TLS ClientHello (with SNI in plaintext) is forwarded to the
real server, which uses SNI to route to the right tenant.

If we put `127.0.0.1` in the connection URL, the client sends SNI=127.0.0.1,
which Supabase/Upstash silently drops because no tenant matches.

**Mitigation in init:** read `POSTGRES_BRIDGE_HOST` / `REDIS_BRIDGE_HOST`
from the pushed VSOCK config and write them to `/etc/hosts` inside the
enclave, mapping each to `127.0.0.1`. The URL then keeps the real
hostname (so SNI is correct) while traffic still flows through loopback
to the bridge.

## Stacked URL-encoding gotcha

The Postgres password `Pinaivu@2026` contains `@`, which collides with
the userinfo/host delimiter in URLs. sqlx parses
`postgresql://postgres:Pinaivu@2026@host:5432/db` ambiguously and may
hang or fail silently. Encode it as `%40`:
`postgresql://postgres:Pinaivu%402026@host:5432/db`.

This is unrelated to the SNI/IPv4/IPv6 chain, but appeared in the
debugging session because the fix landed alongside the SSL changes.

## Attempt log

| # | Theory                                                | Action                                                          | Result                                  |
|---|-------------------------------------------------------|-----------------------------------------------------------------|-----------------------------------------|
| 1 | Stdout buffering swallows log lines                   | (no change — `tracing` was already on stderr)                   | Confirmed not the bug                   |
| 2 | `Result`-style error from main wasn't being printed   | Added `FATAL:` wrapper + `CHK NN` checkpoints                   | Diagnostic now usable                   |
| 3 | Local `.env` edits weren't reaching prod              | Realised deploy reads `PINAIVU_ENV_FILE` GH secret, not `.env`  | Pivoted to editing the GH secret        |
| 4 | TLS hostname mismatch (127.0.0.1 vs Supabase cert)    | Added `?sslmode=require`                                        | Still hung — required but not sufficient|
| 5 | URL parser confused by `@` in password                | Encoded as `%40`                                                | Still hung — fixed a latent bug         |
| 6 | TLS SNI routing requires real hostname in URL         | URL → real hostname, `/etc/hosts` maps to 127.0.0.1 in init     | Still hung — needed but not the cause   |
| 7 | socat resolving AAAA first, EC2 has no IPv6           | `TCP:` → `TCP4:` in all outbound socat bridges                  | New error: "Name or service not known"  |
| 8 | Direct hostname is IPv6-only, no A record exists      | Switch to Supabase Session Pooler endpoint                      | **Coordinator booted** ✅                |
| 9 | Redis would have hit the same trifecta + need TLS+auth| `rediss://default:<pw>@careful-ram-…upstash.io:6379`            | Wired alongside Postgres fix            |

## Lessons

- **Make failures loud before debugging deploys.** `eprintln!` checkpoints
  + a `FATAL:` wrapper around main are cheap insurance and would have
  caught this in attempt 1, not attempt 8.
- **A VSOCK bridge is bytes, not protocol.** TLS, auth, and SNI must all
  be correct end-to-end; the bridge passes them through transparently.
- **`/etc/hosts` is the right place to mediate** between an SNI-sensitive
  URL and a loopback bridge. It keeps the protocol-layer hostname intact
  while still routing through the local socket.
- **Default `socat TCP:` happily picks IPv6** on dual-stack hosts even
  when the surrounding network is IPv4-only. Pin to `TCP4:` for outbound
  in EC2-style environments unless you've deliberately enabled IPv6.
- **Supabase free-tier direct connect is IPv6-only.** Plan on the Session
  Pooler from day one if the client side is IPv4 or runs through a
  bridge that you don't control.
