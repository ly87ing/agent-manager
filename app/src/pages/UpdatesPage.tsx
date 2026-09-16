import { useQueries } from "@tanstack/react-query";
import { useRef, useState } from "react";
import { Loader2, PlayCircle, RefreshCw, WifiOff } from "lucide-react";
import { ErrorPanel } from "../components/ErrorPanel";
import { FreshnessBadge } from "../components/FreshnessBadge";
import { PageError, PageHeader, PageLoading } from "../components/PageState";
import type { AuditData, EngineElement, FreshnessState, StatusData } from "../lib/contract";
import { runEngine, toDiagnostic } from "../lib/engine";
import { engineKeys, useEngineQuery, useRefreshUpstream } from "../lib/queries";
import { useExecutions } from "../lib/execution";
import { useDirectApply } from "../lib/useDirectApply";
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

/** 行内更新/全部更新的显式 --action：agents→upgrade，mcp/skills→update（引擎 apply 强制要求）。 */
const CONVERGE_ACTIONS: Record<string, string> = {
  agents: "upgrade",
  mcp: "update",
  skills: "update",
};

function convergeAction(domain: string): string {
  return CONVERGE_ACTIONS[domain] ?? "update";
}

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
  const [batchUpdating, setBatchUpdating] = useState(false);
  const batchGuard = useRef(false);
  const rowApplyGuard = useRef<Set<string>>(new Set());
  const [rowApplyPending, setRowApplyPending] = useState<Set<string>>(new Set());
  const directApply = useDirectApply();
  const { execute } = useExecutions();
  const anyApplyInFlight = batchUpdating || directApply.executing || rowApplyPending.size > 0;

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

  /** 行内更新：仅针对选定元素直接执行 apply，携带显式 --action。
   * 与批量更新互斥，ref 守卫 + state 双重防止同一行连点，并驱动按钮禁用。
   * 任一行在途时，其他行亦不得启动 directApply。 */
  const runRowApply = (element: EngineElement) => {
    if (batchGuard.current || rowApplyGuard.current.size > 0 || rowApplyGuard.current.has(element.id)) return;
    rowApplyGuard.current.add(element.id);
    setRowApplyPending((prev) => new Set(prev).add(element.id));
    void directApply
      .apply({
        command: `${element.domain}:apply`,
        label: `更新 ${element.id}`,
        args: ["--only", element.id, "--action", convergeAction(element.domain)],
      })
      .finally(() => {
        rowApplyGuard.current.delete(element.id);
        setRowApplyPending((prev) => {
          const next = new Set(prev);
          next.delete(element.id);
          return next;
        });
      });
  };

  /** 顶部聚合更新：按域顺序执行各域 scoped apply；每个域的执行 toast 各自可见，
   * 首个失败即停（失败域与已成功域均有 toast，剩余域未执行），不会只执行第一域而不报告。
   * 与行内更新互斥：batch ref 占位后同一 tick 内任意行不得启动，行在途时批量也不得启动。 */
  const runUpdateAll = () => {
    const domainGroups = DOMAINS.map((domain) => {
      const items = behindElements.filter((el) => el.domain === domain.key);
      return {
        key: domain.key,
        label: domain.label,
        items,
        ids: items.map((el) => el.id),
        action: convergeAction(domain.key),
      };
    }).filter((g) => g.items.length > 0);

    if (domainGroups.length === 0 || batchGuard.current || rowApplyGuard.current.size > 0) return;
    batchGuard.current = true;
    setBatchUpdating(true);
    void (async () => {
      try {
        for (const group of domainGroups) {
          const envelope = await execute({
            command: `${group.key}:apply`,
            args: [...group.ids.flatMap((id) => ["--only", id]), "--action", group.action],
            label: `更新 ${group.label}`,
          });
          if (!envelope || !envelope.ok) return;
        }
      } finally {
        batchGuard.current = false;
        setBatchUpdating(false);
      }
    })();
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
          disabled={behindElements.length === 0 || anyApplyInFlight}
          onClick={runUpdateAll}
          className={cn(
            "inline-flex items-center gap-2 rounded-md px-4 py-2 text-sm font-medium text-white transition-colors",
            behindElements.length === 0 || anyApplyInFlight ? "cursor-not-allowed bg-slate-300" : "bg-emerald-600 hover:bg-emerald-700",
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
                          disabled={anyApplyInFlight || (offline && element.domain === "agents")}
                          title={
                            offline && element.domain === "agents"
                              ? "离线模式下不可用（升级/安装需要联网）"
                              : "更新此项：直接执行，目标被外部修改时会拒绝覆盖"
                          }
                          onClick={() => runRowApply(element)}
                          className={cn(
                            "rounded border px-2 py-1 text-xs",
                            anyApplyInFlight || (offline && element.domain === "agents")
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
    </div>
  );
}
