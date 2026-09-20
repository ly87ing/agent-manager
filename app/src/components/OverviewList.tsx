import { useQueries } from "@tanstack/react-query";
import { CheckCircle2, PlayCircle, RefreshCw } from "lucide-react";
import type { StatusData } from "../lib/contract";
import { runEngine } from "../lib/engine";
import {
  buildOverview,
  syncableOf,
  OVERVIEW_DOMAINS,
  SYNC_LABEL,
  UPDATE_LABEL,
  type OverviewItem,
} from "../lib/overviewModel";
import { engineKeys, useRefreshUpstream } from "../lib/queries";
import { usageTone } from "../lib/agentUsage";
import { useTakeover } from "../lib/useTakeover";
import { cn } from "../lib/utils";

/**
 * 总览清单：本机有哪些助手 / 实例 / MCP / 技能 / 规则 / Hooks / 快捷命令，
 * 各自什么版本、要不要更新、同步到位没有。不同步的能一键做掉。
 *
 * 刻意不出路径、摘要、逐助手明细——那些在各自页面里看；这里只回答「有什么、什么版本、
 * 要不要动手」。
 */

const UPDATE_STYLE: Record<string, string> = {
  current: "text-success",
  behind: "text-warning",
  unknown: "text-muted-foreground",
  none: "",
};

const USAGE_STYLE: Record<string, string> = {
  ok: "text-muted-foreground",
  warn: "text-warning",
  critical: "text-destructive",
};

const SYNC_STYLE: Record<string, string> = {
  ok: "text-success",
  pending: "text-warning",
  decision: "text-destructive",
  absent: "text-muted-foreground",
  disabled: "text-muted-foreground",
};

function versionText(item: OverviewItem): string {
  if (item.update === "behind" && item.version && item.upstream) {
    return `${short(item.version)} → ${short(item.upstream)}`;
  }
  if (item.version) return short(item.version);
  return "";
}

/** 摘要型版本（git 哈希）只显示短形，完整值进悬停。 */
function short(value: string): string {
  return /^[0-9a-f]{16,}$/i.test(value) ? `${value.slice(0, 7)}…` : value;
}

