import { useQueries } from "@tanstack/react-query";
import { useState } from "react";
import { Loader2, RefreshCw, WifiOff } from "lucide-react";
import { ErrorPanel } from "../components/ErrorPanel";
import { FreshnessBadge } from "../components/FreshnessBadge";
import { PageError, PageHeader, PageLoading } from "../components/PageState";
import type { AuditData, EngineElement, FreshnessState, StatusData } from "../lib/contract";
import { runEngine, toDiagnostic } from "../lib/engine";
import { engineKeys, useEngineQuery, useRefreshUpstream } from "../lib/queries";
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
  { key: "tools", label: "工具" },
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

  const anyPending = auditQuery.isPending || statusResults.every((result) => result.isPending);
  if (anyPending) return <PageLoading />;

  const offline = auditQuery.data?.mode.offline ?? false;
  const failed = statusResults.filter((result) => result.isError);
  const allFailed = statusResults.length > 0 && failed.length === statusResults.length;
  if (allFailed) return <PageError title="更新中心数据加载失败" error={failed[0].error} />;

  const elements = sortByFreshness(
    statusResults.flatMap((result) => (result.data ? result.data.data.elements : [])),
  );

  return (
    <div className="space-y-4">
      <PageHeader
        title="更新中心"
        description="全组件上游新鲜度总表（agents / tools / mcp / skills，按 落后 → 未知 → 已是最新 → 领先 排序）。探测只写缓存，收敛动作 Phase 2 开放。"
      />

      {offline && (
        <div
          role="status"
          className="flex items-center gap-2 rounded-lg border border-amber-200 bg-amber-50 px-4 py-2 text-sm text-amber-800"
        >
          <WifiOff className="h-4 w-4 shrink-0" aria-hidden />
          离线模式：整表为上次检查的缓存值（只读），刷新与重新探测已禁用。
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

      <div className="flex justify-end">
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
          全部刷新上游状态
        </button>
      </div>

      <section className="overflow-x-auto rounded-lg border border-slate-200 bg-white p-5 shadow-sm">
        <table className="w-full text-left text-sm">
          <thead>
            <tr className="border-b border-slate-200 text-xs text-slate-500">
              <th className="py-2 pr-4 font-medium">元素</th>
              <th className="py-2 pr-4 font-medium">current → upstream</th>
              <th className="py-2 pr-4 font-medium">状态</th>
              <th className="py-2 pr-4 font-medium">上次检查</th>
              <th className="py-2 pr-4 font-medium">缓存年龄</th>
              <th className="py-2 pr-4 font-medium">探测源</th>
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
                  <td className={cn("py-2 pr-4 text-xs", stale ? "text-amber-600" : "text-slate-500")}>
                    {element.freshness.cache_age_sec !== null
                      ? `${humanizeCacheAge(element.freshness.cache_age_sec)}${stale ? "（陈旧）" : ""}`
                      : "—"}
                  </td>
                  <td className="py-2 pr-4 font-mono text-xs text-slate-500">{element.freshness.probe ?? "—"}</td>
                  <td className="py-2">
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
                      {rowPending ? "探测中…" : "重新探测"}
                    </button>
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
        {elements.length === 0 && <p className="py-4 text-center text-sm text-slate-400">没有可探测上游的元素</p>}
      </section>
    </div>
  );
}
