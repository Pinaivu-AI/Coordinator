#!/usr/bin/env python3
"""
Full API smoke-test for the Pinaivu coordinator API platform.

Tests:
  1.  GET /enclave_health          — TLS fingerprint present
  2.  GET /v1/models               — public, no key needed
  3.  POST /v1/accounts            — create test account
  4.  POST /v1/keys                — create key, returns raw key once
  5.  GET  /v1/keys                — list keys for account
  6.  POST /v1/chat/completions    — valid key → 200/503
  7.  POST /v1/chat/completions    — no key    → 401
  8.  POST /v1/chat/completions    — bad key   → 401
  9.  GET  /v1/usage               — valid key → 200
  10. Rate limit                   — 11 rapid requests → at least one 429
  11. DELETE /v1/keys/:id          — revoke key
  12. POST /v1/chat/completions    — revoked key → 401

Run:
  python3 api_test.py --secret YOUR_SIDECAR_SECRET
  python3 api_test.py --secret YOUR_SIDECAR_SECRET --base https://13.206.80.190:4000
"""

import argparse
import base64
import hashlib
import json
import os
import ssl
import sys
import time
import urllib.request
import urllib.error

parser = argparse.ArgumentParser()
parser.add_argument("--base",   default="https://13.206.80.190:4000")
parser.add_argument("--secret", required=True, help="SIDECAR_SECRET value")
args = parser.parse_args()

BASE   = args.base
SECRET = args.secret

# Skip TLS verification — self-signed enclave cert
CTX = ssl.create_default_context()
CTX.check_hostname = False
CTX.verify_mode    = ssl.CERT_NONE

PASS = "\033[92mPASS\033[0m"
FAIL = "\033[91mFAIL\033[0m"
SKIP = "\033[93mSKIP\033[0m"
results = []


def check(label, ok, detail=""):
    tag = PASS if ok else FAIL
    line = f"  [{tag}] {label}"
    if detail:
        line += f"  →  {detail}"
    print(line)
    results.append((label, ok))
    return ok


def get(path, key=None):
    headers = {}
    if key:
        headers["Authorization"] = f"Bearer {key}"
    req = urllib.request.Request(f"{BASE}{path}", headers=headers)
    try:
        with urllib.request.urlopen(req, timeout=10, context=CTX) as r:
            return r.status, json.loads(r.read())
    except urllib.error.HTTPError as e:
        try:
            return e.code, json.loads(e.read())
        except Exception:
            return e.code, {}


def post(path, body, key=None, admin=False):
    headers = {"Content-Type": "application/json"}
    if key:
        headers["Authorization"] = f"Bearer {key}"
    if admin:
        headers["x-sidecar-secret"] = SECRET
    req = urllib.request.Request(
        f"{BASE}{path}",
        data=json.dumps(body).encode(),
        headers=headers,
        method="POST",
    )
    try:
        with urllib.request.urlopen(req, timeout=10, context=CTX) as r:
            return r.status, json.loads(r.read())
    except urllib.error.HTTPError as e:
        try:
            return e.code, json.loads(e.read())
        except Exception:
            return e.code, {}


def delete(path, admin=False):
    headers = {}
    if admin:
        headers["x-sidecar-secret"] = SECRET
    req = urllib.request.Request(
        f"{BASE}{path}", headers=headers, method="DELETE"
    )
    try:
        with urllib.request.urlopen(req, timeout=10, context=CTX) as r:
            return r.status, json.loads(r.read())
    except urllib.error.HTTPError as e:
        try:
            return e.code, json.loads(e.read())
        except Exception:
            return e.code, {}


# ── Test 1: enclave health ─────────────────────────────────────────────────────
print("\n── 1. GET /enclave_health ───────────────────────────────────────────────")
s, h = get("/enclave_health")
check("HTTP 200",                    s == 200,       f"got {s}")
check("x25519_pubkey_hex present",   bool(h.get("x25519_pubkey_hex")))
check("tls_cert_fingerprint present",bool(h.get("tls_cert_fingerprint")),
      (h.get("tls_cert_fingerprint") or "")[:16] + "…")
check("peer_id present",             bool(h.get("peer_id")))
print(f"  enclave_object_id : {h.get('enclave_object_id')}")

# ── Test 2: models (public) ────────────────────────────────────────────────────
print("\n── 2. GET /v1/models (public) ───────────────────────────────────────────")
s2, m = get("/v1/models")
check("HTTP 200",    s2 == 200, f"got {s2}")
check("data array",  isinstance(m.get("data"), list))
print(f"  models available: {[x['id'] for x in m.get('data', [])]}")

# ── Test 3: create account ─────────────────────────────────────────────────────
print("\n── 3. POST /v1/accounts ─────────────────────────────────────────────────")
s3, acct = post("/v1/accounts", {"email": f"test-{int(time.time())}@pinaivu.test"}, admin=True)
if check("HTTP 200", s3 == 200, f"got {s3}  body={acct}"):
    ACCOUNT_ID = acct["id"]
    print(f"  account_id: {ACCOUNT_ID}")
    print(f"  credits_nanox: {acct.get('credits_nanox')}")
else:
    print(f"  body: {acct}")
    print("\n  ⚠  Cannot continue without account. Is SIDECAR_SECRET correct?")
    sys.exit(1)

