import { enclaveHealth, listModels } from "~/lib/coordinator";

export const revalidate = 30; // refresh every 30 s

export default async function OverviewPage() {
  const [health, models] = await Promise.all([
    enclaveHealth().catch(() => null),
    listModels().catch(() => ({ data: [] })),
  ]);

  const uptime = health
    ? formatUptime(health.uptime_ms)
    : "—";

  return (
    <div>
      <h1 className="text-2xl font-semibold mb-1">Overview</h1>
      <p className="text-gray-400 text-sm mb-8">
        Live status of the Pinaivu coordinator enclave.
      </p>

      {/* Stat cards */}
      <div className="grid grid-cols-1 sm:grid-cols-2 xl:grid-cols-4 gap-4 mb-10">
        <StatCard label="Enclave status" value={health ? "Online" : "Unreachable"} accent={health ? "green" : "red"} />
        <StatCard label="Uptime"         value={uptime} />
        <StatCard label="Models live"    value={String(models.data.length)} />
        <StatCard label="On-chain ID"    value={health?.enclave_object_id ? "Registered" : "Pending"} accent={health?.enclave_object_id ? "green" : "yellow"} />
      </div>

      {/* TLS fingerprint */}
      {health?.tls_cert_fingerprint && (
        <section className="mb-10">
          <h2 className="text-sm font-medium text-gray-400 uppercase tracking-wide mb-2">
            TLS Certificate Fingerprint (SHA-256)
          </h2>
          <div className="bg-gray-900 border border-gray-800 rounded-lg px-4 py-3 font-mono text-xs text-indigo-300 break-all">
            {health.tls_cert_fingerprint}
          </div>
          <p className="text-xs text-gray-500 mt-1">
            Pin this in your HTTPS client. Cross-check against the attestation
            document to verify the cert was generated inside the Nitro Enclave.
          </p>
        </section>
      )}

      {/* Quickstart */}
      <section>
        <h2 className="text-sm font-medium text-gray-400 uppercase tracking-wide mb-3">
          Quickstart
        </h2>
        <pre className="bg-gray-900 border border-gray-800 rounded-lg px-5 py-4 text-sm text-gray-200 overflow-x-auto whitespace-pre-wrap">
{`from openai import OpenAI

client = OpenAI(
    base_url="https://api.pinaivu.com/v1",
    api_key="sk-pnv-your-key-here"
)

response = client.chat.completions.create(
    model="qwen-72b",
    messages=[{"role": "user", "content": "Hello!"}]
)
print(response.choices[0].message.content)`}
        </pre>
      </section>
    </div>
  );
}

function StatCard({
  label,
  value,
  accent = "none",
}: {
  label: string;
  value: string;
  accent?: "green" | "red" | "yellow" | "none";
}) {
  const dot: Record<string, string> = {
    green: "bg-green-400",
    red: "bg-red-400",
    yellow: "bg-yellow-400",
    none: "bg-gray-600",
  };
  return (
    <div className="bg-gray-900 border border-gray-800 rounded-xl px-5 py-4">
      <p className="text-xs text-gray-500 mb-1">{label}</p>
      <div className="flex items-center gap-2">
        <span className={`w-2 h-2 rounded-full ${dot[accent]}`} />
        <span className="text-lg font-semibold">{value}</span>
      </div>
    </div>
  );
}

function formatUptime(ms: number): string {
  const s = Math.floor(ms / 1000);
  const d = Math.floor(s / 86400);
  const h = Math.floor((s % 86400) / 3600);
  const m = Math.floor((s % 3600) / 60);
  if (d > 0) return `${d}d ${h}h`;
  if (h > 0) return `${h}h ${m}m`;
  return `${m}m`;
}
