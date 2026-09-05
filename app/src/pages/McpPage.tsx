import { useState } from "react";
import { Check, X } from "lucide-react";
import { DriftExplanationPanel } from "../components/DriftExplanationPanel";
import { ErrorPanel } from "../components/ErrorPanel";
import { FreshnessBadge } from "../components/FreshnessBadge";
import { MatrixCell } from "../components/MatrixCell";
import { PageError, PageHeader, PageLoading } from "../components/PageState";
import { StateBadge } from "../components/StateBadge";
import type { EngineElement, StatusData } from "../lib/contract";
import { agentCellsOf, type McpObserved, type McpSpec } from "../lib/observed";
import { useEngineQuery } from "../lib/queries";
import { cn } from "../lib/utils";

function EnvResolutionList({ element }: { element: EngineElement }) {
  const observed = element.observed as McpObserved;
  const env = observed.env ?? {};
  const entries = Object.entries(env);
  if (entries.length === 0) {
    return <p className="text-xs text-slate-400">无 env 占位符。</p>;
  }
  return (
    <ul className="space-y-1">
      {entries.map(([name, resolution]) => (
        <li key={name} className="flex items-center gap-2 text-xs">
          {resolution.resolved ? (
            <Check className="h-3.5 w-3.5 text-emerald-600" aria-label="已解析" />
          ) : (
            <X className="h-3.5 w-3.5 text-red-500" aria-label="未解析" />
          )}
          <span className="font-mono text-slate-700">{name}</span>
          <span className={resolution.resolved ? "text-emerald-600" : "text-red-600"}>
            {resolution.resolved ? "已解析" : "未解析（渲染为占位符名）"}
          </span>
        </li>
      ))}
    </ul>
  );
}

function ServerDetail({ element }: { element: EngineElement }) {
  const spec = element.spec as McpSpec;
  const cells = agentCellsOf(element);
  const extraArgs = spec.extra_args_by_agent ?? {};
  return (
    <section data-testid={`mcp-detail-${element.id}`} className="space-y-4 rounded-lg border border-slate-200 bg-white p-5 shadow-sm">
      <div className="flex items-center justify-between gap-3">
        <h3 className="text-base font-semibold text-slate-800">{element.id}</h3>
        <div className="flex items-center gap-2">
          <StateBadge state={element.state} />
          <FreshnessBadge freshness={element.freshness} className="items-end" />
        </div>
      </div>

      <div>
        <h4 className="mb-1 text-xs font-semibold text-slate-500">启动命令</h4>
        <code className="block overflow-x-auto rounded bg-slate-900 px-3 py-2 font-mono text-xs break-all text-slate-100">
          {spec.command} {(spec.args ?? []).join(" ")}
        </code>
        <p className="mt-1 text-xs text-slate-400">
          startup_timeout_sec: {spec.startup_timeout_sec ?? "—"} · 目标 agent:{" "}
          {spec.agents === null || spec.agents === undefined ? "全部" : spec.agents.join(", ")}
        </p>
      </div>

      {Object.keys(extraArgs).length > 0 && (
        <div>
          <h4 className="mb-1 text-xs font-semibold text-slate-500">per-agent extra_args</h4>
          <ul className="space-y-1">
            {Object.entries(extraArgs).map(([agent, args]) => (
              <li key={agent} className="text-xs">
                <span className="font-mono text-slate-500">{agent}: </span>
                <code className="font-mono text-slate-700">{args.join(" ")}</code>
              </li>
            ))}
          </ul>
        </div>
      )}

      <div>
        <h4 className="mb-1 text-xs font-semibold text-slate-500">env 占位符解析（值永不显示）</h4>
        <EnvResolutionList element={element} />
      </div>

      <div>
        <h4 className="mb-1 text-xs font-semibold text-slate-500">目标配置文件</h4>
        <ul className="space-y-1">
          {Object.entries(cells).map(([agent, cell]) => (
            <li key={agent} className="flex flex-wrap items-center gap-2 text-xs">
              <span className="font-mono text-slate-500">{agent}</span>
              <code className="font-mono break-all text-slate-700">{cell.target ?? "—"}</code>
              <span className={cell.target_exists ? "text-emerald-600" : "text-slate-400"}>
                {cell.target_exists ? "存在" : "未生成"}
              </span>
            </li>
          ))}
        </ul>
      </div>

      <DriftExplanationPanel element={element} planCommand="mcp:plan" cellKind="mcp" />
    </section>
  );
}

export default function McpPage() {
  const query = useEngineQuery<StatusData>("mcp:status");
  const [selectedId, setSelectedId] = useState<string | null>(null);

  if (query.isPending) return <PageLoading />;
  if (query.isError) return <PageError title="mcp:status 调用失败" error={query.error} />;

  const elements = query.data.data.elements;
  const agentColumns = Array.from(
    new Set(elements.flatMap((element) => Object.keys(agentCellsOf(element)))),
  ).sort();
  const selected = elements.find((element) => element.id === selectedId) ?? null;

  return (
    <div className="space-y-4">
      <PageHeader
        title="MCP"
        description="server × agent 注入矩阵（只读）。单元格点击切换属单元素 apply，Phase 2 开放。"
      />
      {query.data.errors.length > 0 && <ErrorPanel title="mcp:status 错误" errors={query.data.errors} />}

      <section className="overflow-x-auto rounded-lg border border-slate-200 bg-white p-5 shadow-sm">
        <table className="w-full text-left text-sm">
          <thead>
            <tr className="border-b border-slate-200 text-xs text-slate-500">
              <th className="py-2 pr-4 font-medium">Server</th>
              {agentColumns.map((agent) => (
                <th key={agent} className="py-2 pr-4 font-mono font-medium">
                  {agent}
                </th>
              ))}
              <th className="py-2 font-medium">上游状态</th>
            </tr>
          </thead>
          <tbody>
            {elements.map((element) => {
              const cells = agentCellsOf(element);
              const isSelected = element.id === selectedId;
              return (
                <tr
                  key={element.id}
                  className={cn("border-b border-slate-100 last:border-0", isSelected && "bg-indigo-50/40")}
                >
                  <td className="py-2 pr-4">
                    <button
                      type="button"
                      onClick={() => setSelectedId(isSelected ? null : element.id)}
                      aria-expanded={isSelected}
                      className="font-mono text-xs text-indigo-600 hover:text-indigo-800"
                    >
                      {element.id}
                    </button>
                  </td>
                  {agentColumns.map((agent) => {
                    const cell = cells[agent];
                    return (
                      <td key={agent} className="py-2 pr-4">
                        {cell ? (
                          <MatrixCell state={cell.state} kind="mcp" title={`${agent}: ${cell.state}`} />
                        ) : (
                          <span className="text-xs text-slate-300">—</span>
                        )}
                      </td>
                    );
                  })}
                  <td className="py-2">
                    <FreshnessBadge freshness={element.freshness} />
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
        {elements.length === 0 && <p className="py-4 text-center text-sm text-slate-400">无 MCP server 定义</p>}
      </section>

      {selected ? (
        <ServerDetail element={selected} />
      ) : (
        <p className="text-xs text-slate-400">点击矩阵中的 server id 展开详情（命令/env 解析/目标文件/漂移解释）。</p>
      )}
    </div>
  );
}