# ── Test 4: create key ────────────────────────────────────────────────────────
print("\n── 4. POST /v1/keys ─────────────────────────────────────────────────────")
s4, kdata = post("/v1/keys", {
    "account_id": ACCOUNT_ID,
    "name":        "api-test-key",
    "rpm_limit":   10,
    "daily_limit": 500,
}, admin=True)
if check("HTTP 200",       s4 == 200, f"got {s4}"):
    RAW_KEY = kdata["key"]
    KEY_ID  = kdata["id"]
    check("raw key starts with sk-pnv-", RAW_KEY.startswith("sk-pnv-"), RAW_KEY[:16] + "…")
    print(f"  key_id     : {KEY_ID}")
    print(f"  key_prefix : {kdata.get('key_prefix')}")
else:
    print(f"  body: {kdata}")
    sys.exit(1)

# ── Test 5: list keys ─────────────────────────────────────────────────────────
print("\n── 5. GET /v1/keys ──────────────────────────────────────────────────────")
s5, keys = get(f"/v1/keys?account_id={ACCOUNT_ID}")
check("HTTP 200",         s5 == 200,            f"got {s5}")
check("key appears in list", any(k["id"] == KEY_ID for k in (keys if isinstance(keys, list) else [])))

# ── Test 6: valid key → 200 or 503 ───────────────────────────────────────────
print("\n── 6. POST /v1/chat/completions (valid key) ─────────────────────────────")
s6, r6 = post("/v1/chat/completions", {
    "model": "llama3.2:1b",
    "messages": [{"role": "user", "content": "say hi"}],
    "client_pubkey_hex": "01" * 32,
}, key=RAW_KEY)
check("valid key → 200 or 503 (not 401)", s6 in (200, 503), f"HTTP {s6}")
if s6 == 200:
    check("dispatch_token present", "dispatch_token" in r6)
elif s6 == 503:
    print("  (no GPU node connected — auth passed, auction found no bids)")

# ── Test 7: no key → 401 ──────────────────────────────────────────────────────
print("\n── 7. POST /v1/chat/completions (no key) ────────────────────────────────")
s7, _ = post("/v1/chat/completions", {
    "model": "llama3.2:1b",
    "messages": [{"role": "user", "content": "hi"}],
    "client_pubkey_hex": "01" * 32,
})
check("no key → 401", s7 == 401, f"HTTP {s7}")

# ── Test 8: bad key → 401 ─────────────────────────────────────────────────────
print("\n── 8. POST /v1/chat/completions (bad key) ───────────────────────────────")
s8, _ = post("/v1/chat/completions", {
    "model": "llama3.2:1b",
    "messages": [{"role": "user", "content": "hi"}],
    "client_pubkey_hex": "01" * 32,
}, key="sk-pnv-" + "x" * 48)
check("bad key → 401", s8 == 401, f"HTTP {s8}")

# ── Test 9: usage endpoint ────────────────────────────────────────────────────
print("\n── 9. GET /v1/usage ─────────────────────────────────────────────────────")
s9, u9 = get("/v1/usage?days=7", key=RAW_KEY)
check("HTTP 200",              s9 == 200, f"HTTP {s9}")
check("total_requests present", "total_requests" in (u9 if isinstance(u9, dict) else {}))
if s9 == 200:
    print(f"  total_requests : {u9.get('total_requests')}")
    print(f"  total_cost_nanox: {u9.get('total_cost_nanox')}")

# ── Test 10: rate limiting ─────────────────────────────────────────────────────
print("\n── 10. Rate limiting (rpm_limit=10, fire 12 requests) ──────────────────")
got_429 = False
for i in range(12):
    s, _ = post("/v1/chat/completions", {
        "model": "llama3.2:1b",
        "messages": [{"role": "user", "content": "hi"}],
        "client_pubkey_hex": "01" * 32,
    }, key=RAW_KEY)
    if s == 429:
        got_429 = True
        print(f"  → 429 on request {i+1}")
        break
check("got HTTP 429 after exceeding rpm_limit", got_429)

# Wait for Redis window to expire before revoking
time.sleep(2)

# ── Test 11: revoke key ───────────────────────────────────────────────────────
print("\n── 11. DELETE /v1/keys/:id ──────────────────────────────────────────────")
s11, r11 = delete(f"/v1/keys/{KEY_ID}", admin=True)
check("HTTP 200",     s11 == 200, f"got {s11}")
check("revoked=true", r11.get("revoked") is True)

# ── Test 12: revoked key → 401 ────────────────────────────────────────────────
print("\n── 12. POST /v1/chat/completions (revoked key) ──────────────────────────")
s12, _ = post("/v1/chat/completions", {
    "model": "llama3.2:1b",
    "messages": [{"role": "user", "content": "hi"}],
    "client_pubkey_hex": "01" * 32,
}, key=RAW_KEY)
check("revoked key → 401", s12 == 401, f"HTTP {s12}")

# ── Summary ───────────────────────────────────────────────────────────────────
passed = sum(1 for _, ok in results if ok)
total  = len(results)
print(f"\n{'─'*60}")
print(f"  {passed}/{total} checks passed")
if passed == total:
    print(f"  [{PASS}] All API checks green.")
else:
    failed = [label for label, ok in results if not ok]
    print(f"  [{FAIL}] Failed: {', '.join(failed)}")
print(f"{'─'*60}\n")

sys.exit(0 if passed == total else 1)
