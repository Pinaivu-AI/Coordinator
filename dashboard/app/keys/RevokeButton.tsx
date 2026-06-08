"use client";

import { useState } from "react";

export default function RevokeButton({ keyId }: { keyId: string }) {
  const [confirming, setConfirming] = useState(false);
  const [done, setDone] = useState(false);

  async function handleRevoke() {
    await fetch(`/api/keys/${keyId}`, { method: "DELETE" });
    setDone(true);
    window.location.reload();
  }

  if (done) return <span className="text-xs text-gray-600">Revoked</span>;

  if (confirming) {
    return (
      <span className="flex items-center gap-2 text-xs">
        <button onClick={handleRevoke} className="text-red-400 hover:text-red-300">Confirm</button>
        <button onClick={() => setConfirming(false)} className="text-gray-500 hover:text-gray-300">Cancel</button>
      </span>
    );
  }

  return (
    <button onClick={() => setConfirming(true)}
      className="text-xs text-gray-600 hover:text-red-400 transition-colors">
      Revoke
    </button>
  );
}
