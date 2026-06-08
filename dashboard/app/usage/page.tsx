/**
 * /usage — usage analytics page.
 *
 * Requires DASHBOARD_API_KEY in env — the operator's own key used to
 * call GET /v1/usage on the coordinator.
 */
import { getUsage } from "~/lib/coordinator";
import UsageChart from "./UsageChart";

export const revalidate = 60;

const API_KEY = process.env.DASHBOARD_API_KEY ?? "";

export default async function UsagePage({
  searchParams,
}: {
  searchParams: { days?: string };
}) {
  const days = Math.min(Number(searchParams.days ?? 30), 90);

  const summary = API_KEY
    ? await getUsage(API_KEY, days).catch(() => null)
    : null;

  if (!API_KEY) {
    return (
      <div>
        <h1 className="text-2xl font-semibold mb-4">Usage</h1>
        <div className="bg-yellow-900/30 border border-yellow-700 rounded-lg px-4 py-3 text-sm text-yellow-300">
          Set <code>DASHBOARD_API_KEY</code> in your environment to view usage.
        </div>
      </div>
    );
  }

  return (
    <div>
      <div className="flex items-center justify-between mb-6">
        <div>
          <h1 className="text-2xl font-semibold mb-1">Usage</h1>
          <p className="text-gray-400 text-sm">Last {days} days</p>
        </div>
        <DaysSelector current={days} />
      </div>

      {/* Summary cards */}
      {summary && (
        <>
          <div className="grid grid-cols-2 xl:grid-cols-4 gap-4 mb-8">
            <StatCard label="Requests"       value={fmt(summary.total_requests)} />
            <StatCard label="Input tokens"   value={fmtTokens(summary.total_input_tokens)} />
            <StatCard label="Output tokens"  value={fmtTokens(summary.total_output_tokens)} />
            <StatCard label="Total cost"     value={`${(summary.total_cost_nanox / 1e6).toFixed(4)} X`} />
          </div>

          {/* Chart */}
          <div className="bg-gray-900 border border-gray-800 rounded-xl p-5 mb-8">
            <p className="text-xs text-gray-500 uppercase tracking-wide mb-4">Requests per day</p>
            <UsageChart records={summary.records} />
          </div>

          {/* Table */}
          <div className="overflow-x-auto">
            <table className="w-full text-sm border-collapse">
              <thead>
                <tr className="border-b border-gray-800 text-left text-gray-500 text-xs uppercase tracking-wide">
                  <th className="pb-2 pr-4">Date</th>
                  <th className="pb-2 pr-4">Model</th>
                  <th className="pb-2 pr-4">In tokens</th>
                  <th className="pb-2 pr-4">Out tokens</th>
                  <th className="pb-2 pr-4">Latency</th>
                  <th className="pb-2">Cost (X)</th>
                </tr>
              </thead>
              <tbody>
                {summary.records.slice(0, 100).map((r, i) => (
                  <tr key={i} className="border-b border-gray-800/60 hover:bg-gray-900/40">
                    <td className="py-2 pr-4 text-gray-500 whitespace-nowrap">
                      {new Date(r.created_at).toLocaleDateString("en-US", { month: "short", day: "numeric" })}
                    </td>
                    <td className="py-2 pr-4 font-mono text-indigo-300 text-xs">{r.model}</td>
                    <td className="py-2 pr-4 text-gray-400">{r.input_tokens.toLocaleString()}</td>
                    <td className="py-2 pr-4 text-gray-400">{r.output_tokens.toLocaleString()}</td>
                    <td className="py-2 pr-4 text-gray-500">
                      {r.latency_ms != null ? `${r.latency_ms} ms` : "—"}
                    </td>
                    <td className="py-2 text-gray-400">
                      {(r.cost_nanox / 1e6).toFixed(6)}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
            {summary.records.length > 100 && (
              <p className="text-xs text-gray-600 mt-2">
                Showing 100 of {summary.records.length} records.
              </p>
            )}
          </div>
        </>
      )}

      {!summary && (
        <p className="text-gray-500 text-sm">No usage data for this period.</p>
      )}
    </div>
  );
}

function StatCard({ label, value }: { label: string; value: string }) {
  return (
    <div className="bg-gray-900 border border-gray-800 rounded-xl px-5 py-4">
      <p className="text-xs text-gray-500 mb-1">{label}</p>
      <p className="text-xl font-semibold">{value}</p>
    </div>
  );
}

function DaysSelector({ current }: { current: number }) {
  return (
    <div className="flex gap-1">
      {[7, 30, 90].map((d) => (
        <a key={d} href={`?days=${d}`}
          className={`px-3 py-1 rounded text-sm transition-colors ${
            current === d
              ? "bg-indigo-600 text-white"
              : "text-gray-400 hover:text-white hover:bg-gray-800"
          }`}>
          {d}d
        </a>
      ))}
    </div>
  );
}

function fmt(n: number) { return n.toLocaleString(); }
function fmtTokens(n: number) {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000)     return `${(n / 1_000).toFixed(1)}K`;
  return String(n);
}
