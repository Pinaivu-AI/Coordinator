"use client";

import { useState } from "react";

export default function CreateKeyForm({ accountId }: { accountId: string }) {
  const [open, setOpen] = useState(false);
  const [name, setName] = useState("");
  const [rpm, setRpm] = useState(60);
  const [daily, setDaily] = useState(1000);
  const [result, setResult] = useState<{ key: string; key_prefix: string } | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");

  async function handleCreate() {
    setLoading(true);
    setError("");
    try {
      const res = await fetch("/api/keys", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ account_id: accountId, name, rpm_limit: rpm, daily_limit: daily }),
      });
      if (!res.ok) throw new Error(await res.text());
      const data = await res.json();
      setResult(data);
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : "Unknown error");
    } finally {
      setLoading(false);
    }
  }

  if (result) {
    return (
      <div className="bg-green-900/30 border border-green-700 rounded-lg px-5 py-4 max-w-lg">
        <p className="text-sm text-green-300 font-medium mb-2">
          Key created — copy it now, it won&apos;t be shown again.
        </p>
        <code className="block bg-gray-950 rounded px-3 py-2 text-xs font-mono text-green-200 break-all mb-3">
          {result.key}
        </code>
        <button
          onClick={() => { setResult(null); setOpen(false); setName(""); window.location.reload(); }}
          className="text-xs text-gray-400 hover:text-white"
        >
          Done
        </button>
      </div>
    );
  }

  if (!open) {
    return (
      <button
        onClick={() => setOpen(true)}
        className="px-4 py-2 bg-indigo-600 hover:bg-indigo-500 text-white text-sm rounded-lg transition-colors"
      >
        + New key
      </button>
    );
  }

  return (
    <div className="bg-gray-900 border border-gray-700 rounded-lg px-5 py-4 w-80">
      <p className="text-sm font-medium mb-3">New API key</p>

      <label className="block text-xs text-gray-400 mb-1">Name (optional)</label>
      <input
        value={name}
        onChange={(e) => setName(e.target.value)}
        placeholder="e.g. production"
        className="w-full bg-gray-800 border border-gray-700 rounded px-3 py-1.5 text-sm text-white mb-3 outline-none focus:border-indigo-500"
      />

      <div className="flex gap-3 mb-4">
        <div className="flex-1">
          <label className="block text-xs text-gray-400 mb-1">RPM limit</label>
          <input type="number" value={rpm} onChange={(e) => setRpm(Number(e.target.value))} min={1}
            className="w-full bg-gray-800 border border-gray-700 rounded px-3 py-1.5 text-sm text-white outline-none focus:border-indigo-500" />
        </div>
        <div className="flex-1">
          <label className="block text-xs text-gray-400 mb-1">Daily limit</label>
          <input type="number" value={daily} onChange={(e) => setDaily(Number(e.target.value))} min={1}
            className="w-full bg-gray-800 border border-gray-700 rounded px-3 py-1.5 text-sm text-white outline-none focus:border-indigo-500" />
        </div>
      </div>

      {error && <p className="text-xs text-red-400 mb-2">{error}</p>}

      <div className="flex gap-2">
        <button onClick={handleCreate} disabled={loading}
          className="flex-1 py-2 bg-indigo-600 hover:bg-indigo-500 disabled:opacity-50 text-white text-sm rounded-lg transition-colors">
          {loading ? "Creating…" : "Create"}
        </button>
        <button onClick={() => setOpen(false)} className="px-4 py-2 text-sm text-gray-400 hover:text-white">
          Cancel
        </button>
      </div>
    </div>
  );
}
