/**
 * /keys — API key management page.
 *
 * Reads DASHBOARD_ACCOUNT_ID from env so the dashboard is always scoped
 * to one operator account. Set it in .env.local for dev.
 */
import { listKeys } from "~/lib/coordinator";
import CreateKeyForm from "./CreateKeyForm";
import RevokeButton from "./RevokeButton";

export const revalidate = 0; // always fresh

const ACCOUNT_ID = process.env.DASHBOARD_ACCOUNT_ID ?? "";

export default async function KeysPage() {
  const keys = ACCOUNT_ID
    ? await listKeys(ACCOUNT_ID).catch(() => [])
    : [];

  return (
    <div>
      <div className="flex items-start justify-between mb-6">
        <div>
          <h1 className="text-2xl font-semibold mb-1">API Keys</h1>
          <p className="text-gray-400 text-sm">
            Keys authenticate requests to{" "}
            <code className="text-indigo-300 text-xs bg-gray-800 px-1.5 py-0.5 rounded">
              POST /v1/chat/completions
            </code>
          </p>
        </div>
        {ACCOUNT_ID && <CreateKeyForm accountId={ACCOUNT_ID} />}
      </div>

      {!ACCOUNT_ID && (
        <div className="bg-yellow-900/30 border border-yellow-700 rounded-lg px-4 py-3 text-sm text-yellow-300 mb-6">
          Set <code>DASHBOARD_ACCOUNT_ID</code> in your environment to manage keys.
        </div>
      )}

      {keys.length === 0 ? (
        <p className="text-gray-500 text-sm">No active keys. Create one above.</p>
      ) : (
        <div className="overflow-x-auto">
          <table className="w-full text-sm border-collapse">
            <thead>
              <tr className="border-b border-gray-800 text-left text-gray-500 text-xs uppercase tracking-wide">
                <th className="pb-2 pr-6">Prefix</th>
                <th className="pb-2 pr-6">Name</th>
                <th className="pb-2 pr-6">RPM</th>
                <th className="pb-2 pr-6">Daily</th>
                <th className="pb-2 pr-6">Created</th>
                <th className="pb-2 pr-6">Last used</th>
                <th className="pb-2" />
              </tr>
            </thead>
            <tbody>
              {keys.map((k) => (
                <tr key={k.id} className="border-b border-gray-800/60 hover:bg-gray-900/40">
                  <td className="py-3 pr-6 font-mono text-indigo-300">{k.key_prefix}…</td>
                  <td className="py-3 pr-6 text-gray-300">{k.name ?? <span className="text-gray-600">—</span>}</td>
                  <td className="py-3 pr-6 text-gray-400">{k.rpm_limit}/min</td>
                  <td className="py-3 pr-6 text-gray-400">{k.daily_limit}/day</td>
                  <td className="py-3 pr-6 text-gray-500">{fmtDate(k.created_at)}</td>
                  <td className="py-3 pr-6 text-gray-500">
                    {k.last_used_at ? fmtDate(k.last_used_at) : <span className="text-gray-700">Never</span>}
                  </td>
                  <td className="py-3">
                    <RevokeButton keyId={k.id} />
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}

function fmtDate(iso: string) {
  return new Date(iso).toLocaleDateString("en-US", {
    month: "short", day: "numeric", year: "numeric",
  });
}