export function OverviewList({ onNavigate }: { onNavigate?: (page: string) => void }) {
  const results = useQueries({
    queries: OVERVIEW_DOMAINS.map(({ domain }) => ({
      queryKey: engineKeys.command(`${domain}:status`),
      queryFn: () => runEngine<StatusData>(`${domain}:status`),
    })),
  });
  const sync = useTakeover();
  const refresh = useRefreshUpstream();

  const loading = results.some((result) => result.isPending);
  const byDomain = Object.fromEntries(
    OVERVIEW_DOMAINS.map(({ domain }, index) => [domain, results[index].data]),
  );
  const groups = buildOverview(byDomain);
  const syncable = syncableOf(groups);
  const behindCount = groups.reduce(
    (sum, group) => sum + group.items.filter((item) => item.update === "behind").length,
    0,
  );
  const decisionCount = groups.reduce(
    (sum, group) => sum + group.items.filter((item) => item.sync === "decision").length,
    0,
  );

  const runSyncAll = () => {
    void sync
      .run(
        syncable.map((item) => ({
          key: `${item.domain}:${item.name}`,
          label: item.name,
          steps: [{ kind: "apply" as const, command: `${item.domain}:apply`, args: item.args }],
        })),
        "同步",
      )
      .then((outcome) => {
        if (outcome) results.forEach((result) => void result.refetch());
      });
  };

  if (loading) {
    return <p className="text-xs text-muted-foreground">正在检查各项配置…</p>;
  }

  return (
    <div className="space-y-4">
      <div className="flex flex-wrap items-center gap-2">
        {syncable.length > 0 ? (
          <button
            type="button"
            data-testid="sync-all-button"
            disabled={sync.executing}
            onClick={runSyncAll}
            className="inline-flex items-center gap-1.5 rounded-md bg-primary px-4 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-60"
          >
            <PlayCircle className="h-4 w-4" aria-hidden />
            {sync.executing
              ? sync.progress
                ? `正在同步 ${sync.progress.done + 1}/${sync.progress.total}…`
                : "正在同步…"
              : `全部同步（${syncable.length}）`}
          </button>
        ) : (
          <span data-testid="sync-all-clear" className="inline-flex items-center gap-1.5 text-sm text-success">
            <CheckCircle2 className="h-4 w-4" aria-hidden /> 全部已同步
          </span>
        )}
        <button
          type="button"
          data-testid="refresh-upstream-button"
          disabled={refresh.isPending}
          onClick={() => refresh.mutate(undefined)}
          className="inline-flex items-center gap-1.5 rounded-md border px-3 py-2 text-sm text-muted-foreground hover:bg-accent disabled:opacity-60"
        >
          <RefreshCw className={cn("h-4 w-4", refresh.isPending && "animate-spin")} aria-hidden />
          {behindCount > 0 ? `检查更新（${behindCount} 项可更新）` : "检查更新"}
        </button>
        {decisionCount > 0 && (
          <span className="text-xs text-muted-foreground">另有 {decisionCount} 项要你决定，见下方标红处</span>
        )}
      </div>

      {sync.progress && (
        <div data-testid="sync-progress" className="rounded-md border px-3 py-2">
          <p className="text-xs text-foreground">
            第 {sync.progress.done + 1} / {sync.progress.total} 项：{sync.progress.current}
          </p>
          <div className="mt-2 h-1.5 w-full overflow-hidden rounded bg-muted">
            <div
              className="h-full rounded bg-primary transition-[width] duration-200"
              style={{ width: `${(sync.progress.done / sync.progress.total) * 100}%` }}
            />
          </div>
        </div>
      )}

      {/* 七个域合在一张卡片里分组：每组一个边框加一圈内边距，十来行内容就要滚一屏。 */}
      <div className="divide-y rounded-lg border bg-card shadow-sm">
        {groups.map((group) => (
        <section key={group.domain} data-testid={`overview-${group.domain}`} className="px-4 py-3">
          <div className="mb-1 flex flex-wrap items-baseline justify-between gap-2">
            <h3 className="text-sm font-semibold text-foreground">
              {group.label}
              <span className="ml-2 text-xs font-normal text-muted-foreground">{group.items.length} 项</span>
            </h3>
            <span className={cn("text-xs", group.pending > 0 ? "text-warning" : "text-success")}>
              {group.pending > 0 ? `${group.pending} 项待处理` : "全部已同步"}
            </span>
          </div>
          <ul>
            {group.items.map((item) => (
              <li
                key={item.id}
                data-testid={`overview-item-${item.id}`}
                className="flex flex-wrap items-center gap-x-3 gap-y-1 py-1 text-xs"
              >
                <span className="min-w-0 flex-1 truncate font-mono text-foreground" title={item.id}>
                  {item.name}
                </span>
                <span
                  className="w-32 shrink-0 truncate font-mono text-muted-foreground"
                  title={item.version ?? undefined}
                >
                  {versionText(item)}
                </span>
                {/* 额度：一行只放得下一个数字，放最接近用完的那个窗口；详情在助手页 */}
                <span
                  className={cn(
                    "w-20 shrink-0 tabular-nums",
                    item.usedPercent != null &&
                      (item.usageStale ? "text-muted-foreground/60" : USAGE_STYLE[usageTone(item.usedPercent)]),
                  )}
                  title={
                    item.usedPercent == null
                      ? undefined
                      : item.usageStale
                        ? "这个数字是助手很久以前记的，可能已经不准；详情见助手页"
                        : "额度用量，详情见助手页"
                  }
                >
                  {item.usedPercent != null ? `额度 ${Math.round(item.usedPercent)}%${item.usageStale ? "?" : ""}` : ""}
                </span>
                <span className={cn("w-16 shrink-0", UPDATE_STYLE[item.update])}>{UPDATE_LABEL[item.update]}</span>
                <span className={cn("w-20 shrink-0", SYNC_STYLE[item.sync])} title={item.reason}>
                  {SYNC_LABEL[item.sync]}
                </span>
                {/* 按钮槽位固定宽度：只在部分行出现的话，其余行的状态列会被挤得对不齐 */}
                <span className="w-14 shrink-0 text-right">
                  {(item.sync === "decision" || item.update === "behind") && onNavigate && (
                    <button
                      type="button"
                      data-testid={`overview-goto-${item.id}`}
                      onClick={() => onNavigate(pageOf(item.domain))}
                      className="rounded border px-2 text-[11px] leading-5 text-muted-foreground hover:bg-accent"
                    >
                      去处理
                    </button>
                  )}
                </span>
              </li>
            ))}
          </ul>
        </section>
        ))}
      </div>
    </div>
  );
}

function pageOf(domain: string): string {
  if (domain === "rules" || domain === "hooks") return "rules-hooks";
  if (domain === "aliases") return "settings:aliases";
  if (domain === "profiles") return "agents";
  return domain;
}
