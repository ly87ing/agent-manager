import { useQueries } from "@tanstack/react-query";
import { useState } from "react";
import { Loader2, PlayCircle, RefreshCw, WifiOff } from "lucide-react";
import { ErrorPanel } from "../components/ErrorPanel";
import { FreshnessBadge } from "../components/FreshnessBadge";
import { PageError, PageHeader, PageLoading } from "../components/PageState";
import type { AuditData, EngineElement, Envelope, FreshnessState, StatusData } from "../lib/contract";
import { runEngine, toDiagnostic } from "../lib/engine";
import { engineKeys, useEngineQuery, useRefreshUpstream } from "../lib/queries";
import { useExecutions } from "../lib/execution";
import { useExecuteApply, usePlanPreview } from "../lib/usePlanPreview";
import { cn, formatCheckedAt, humanizeCacheAge, isCacheStale } from "../lib/utils";

/** 提案 §4.12：behind → unknown → current → ahead；not-applicable 无上游概念，不进表。 */
const FRESHNESS_SORT_RANK: Record<FreshnessState, number> = {
  behind: 0,
  unknown: 1,
  current: 2,
  ahead: 3,
  "not-applicable": 4,
};

const DOMAINS = [
  { key: "agents", label: "Agents" },
  { key: "mcp", label: "MCP" },
  { key: "skills", label: "Skills" },
] as const;

export function sortByFreshness(elements: EngineElement[]): EngineElement[] {
  return elements
    .filter((element) => element.freshness.state !== "not-applicable")
    .slice()
    .sort((a, b) => {
      const rank = FRESHNESS_SORT_RANK[a.freshness.state] - FRESHNESS_SORT_RANK[b.freshness.state];
      return rank !== 0 ? rank : a.id.localeCompare(b.id);
    });
}

function shortenVersion(value: string | null): string {
  if (!value) return "?";
  return value.length > 16 ? `${value.slice(0, 7)}…` : value;
}

