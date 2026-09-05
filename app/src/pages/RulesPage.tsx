import { useState } from "react";
import { ChevronDown, ChevronRight, PlayCircle } from "lucide-react";
import { DriftExplanationPanel } from "../components/DriftExplanationPanel";
import { ErrorPanel } from "../components/ErrorPanel";
import { PageError, PageHeader, PageLoading } from "../components/PageState";
import { StateBadge } from "../components/StateBadge";
import type { EngineElement, StatusData } from "../lib/contract";
import type { RulesObserved, RulesSpec } from "../lib/observed";
import { useEngineQuery } from "../lib/queries";
import { useExecuteApply, usePlanPreview } from "../lib/usePlanPreview";
import { cn } from "../lib/utils";

/** rules:dist 构建新鲜度卡（收敛轴：src→dist 构建是否落后）。 */
function DistFreshnessCard({ element }: { element: EngineElement }) {
  const spec = element.spec as RulesSpec;
  const observed = element.observed as RulesObserved;
  const stale = observed.stale_products ?? [];
  const missing = observed.missing_products ?? [];
  const behind = element.state !== "managed-ok" || stale.length > 0 || missing.length > 0;
  return (
    <section data-testid="rules-dist-card" className="rounded-lg border border-slate-200 bg-white p-5 shadow-sm">
      <div className="flex items-center justify-between">
        <h3 className="text-sm font-semibold text-slate-700">dist 构建新鲜度</h3>
        <StateBadge state={element.state} />
      </div>
      <dl className="mt-3 space-y-1.5 text-xs text-slate-600">
        <div className="flex gap-2">
          <dt className="text-slate-400">源码目录</dt>
          <dd className="font-mono">{spec.source_dir ?? "—"}</dd>
        </div>
        <div className="flex gap-2">
          <dt className="text-slate-400">产物目录</dt>
          <dd className="font-mono">{spec.dist_dir ?? "—"}</dd>
        </div>
        <div className="flex gap-2">
          <dt className="text-slate-400">src 最新改动</dt>
          <dd className="font-mono">
            {observed.src_latest_mtime ? new Date(observed.src_latest_mtime * 1000).toLocaleString("zh-CN", { hour12: false }) : "—"}
          </dd>
        </div>
        <div className="flex gap-2">
          <dt className="text-slate-400">产物</dt>
          <dd className="font-mono">{observed.products?.length ?? 0} 个</dd>
        </div>
      </dl>
      {behind && (
        <div className="mt-3 rounded-md border border-amber-200 bg-amber-50 px-3 py-2 text-xs text-amber-800">
          构建落后源码：
          {stale.length > 0 && <span className="block font-mono">陈旧产物 {stale.join(", ")}</span>}
          {missing.length > 0 && <span className="block font-mono">缺失产物 {missing.join(", ")}</span>}
          运行 <code className="font-mono">make update</code> 重建；GUI 构建按钮在 Phase 2 提供。
        </div>
      )}
    </section>
  );
}

export default function RulesPage() {
  const query = useEngineQuery<StatusData>("rules:status");
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const preview = usePlanPreview();
  const executeApply = useExecuteApply();

  if (query.isPending) return <PageLoading />;
  if (query.isError) return <PageError title="rules:status 调用失败" error={query.error} />;

  const elements = query.data.data.elements;
  const dist = elements.find((element) => element.id === "rules:dist") ?? null;
  const targets = elements.filter((element) => element.id !== "rules:dist");

  return (
    <div className="space-y-4">
      <PageHeader title="Rules" description="rules-targets.list 目标清单与 dist 构建新鲜度（rules 无上游轴；收敛 = 构建 + 分发）。" />
      {query.data.errors.length > 0 && <ErrorPanel title="rules:status 错误" errors={query.data.errors} />}

      <div className="flex justify-end">
        <button
          type="button"
          data-testid="apply-rules-button"
          onClick={() =>
            preview.open({
              title: "收敛 Rules 域",
              subtitle: "rules:apply = src→dist 构建 + 目标分发（执行前自动快照）。",
              planCommands: ["rules:plan"],
              confirmLabel: "收敛 Rules",
              execute: executeApply({ command: "rules:apply", label: "收敛 Rules 域" }),
            })
          }
          className="inline-flex items-center gap-2 rounded-md bg-indigo-600 px-4 py-2 text-sm font-medium text-white hover:bg-indigo-700"
        >
          <PlayCircle className="h-4 w-4" aria-hidden /> 收敛此域
        </button>
      </div>

      {dist && <DistFreshnessCard element={dist} />}

      <section className="overflow-x-auto rounded-lg border border-slate-200 bg-white p-5 shadow-sm">
        <table className="w-full text-left text-sm"><thead>
            <tr className="border-b border-slate-200 text-xs text-slate-500">
              <th className="py-2 pr-4 font-medium">目标</th>
              <th className="py-2 pr-4 font-medium">源</th>
              <th className="py-2 pr-4 font-medium">目标路径</th>
              <th className="py-2 pr-4 font-medium">构建</th>
              <th className="py-2 font-medium">收敛状态</th>
            </tr>
          </thead>
          <tbody>
            {targets.map((element) => {
              const spec = element.spec as RulesSpec;
              const observed = element.observed as RulesObserved;
              const isExpanded = element.id === expandedId;
              return (
                <FragmentRow
                  key={element.id}
                  element={element}
                  name={spec.name ?? element.id}
                  source={spec.source}
                  target={observed.target}
                  buildStale={observed.build_stale === true}
                  isExpanded={isExpanded}
                  onToggle={() => setExpandedId(isExpanded ? null : element.id)}
                />
              );
            })}
          </tbody>
        </table>
      </section>

      {preview.modal}
    </div>
  );
}

function FragmentRow({
  element,
  name,
  source,
  target,
  buildStale,
  isExpanded,
  onToggle,
}: {
  element: EngineElement;
  name: string;
  source?: string;
  target?: string;
  buildStale: boolean;
  isExpanded: boolean;
  onToggle: () => void;
}) {
  return (
    <>
      <tr className={cn("border-b border-slate-100", !isExpanded && "last:border-0")}>
        <td className="py-2 pr-4">
          <button
            type="button"
            onClick={onToggle}
            aria-expanded={isExpanded}
            className="inline-flex items-center gap-1 font-mono text-xs text-indigo-600 hover:text-indigo-800"
          >
            {isExpanded ? <ChevronDown className="h-3 w-3" aria-hidden /> : <ChevronRight className="h-3 w-3" aria-hidden />}
            {name}
          </button>
        </td>
        <td className="py-2 pr-4 font-mono text-xs break-all text-slate-500">{source ?? "—"}</td>
        <td className="py-2 pr-4 font-mono text-xs break-all text-slate-600">{target ?? "—"}</td>
        <td className="py-2 pr-4 text-xs">
          {buildStale ? <span className="text-amber-600">构建落后</span> : <span className="text-slate-400">新鲜</span>}
        </td>
        <td className="py-2">
          <StateBadge state={element.state} />
        </td>
      </tr>
      {isExpanded && (
        <tr className="border-b border-slate-100 last:border-0">
          <td colSpan={5} className="py-3">
            <DriftExplanationPanel element={element} planCommand="rules:plan" />
          </td>
        </tr>
      )}
    </>
  );
}
