import { listKeys } from "~/lib/coordinator";
import CreateKeyForm from "./CreateKeyForm";
import RevokeButton from "./RevokeButton";

export const revalidate = 0;

const ACCOUNT_ID = process.env.DASHBOARD_ACCOUNT_ID ?? "";

export default async function KeysPage() {
  const keys = ACCOUNT_ID
    ? await listKeys(ACCOUNT_ID).catch(() => [])
    : [];

  return (
    <div>
      {/* Header */}
      <div className="flex items-center justify-between mb-6">
        <div>
          <h1 className="text-2xl font-semibold mb-1">API Keys</h1>
          <p className="text-gray-400 text-sm">
            One key works for all models. Pass it as a Bearer token.
          </p>
        </div>
        {ACCOUNT_ID && <CreateKeyForm accountId={ACCOUNT_ID} />}
      </div>

      {!ACCOUNT_ID && (
        <div className="bg-yellow-900/30 border border-yellow-700 rounded-lg px-4 py-3 text-sm text-yellow-300 mb-6">
          Set <code className="bg-gray-900 px-1 rounded">DASHBOARD_ACCOUNT_ID</code> in{" "}
          <code className="bg-gray-900 px-1 rounded">.env.local</code> — or go to{" "}
          <a href="/setup" className="underline">Setup</a> to create one.
        </div>
      )}

      {/* How to use */}
      <div className="bg-gray-900 border border-gray-800 rounded-xl px-5 py-4 mb-8">
        <p className="text-xs text-gray-500 uppercase tracking-wide mb-3">How to use</p>
        <pre className="text-xs font-mono text-gray-300 overflow-x-auto whitespace-pre-wrap">{`from openai import OpenAI

client = OpenAI(
    base_url="${process.env.COORDINATOR_URL ?? "https://api.pinaivu.com"}/v1",
    api_key="sk-pnv-your-key-here"
)
response = client.chat.completions.create(
    model="qwen-72b",          # or any model from /models
    messages=[{"role": "user", "content": "Hello!"}]
)
print(response.choices[0].message.content)`}
        </pre>
      </div>

      {/* Keys table */}
      {keys.length === 0 ? (
        <p className="text-gray-500 text-sm">No keys yet. Create one above.</p>
      ) : (
        <div className="overflow-x-auto">
          <table className="w-full text-sm border-collapse">
            <thead>
              <tr className="border-b border-gray-800 text-left text-gray-500 text-xs uppercase tracking-wide">
                <th className="pb-2 pr-6">Key</th>
                <th className="pb-2 pr-6">Name</th>
                <th className="pb-2 pr-6">Created</th>
                <th className="pb-2 pr-6">Last used</th>
                <th className="pb-2" />
              </tr>
            </thead>
            <tbody>
              {keys.map((k) => (
                <tr key={k.id} className="border-b border-gray-800/60 hover:bg-gray-900/30">
                  <td className="py-3 pr-6 font-mono text-indigo-300 text-xs">
                    {k.key_prefix}••••••••
                  </td>
                  <td className="py-3 pr-6 text-gray-300">
                    {k.name ?? <span className="text-gray-600">—</span>}
                  </td>
                  <td className="py-3 pr-6 text-gray-500">{fmtDate(k.created_at)}</td>
                  <td className="py-3 pr-6 text-gray-500">
                    {k.last_used_at
                      ? fmtDate(k.last_used_at)
                      : <span className="text-gray-700">Never</span>}
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
