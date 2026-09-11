import { useState } from "react";
import { ChevronDown, ChevronRight, PlayCircle } from "lucide-react";
import { DriftExplanationPanel } from "../components/DriftExplanationPanel";
import { SourceDefinitionsPanel } from "../components/SourceDefinitionsPanel";
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
        <h3 className="text-sm font-semibold text-slate-700">规则生成状态</h3>
        <StateBadge state={element.state} />
      </div>
      <dl className="mt-3 space-y-1.5 text-xs text-slate-600">
        <div className="flex gap-2">
          <dt className="text-slate-400">源文件目录</dt>
          <dd className="font-mono">{spec.source_dir ?? "—"}</dd>
        </div>
        <div className="flex gap-2">
          <dt className="text-slate-400">生成结果目录</dt>
          <dd className="font-mono">{spec.dist_dir ?? "—"}</dd>
        </div>
        <div className="flex gap-2">
          <dt className="text-slate-400">源文件最近改动</dt>
          <dd className="font-mono">
            {observed.src_latest_mtime ? new Date(observed.src_latest_mtime * 1000).toLocaleString("zh-CN", { hour12: false }) : "—"}
          </dd>
        </div>
        <div className="flex gap-2">
          <dt className="text-slate-400">生成结果</dt>
          <dd className="font-mono">{observed.products?.length ?? 0} 个</dd>
        </div>
      </dl>
      {behind && (
        <div className="mt-3 rounded-md border border-amber-200 bg-amber-50 px-3 py-2 text-xs text-amber-800">
          生成结果落后于源文件：
          {stale.length > 0 && <span className="block font-mono">需要重新生成:{stale.join(", ")}</span>}
          {missing.length > 0 && <span className="block font-mono">缺少生成结果:{missing.join(", ")}</span>}
          点击「立即同步」重新生成并分发。
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
  if (!query.data.ok) {
    return (
      <div className="space-y-4">
        <PageHeader title="规则" description="各助手的规则文件清单与生成状态（规则没有官方版本概念；同步 = 生成 + 分发）。" />
        <ErrorPanel title="rules:status 错误" errors={query.data.errors} />
      </div>
    );
  }

  const elements = query.data.data.elements;
  const dist = elements.find((element) => element.id === "rules:dist") ?? null;
  const targets = elements.filter((element) => element.id !== "rules:dist");

  return (
    <div className="space-y-4">
      <PageHeader title="规则" description="各助手的规则文件清单与生成状态（规则没有官方版本概念；同步 = 生成 + 分发）。" />
      {query.data.errors.length > 0 && <ErrorPanel title="rules:status 错误" errors={query.data.errors} />}
      <SourceDefinitionsPanel domain="rules" />

      <div className="flex justify-end">
        <button
          type="button"
          data-testid="apply-rules-button"
          onClick={() =>
            preview.open({
              title: "同步规则",
              subtitle: "重新生成规则并分发到各助手（执行前自动备份）。",
              planCommands: ["rules:plan"],
              confirmLabel: "立即同步",
              execute: executeApply({ command: "rules:apply", label: "同步规则" }),
            })
          }
          className="inline-flex items-center gap-2 rounded-md bg-indigo-600 px-4 py-2 text-sm font-medium text-white hover:bg-indigo-700"
        >
          <PlayCircle className="h-4 w-4" aria-hidden /> 立即同步
        </button>
      </div>

      {dist && <DistFreshnessCard element={dist} />}

      <section className="overflow-x-auto rounded-lg border border-slate-200 bg-white p-5 shadow-sm">
        <table className="w-full text-left text-sm"><thead>
            <tr className="border-b border-slate-200 text-xs text-slate-500">
              <th className="py-2 pr-4 font-medium">名称</th>
              <th className="py-2 pr-4 font-medium">来源文件</th>
              <th className="py-2 pr-4 font-medium">写入位置</th>
              <th className="py-2 pr-4 font-medium">生成</th>
              <th className="py-2 font-medium">同步状态</th>
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
          {buildStale ? <span className="text-amber-600">需重新生成</span> : <span className="text-slate-400">已是最新</span>}
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
