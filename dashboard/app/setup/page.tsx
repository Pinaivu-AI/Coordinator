/**
 * /setup — one-time bootstrap page.
 *
 * Creates an account and a first API key entirely from the browser.
 * Run this once, then paste the values into .env.local and restart.
 * The page is always accessible even when DASHBOARD_ACCOUNT_ID is not set.
 */
"use client";

import { useState } from "react";

type Step = "idle" | "creating_account" | "creating_key" | "done" | "error";

interface Result {
  accountId: string;
  apiKey:    string;
  prefix:    string;
}

export default function SetupPage() {
  const [email, setEmail]   = useState("");
  const [step, setStep]     = useState<Step>("idle");
  const [result, setResult] = useState<Result | null>(null);
  const [error, setError]   = useState("");

  async function run() {
    setStep("creating_account");
    setError("");
    try {
      // 1. Create account
      const aRes = await fetch("/api/accounts", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ email: email || undefined }),
      });
      if (!aRes.ok) throw new Error(`Account: ${await aRes.text()}`);
      const acct = await aRes.json();

      // 2. Create key for that account
      setStep("creating_key");
      const kRes = await fetch("/api/keys", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          account_id:  acct.id,
          name:        "default",
          rpm_limit:   60,
          daily_limit: 10000,
        }),
      });
      if (!kRes.ok) throw new Error(`Key: ${await kRes.text()}`);
      const kdata = await kRes.json();

      setResult({ accountId: acct.id, apiKey: kdata.key, prefix: kdata.key_prefix });
      setStep("done");
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
      setStep("error");
    }
  }

  return (
    <div className="max-w-lg">
      <h1 className="text-2xl font-semibold mb-1">One-time Setup</h1>
      <p className="text-gray-400 text-sm mb-8">
        Creates your first account and API key. Run this once, then paste the
        values into <code className="text-indigo-300 text-xs bg-gray-800 px-1.5 py-0.5 rounded">.env.local</code> and
        restart the dashboard.
      </p>

      {step === "done" && result ? (
        <div className="bg-gray-900 border border-gray-700 rounded-xl px-6 py-5 space-y-5">
          <h2 className="text-green-400 font-medium">Setup complete ✓</h2>

          <EnvBlock
            label="Paste into .env.local"
            value={`DASHBOARD_ACCOUNT_ID=${result.accountId}\nDASHBOARD_API_KEY=${result.apiKey}`}
          />

          <div className="text-xs text-yellow-300 bg-yellow-900/30 border border-yellow-700 rounded px-3 py-2">
            ⚠ This is the only time the raw API key is shown. Copy it now.
          </div>

          <div className="space-y-2">
            <Row label="Account ID" value={result.accountId} mono />
            <Row label="API key"    value={result.apiKey}    mono secret />
          </div>

          <p className="text-xs text-gray-500">
            After updating .env.local, restart with{" "}
            <code className="bg-gray-800 px-1 rounded">npm run dev</code> and
            go to{" "}
            <a href="/keys" className="text-indigo-400 hover:underline">/keys</a>.
          </p>
        </div>
      ) : (
        <div className="bg-gray-900 border border-gray-800 rounded-xl px-6 py-5">
          <label className="block text-xs text-gray-400 mb-1">
            Email (optional)
          </label>
          <input
            value={email}
            onChange={(e) => setEmail(e.target.value)}
            placeholder="you@example.com"
            className="w-full bg-gray-800 border border-gray-700 rounded px-3 py-2 text-sm text-white mb-4 outline-none focus:border-indigo-500"
          />

          {error && (
            <div className="bg-red-900/30 border border-red-700 rounded px-3 py-2 text-xs text-red-300 mb-4">
              {error}
              <p className="mt-1 text-red-400">
                Make sure <code>SIDECAR_SECRET</code> in .env.local matches the
                coordinator&apos;s value.
              </p>
            </div>
          )}

          <button
            onClick={run}
            disabled={step !== "idle" && step !== "error"}
            className="w-full py-2.5 bg-indigo-600 hover:bg-indigo-500 disabled:opacity-50 text-white text-sm font-medium rounded-lg transition-colors"
          >
            {step === "creating_account" && "Creating account…"}
            {step === "creating_key"     && "Creating API key…"}
            {(step === "idle" || step === "error") && "Create account + key"}
          </button>
        </div>
      )}
    </div>
  );
}

function Row({
  label, value, mono = false, secret = false,
}: {
  label: string; value: string; mono?: boolean; secret?: boolean;
}) {
  const [show, setShow] = useState(!secret);
  return (
    <div>
      <p className="text-xs text-gray-500 mb-0.5">{label}</p>
      <div className="flex items-center gap-2">
        <code className={`flex-1 text-xs bg-gray-800 px-2 py-1 rounded break-all ${mono ? "font-mono text-indigo-300" : ""}`}>
          {show ? value : "sk-pnv-••••••••••••"}
        </code>
        {secret && (
          <button onClick={() => setShow(!show)} className="text-xs text-gray-500 hover:text-gray-300 shrink-0">
            {show ? "hide" : "show"}
          </button>
        )}
      </div>
    </div>
  );
}

function EnvBlock({ label, value }: { label: string; value: string }) {
  const [copied, setCopied] = useState(false);
  function copy() {
    navigator.clipboard.writeText(value);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  }
  return (
    <div>
      <div className="flex items-center justify-between mb-1">
        <p className="text-xs text-gray-500">{label}</p>
        <button onClick={copy} className="text-xs text-indigo-400 hover:text-indigo-300">
          {copied ? "Copied!" : "Copy"}
        </button>
      </div>
      <pre className="bg-gray-800 rounded px-3 py-2 text-xs text-green-300 font-mono whitespace-pre-wrap">
        {value}
      </pre>
    </div>
  );
}
