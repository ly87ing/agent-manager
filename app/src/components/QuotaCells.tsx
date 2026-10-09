import { freshnessLabel, isStale, isWindowExpired, resetLabel, usageTone, windowLabel } from "../lib/agentUsage";
import type { AgentUsage } from "../lib/observed";
import { cn } from "../lib/utils";

const TONE_BAR = { ok: "bg-success", warn: "bg-warning", critical: "bg-destructive" } as const;

/** 一行放得下的额度：每个未过重置点的窗口一小段条。 */
export function QuotaCells({ usage, testId, now }: { usage: AgentUsage | null; testId: string; now: Date }) {
  const windows = (usage?.windows ?? []).filter((w) => !isWindowExpired(w.resets_at, now));
  if (!usage || windows.length === 0)
    return (
      <span className="text-xs text-muted-foreground" title="额度读自这个账号的使用记录；用它跑过一次后才有">
        用过后显示
      </span>
    );
  const stale = isStale(usage.checked_at, now);
  return (
    <div data-testid={testId} className="flex items-center gap-3" title={`${freshnessLabel(usage.checked_at, now) ?? "更新时间未知"} · 读自 ${usage.source}`}>
      {windows.map((w) => (
        <span key={w.window_minutes} className="flex flex-col gap-1 text-xs">
          <span className="flex items-center gap-1.5">
            <span className="whitespace-nowrap text-muted-foreground">{windowLabel(w.window_minutes)}</span>
            <span className="h-1.5 w-14 overflow-hidden rounded bg-muted">
              <span
                className={cn("block h-full", stale ? "bg-muted-foreground/40" : TONE_BAR[usageTone(w.used_percent)])}
                style={{ width: `${Math.min(100, Math.max(0, w.used_percent))}%` }}
              />
            </span>
            <span className={cn("w-9 tabular-nums", stale && "text-muted-foreground")}>
              {Math.round(w.used_percent)}%{stale ? "?" : ""}
            </span>
          </span>
          <span className="text-[11px] text-muted-foreground">{resetLabel(w.resets_at, now) ?? "重置时间未知"}{stale ? " · 旧记录" : ""}</span>
        </span>
      ))}
    </div>
  );
}