export default function UpdatesPage() {
  const auditQuery = useEngineQuery<AuditData>("system:audit");
  const statusResults = useQueries({
    queries: DOMAINS.map((domain) => ({
      queryKey: engineKeys.command(`${domain.key}:status`),
      queryFn: () => runEngine<StatusData>(`${domain.key}:status`),
    })),
  });
  const refreshAll = useRefreshUpstream();
  const refreshOne = useRefreshUpstream();
  const [pendingRowId, setPendingRowId] = useState<string | null>(null);
  const preview = usePlanPreview();
  const executeApply = useExecuteApply();
  const { execute } = useExecutions();

  const anyPending = auditQuery.isPending || statusResults.every((result) => result.isPending);
  if (anyPending) return <PageLoading />;

  const offline = auditQuery.data?.mode.offline ?? false;
  const failed = statusResults.filter((result) => result.isError);
  const allFailed = statusResults.length > 0 && failed.length === statusResults.length;
  if (allFailed) return <PageError title="更新中心数据加载失败" error={failed[0].error} />;

  const elements = sortByFreshness(
    statusResults.flatMap((result) => (result.data ? result.data.data.elements : [])),
  );
  const behindElements = elements.filter((element) => element.freshness.state === "behind");

  /** 行内收敛：仅针对选定元素执行 plan 与 apply（阶段 A 真实范围控制）。 */
  const openRowApply = (element: EngineElement) => {
    preview.open({
      title: `更新 ${element.id}`,
      subtitle: "仅更新此组件；目标被外部修改时会拒绝覆盖。",
      planCommands: [{ command: `${element.domain}:plan`, args: ["--only", element.id] }],
      requireConfirmText: element.domain === "agents" ? element.id : undefined,
      confirmLabel: "更新此项",
      execute: executeApply({
        command: `${element.domain}:apply`,
        label: `更新 ${element.id}`,
        extraArgs: ["--only", element.id],
      }),
    });
  };

  /** 顶部聚合更新：按域收集全部落后组件的 --only plan 聚合预览 → 顺序执行各域 scoped apply。 */
  const openUpdateAll = () => {
    const domainGroups = DOMAINS.map((domain) => {
      const items = behindElements.filter((el) => el.domain === domain.key);
      return {
        key: domain.key,
        label: domain.label,
        items,
        ids: items.map((el) => el.id),
      };
    }).filter((g) => g.items.length > 0);

    const planCommands = domainGroups.map((g) => ({
      command: `${g.key}:plan`,
      args: g.ids.flatMap((id) => ["--only", id]),
    }));

    preview.open({
      title: "全部更新",
      subtitle: "预览聚合了各域可更新组件的操作；实际执行将按域逐个更新各落后组件；目标被外部修改时会拒绝覆盖。",
      planCommands,
      confirmLabel: "全部更新",
      execute: async (dryRun: boolean): Promise<Envelope | null> => {
        let lastEnvelope: Envelope | null = null;
        for (const group of domainGroups) {
          const envelope = await execute({
            command: `${group.key}:apply`,
            args: [...group.ids.flatMap((id) => ["--only", id]), ...(dryRun ? ["--dry-run"] : [])],
            label: `更新 ${group.label}`,
            dryRun,
          });
          if (!envelope || !envelope.ok) {
            return envelope;
          }
          lastEnvelope = envelope;
        }
        return lastEnvelope;
      },
    });
  };

  return (
    <div className="space-y-4">
      <PageHeader
        title="更新"
        description="各组件与官方最新版本的对比（按 可更新 → 未检查 → 已是最新 → 比官方新 排序）。平时只读本地检查记录；点「检查更新」才联网。"
      />

      {offline && (
        <div
          role="status"
          className="flex items-center gap-2 rounded-lg border border-amber-200 bg-amber-50 px-4 py-2 text-sm text-amber-800"
        >
          <WifiOff className="h-4 w-4 shrink-0" aria-hidden />
          离线模式：显示的是上次检查的结果（只读），检查更新已禁用。
        </div>
      )}

      {statusResults.map((result, index) =>
        result.isError ? (
          <ErrorPanel
            key={DOMAINS[index].key}
            title={`${DOMAINS[index].label} status 加载失败（该域已跳过）`}
            errors={[toDiagnostic(result.error)]}
          />
        ) : null,
      )}

      <div className="flex justify-end gap-2">
        <button
          type="button"
          data-testid="update-all-button"
          disabled={behindElements.length === 0}
          onClick={openUpdateAll}
          className={cn(
            "inline-flex items-center gap-2 rounded-md px-4 py-2 text-sm font-medium text-white transition-colors",
            behindElements.length === 0 ? "cursor-not-allowed bg-slate-300" : "bg-emerald-600 hover:bg-emerald-700",
          )}
        >
          <PlayCircle className="h-4 w-4" aria-hidden />
          全部更新（{behindElements.length}）
        </button>
        <button
          type="button"
          data-testid="refresh-all-button"
          disabled={refreshAll.isPending || offline}
          onClick={() => refreshAll.mutate(undefined)}
          className={cn(
            "inline-flex items-center gap-2 rounded-md px-4 py-2 text-sm font-medium text-white transition-colors",
            refreshAll.isPending || offline ? "cursor-not-allowed bg-slate-300" : "bg-indigo-600 hover:bg-indigo-700",
          )}
        >
          {refreshAll.isPending ? (
            <Loader2 className="h-4 w-4 animate-spin" aria-hidden />
          ) : (
            <RefreshCw className="h-4 w-4" aria-hidden />
          )}
          检查全部更新
        </button>
      </div>

      <section className="overflow-x-auto rounded-lg border border-slate-200 bg-white p-5 shadow-sm">
        <table className="w-full text-left text-sm">
          <thead>
            <tr className="border-b border-slate-200 text-xs text-slate-500">
              <th className="py-2 pr-4 font-medium">元素</th>
              <th className="py-2 pr-4 font-medium">当前 → 最新</th>
              <th className="py-2 pr-4 font-medium">状态</th>
              <th className="py-2 pr-4 font-medium">上次检查</th>
              <th className="py-2 pr-4 font-medium">检查结果时间</th>
              <th className="py-2 pr-4 font-medium">检查方式</th>
              <th className="py-2 font-medium">操作</th>
            </tr>
          </thead>
          <tbody>
            {elements.map((element) => {
              const stale = isCacheStale(element.freshness.cache_age_sec);
              const rowPending = refreshOne.isPending && pendingRowId === element.id;
              return (
                <tr key={element.id} data-testid={`update-row-${element.id}`} className="border-b border-slate-100 last:border-0">
                  <td className="py-2 pr-4 font-mono text-xs text-slate-700">{element.id}</td>
                  <td className="py-2 pr-4 font-mono text-xs text-slate-600">
                    {shortenVersion(element.freshness.current)} → {shortenVersion(element.freshness.upstream)}
                  </td>
                  <td className="py-2 pr-4">
                    <FreshnessBadge freshness={element.freshness} />
                  </td>
                  <td className="py-2 pr-4 text-xs text-slate-500">{formatCheckedAt(element.freshness.checked_at)}</td>
                  <td
                    className={cn("py-2 pr-4 text-xs", stale ? "text-amber-600" : "text-slate-500")}
                    title={stale ? "检查结果较旧（超过 24 小时），建议重新检查" : undefined}
                  >
                    {element.freshness.cache_age_sec !== null ? humanizeCacheAge(element.freshness.cache_age_sec) : "—"}
                  </td>
                  <td className="py-2 pr-4 font-mono text-xs text-slate-500">{element.freshness.probe ?? "—"}</td>
                  <td className="py-2">
                    <div className="flex items-center gap-1.5">
                      <button
                        type="button"
                        data-testid={`reprobe-${element.id}`}
                        disabled={offline || refreshOne.isPending}
                        onClick={() => {
                          setPendingRowId(element.id);
                          refreshOne.mutate(element.id, { onSettled: () => setPendingRowId(null) });
                        }}
                        className={cn(
                          "rounded border px-2 py-1 text-xs",
                          offline || refreshOne.isPending
                            ? "cursor-not-allowed border-slate-200 text-slate-300"
                            : "border-indigo-200 text-indigo-600 hover:bg-indigo-50",
                        )}
                      >
                        {rowPending ? "检查中…" : "重新检查"}
                      </button>
                      {(element.freshness.state === "behind" || element.freshness.state === "unknown") && (
                        <button
                          type="button"
                          data-testid={`update-${element.id}`}
                          disabled={offline && element.domain === "agents"}
                          title={
                            offline && element.domain === "agents"
                              ? "离线模式下不可用（升级/安装需要联网）"
                              : "更新此项（会先预览将要执行的操作）"
                          }
                          onClick={() => openRowApply(element)}
                          className={cn(
                            "rounded border px-2 py-1 text-xs",
                            offline && element.domain === "agents"
                              ? "cursor-not-allowed border-slate-200 text-slate-300"
                              : "border-emerald-200 text-emerald-700 hover:bg-emerald-50",
                          )}
                        >
                          更新此项
                        </button>
                      )}
                    </div>
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
        {elements.length === 0 && <p className="py-4 text-center text-sm text-slate-400">没有需要检查更新的组件</p>}
      </section>

      {preview.modal}
    </div>
  );
}
