"use client";

import { useState } from "react";

export default function CreateKeyForm({ accountId }: { accountId: string }) {
  const [open, setOpen]     = useState(false);
  const [name, setName]     = useState("");
  const [result, setResult] = useState<{ key: string; key_prefix: string } | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError]   = useState("");
  const [copied, setCopied] = useState(false);

  async function handleCreate() {
    if (!name.trim()) { setError("Give your key a name first."); return; }
    setLoading(true);
    setError("");
    try {
      const res = await fetch("/api/keys", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          account_id:  accountId,
          name:        name.trim(),
          // Limits are tier-based — not exposed to users.
          rpm_limit:   60,
          daily_limit: 10000,
        }),
      });
      if (!res.ok) throw new Error(await res.text());
      setResult(await res.json());
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : "Unknown error");
    } finally {
      setLoading(false);
    }
  }

  function copyKey() {
    if (!result) return;
    navigator.clipboard.writeText(result.key);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  }

  function dismiss() {
    setResult(null);
    setOpen(false);
    setName("");
    window.location.reload();
  }

  // ── After creation: show the raw key once ────────────────────────────────────
  if (result) {
    return (
      <div className="bg-gray-900 border border-green-800 rounded-xl px-5 py-4 max-w-md">
        <p className="text-sm font-medium text-green-400 mb-3">
          Key created — copy it now, it won&apos;t be shown again.
        </p>
        <div className="flex items-center gap-2 mb-3">
          <code className="flex-1 bg-gray-950 rounded px-3 py-2 text-xs font-mono text-green-200 break-all">
            {result.key}
          </code>
          <button
            onClick={copyKey}
            className="shrink-0 px-3 py-2 bg-gray-800 hover:bg-gray-700 text-xs text-gray-300 rounded transition-colors"
          >
            {copied ? "Copied!" : "Copy"}
          </button>
        </div>
        <button onClick={dismiss} className="text-xs text-gray-500 hover:text-gray-300">
          Done
        </button>
      </div>
    );
  }

  // ── Create button + inline form ───────────────────────────────────────────────
  if (!open) {
    return (
      <button
        onClick={() => setOpen(true)}
        className="px-4 py-2 bg-indigo-600 hover:bg-indigo-500 text-white text-sm rounded-lg transition-colors"
      >
        + Create key
      </button>
    );
  }

  return (
    <div className="flex items-center gap-3">
      <input
        autoFocus
        value={name}
        onChange={(e) => setName(e.target.value)}
        onKeyDown={(e) => e.key === "Enter" && handleCreate()}
        placeholder="Key name  e.g. production, test"
        className="w-56 bg-gray-900 border border-gray-700 focus:border-indigo-500 rounded-lg px-3 py-2 text-sm text-white outline-none"
      />
      {error && <p className="text-xs text-red-400">{error}</p>}
      <button
        onClick={handleCreate}
        disabled={loading}
        className="px-4 py-2 bg-indigo-600 hover:bg-indigo-500 disabled:opacity-50 text-white text-sm rounded-lg transition-colors"
      >
        {loading ? "Creating…" : "Create"}
      </button>
      <button onClick={() => setOpen(false)} className="text-sm text-gray-500 hover:text-gray-300">
        Cancel
      </button>
    </div>
  );
}
