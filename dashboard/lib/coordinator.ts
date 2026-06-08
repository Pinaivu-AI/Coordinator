/**
 * Server-side coordinator API client.
 * All calls use the SIDECAR_SECRET for admin operations so the secret
 * never reaches the browser.
 */

const BASE = process.env.COORDINATOR_URL!;
const SECRET = process.env.SIDECAR_SECRET ?? "";

// ── types ─────────────────────────────────────────────────────────────────────

export interface ApiKey {
  id: string;
  key_prefix: string;
  name: string | null;
  rpm_limit: number;
  daily_limit: number;
  created_at: string;
  last_used_at: string | null;
}

export interface CreatedKey extends ApiKey {
  key: string; // raw key — shown once
}

export interface Account {
  id: string;
  credits_nanox: number;
  tier: string;
}

export interface ModelInfo {
  id: string;
  object: string;
  owned_by: string;
  nodes_available: number;
  pricing: {
    input_per_1m_tokens_nanox: number;
    output_per_1m_tokens_nanox: number;
  };
}

export interface UsageRecord {
  request_id: string | null;
  model: string;
  input_tokens: number;
  output_tokens: number;
  cost_nanox: number;
  latency_ms: number | null;
  created_at: string;
}

export interface UsageSummary {
  total_requests: number;
  total_input_tokens: number;
  total_output_tokens: number;
  total_cost_nanox: number;
  records: UsageRecord[];
}

// ── helpers ───────────────────────────────────────────────────────────────────

async function adminFetch(path: string, init: RequestInit = {}) {
  const res = await fetch(`${BASE}${path}`, {
    ...init,
    headers: {
      "content-type": "application/json",
      "x-sidecar-secret": SECRET,
      ...(init.headers ?? {}),
    },
    // Node 18 built-in fetch — disable TLS verification for the
    // self-signed enclave cert. In production swap for a real cert.
    // @ts-expect-error node fetch option
    dispatcher: undefined,
  });
  if (!res.ok) {
    const body = await res.text().catch(() => "");
    throw new Error(`${path} → ${res.status}: ${body}`);
  }
  return res.json();
}

// ── accounts ──────────────────────────────────────────────────────────────────

export async function createAccount(
  email?: string,
  walletAddr?: string,
): Promise<Account> {
  return adminFetch("/v1/accounts", {
    method: "POST",
    body: JSON.stringify({ email, wallet_addr: walletAddr }),
  });
}

// ── keys ──────────────────────────────────────────────────────────────────────

export async function createKey(
  accountId: string,
  name?: string,
  rpmLimit = 60,
  dailyLimit = 1000,
): Promise<CreatedKey> {
  return adminFetch("/v1/keys", {
    method: "POST",
    body: JSON.stringify({
      account_id: accountId,
      name,
      rpm_limit: rpmLimit,
      daily_limit: dailyLimit,
    }),
  });
}

export async function listKeys(accountId: string): Promise<ApiKey[]> {
  return adminFetch(`/v1/keys?account_id=${accountId}`);
}

export async function revokeKey(keyId: string): Promise<{ revoked: boolean }> {
  return adminFetch(`/v1/keys/${keyId}`, { method: "DELETE" });
}

// ── models ────────────────────────────────────────────────────────────────────

export async function listModels(): Promise<{ data: ModelInfo[] }> {
  const res = await fetch(`${BASE}/v1/models`);
  if (!res.ok) throw new Error(`/v1/models → ${res.status}`);
  return res.json();
}

// ── usage ─────────────────────────────────────────────────────────────────────

export async function getUsage(
  apiKey: string,
  days = 30,
): Promise<UsageSummary> {
  const res = await fetch(`${BASE}/v1/usage?days=${days}`, {
    headers: { Authorization: `Bearer ${apiKey}` },
  });
  if (!res.ok) throw new Error(`/v1/usage → ${res.status}`);
  return res.json();
}

// ── health ────────────────────────────────────────────────────────────────────

export async function enclaveHealth() {
  const res = await fetch(`${BASE}/enclave_health`);
  if (!res.ok) throw new Error(`/enclave_health → ${res.status}`);
  return res.json();
}
