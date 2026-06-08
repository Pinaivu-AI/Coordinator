import { listModels } from "~/lib/coordinator";

export const revalidate = 30;

export default async function ModelsPage() {
  const { data: models } = await listModels().catch(() => ({ data: [] }));

  return (
    <div>
      <h1 className="text-2xl font-semibold mb-1">Models</h1>
      <p className="text-gray-400 text-sm mb-8">
        Models currently available from online GPU nodes.
      </p>

      {models.length === 0 ? (
        <div className="bg-gray-900 border border-gray-800 rounded-xl px-6 py-10 text-center">
          <p className="text-gray-500 text-sm">No nodes connected. Models appear here once a GPU node dials in.</p>
        </div>
      ) : (
        <div className="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-3 gap-4">
          {models.map((m) => (
            <div key={m.id} className="bg-gray-900 border border-gray-800 rounded-xl px-5 py-4">
              <div className="flex items-start justify-between mb-3">
                <div>
                  <p className="font-mono text-indigo-300 text-sm font-medium">{m.id}</p>
                  <p className="text-xs text-gray-500 mt-0.5">
                    {m.nodes_available} node{m.nodes_available !== 1 ? "s" : ""} available
                  </p>
                </div>
                <span className="w-2 h-2 rounded-full bg-green-400 mt-1" />
              </div>

              <div className="border-t border-gray-800 pt-3 grid grid-cols-2 gap-2 text-xs">
                <div>
                  <p className="text-gray-600 mb-0.5">Input / 1M tokens</p>
                  <p className="text-gray-300">
                    {(m.pricing.input_per_1m_tokens_nanox / 1e9).toFixed(6)} X
                  </p>
                </div>
                <div>
                  <p className="text-gray-600 mb-0.5">Output / 1M tokens</p>
                  <p className="text-gray-300">
                    {(m.pricing.output_per_1m_tokens_nanox / 1e9).toFixed(6)} X
                  </p>
                </div>
              </div>

              <div className="mt-3">
                <code className="text-xs bg-gray-800 text-gray-400 px-2 py-1 rounded">
                  model: &quot;{m.id}&quot;
                </code>
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
