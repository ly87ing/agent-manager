import { useQueries } from "@tanstack/react-query";
import { AlertTriangle, CheckCircle2, PlayCircle } from "lucide-react";
import { runEngine } from "../lib/engine";
import type { StatusData } from "../lib/contract";
import { engineKeys } from "../lib/queries";
import {
  buildSyncPlan,
  groupByDomain,
  SYNC_DOMAIN_LABEL,
  SYNC_DOMAIN_ORDER,
} from "../lib/syncPlan";
import { useTakeover, type TakeoverTask } from "../lib/useTakeover";
import { cn } from "../lib/utils";

/**
 * 同步中心：一个地方看清哪些配置还没落到助手里，安全的那些一键做掉。
 *
 * 配置分散在六个域，挨个页面看差异再各点一次同步太费事。这里汇总，并把「被外部改过、
 * 要你先决定」的单独列出来——那些不进批量，得用户看清代价后到对应页面显式接管。
 */
export function SyncCenter({ onNavigate }: { onNavigate?: (page: string) => void }) {
  const results = useQueries({
    queries: SYNC_DOMAIN_ORDER.map((domain) => ({
      queryKey: engineKeys.command(`${domain}:status`),
      queryFn: () => runEngine<StatusData>(`${domain}:status`),
    })),
  });
  const sync = useTakeover();

  const loading = results.some((result) => result.isPending);
  const plan = buildSyncPlan(results.map((result) => result.data));
  const readyGroups = groupByDomain(plan.ready);
  const decisionGroups = groupByDomain(plan.decisions);

  /** 每个域一个任务，任务内逐项执行；某项失败不影响后面的，结束给汇总。 */
  const runAll = () => {
    const tasks: TakeoverTask[] = readyGroups.flatMap((group) =>
      group.items.map((item) => ({
        key: item.id,
        label: `${SYNC_DOMAIN_LABEL[item.domain] ?? item.domain} ${item.label}`,
        steps: [{ kind: "apply" as const, command: `${item.domain}:apply`, args: item.args }],
      })),
    );
    void sync.run(tasks).then((outcome) => {
      if (outcome) results.forEach((result) => void result.refetch());
    });
  };

  if (loading) {
    return (
      <section data-testid="sync-center" className="rounded-lg border bg-card p-5 shadow-sm">
        <h3 className="text-sm font-semibold text-foreground">同步状态</h3>
        <p className="mt-2 text-xs text-muted-foreground">正在检查各项配置…</p>
      </section>
    );
  }

  const allClear = plan.ready.length === 0 && plan.decisions.length === 0;

  return (
    <section data-testid="sync-center" className="rounded-lg border bg-card p-5 shadow-sm">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0">
          <h3 className="text-sm font-semibold text-foreground">同步状态</h3>
          <p className="mt-1 text-xs text-muted-foreground">
            这里一次看清所有配置有没有落到助手里，不用挨个页面翻。
          </p>
        </div>
        {plan.ready.length > 0 && (
          <button
            type="button"
            data-testid="sync-all-button"
            disabled={sync.executing}
            onClick={runAll}
            className="inline-flex shrink-0 items-center gap-1.5 rounded-md bg-primary px-4 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-60"
          >
            <PlayCircle className="h-4 w-4" aria-hidden />
            {sync.executing
              ? sync.progress
                ? `正在同步 ${sync.progress.done + 1}/${sync.progress.total}…`
                : "正在同步…"
              : `全部同步（${plan.ready.length}）`}
          </button>
        )}
      </div>

      {sync.progress && (
        <div data-testid="sync-progress" className="mt-3 rounded-md border px-3 py-2">
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

      {allClear && (
        <p data-testid="sync-all-clear" className="mt-3 flex items-center gap-1.5 text-xs text-success">
          <CheckCircle2 className="h-4 w-4" aria-hidden /> 全部已同步，没有待处理的差异。
        </p>
      )}

      {readyGroups.length > 0 && (
        <div data-testid="sync-ready" className="mt-3 space-y-2">
          <p className="text-xs font-medium text-foreground">待同步（点上面的按钮一次做完）</p>
          {readyGroups.map((group) => (
            <div key={group.domain} className="rounded-md border px-3 py-2">
              <p className="text-xs font-medium text-foreground">
                {SYNC_DOMAIN_LABEL[group.domain] ?? group.domain}
                <span className="ml-1.5 font-normal text-muted-foreground">{group.items.length} 项</span>
              </p>
              <ul className="mt-1 space-y-0.5">
                {group.items.slice(0, 6).map((item) => (
                  <li key={item.id} className="flex flex-wrap gap-1.5 text-[11px] text-muted-foreground">
                    <span className="font-mono">{item.label}</span>
                    <span>· {item.reason}</span>
                  </li>
                ))}
                {group.items.length > 6 && (
                  <li className="text-[11px] text-muted-foreground">…另有 {group.items.length - 6} 项</li>
                )}
              </ul>
            </div>
          ))}
        </div>
      )}

      {decisionGroups.length > 0 && (
        <div data-testid="sync-decisions" className="mt-3 space-y-2">
          <p className="flex items-center gap-1.5 text-xs font-medium text-warning">
            <AlertTriangle className="h-3.5 w-3.5" aria-hidden />
            要你先决定（不会自动处理）
          </p>
          <p className="text-[11px] text-muted-foreground">
            这些目标被本工具以外的方式改过，或是旧体系留下的。同步不会覆盖它们——到对应页面看清会改掉什么，再决定要不要接管。
          </p>
          {decisionGroups.map((group) => (
            <div key={group.domain} className="rounded-md border border-warning/30 bg-warning/5 px-3 py-2">
              <div className="flex flex-wrap items-center justify-between gap-2">
                <p className="text-xs font-medium text-foreground">
                  {SYNC_DOMAIN_LABEL[group.domain] ?? group.domain}
                  <span className="ml-1.5 font-normal text-muted-foreground">{group.items.length} 项</span>
                </p>
                {onNavigate && (
                  <button
                    type="button"
                    data-testid={`sync-goto-${group.domain}`}
                    onClick={() => onNavigate(pageOf(group.domain))}
                    className="rounded border px-2 py-0.5 text-[11px] text-muted-foreground hover:bg-accent"
                  >
                    去处理
                  </button>
                )}
              </div>
              <ul className="mt-1 space-y-0.5">
                {group.items.slice(0, 5).map((item) => (
                  <li key={item.id} className={cn("flex flex-wrap gap-1.5 text-[11px] text-muted-foreground")}>
                    <span className="font-mono">{item.label}</span>
                    <span>· {item.reason}</span>
                  </li>
                ))}
                {group.items.length > 5 && (
                  <li className="text-[11px] text-muted-foreground">…另有 {group.items.length - 5} 项</li>
                )}
              </ul>
            </div>
          ))}
        </div>
      )}
    </section>
  );
}

/** 域 → 该去哪个页面处理。 */
function pageOf(domain: string): string {
  if (domain === "rules" || domain === "hooks") return "rules-hooks";
  if (domain === "aliases") return "settings:aliases";
  if (domain === "profiles") return "agents";
  return domain;
}
