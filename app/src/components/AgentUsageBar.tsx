import type { AgentUsage } from "../lib/observed";
import {
  creditsLabel,
  freshnessLabel,
  isStale,
  isWindowExpired,
  resetLabel,
  usageTone,
  windowLabel,
} from "../lib/agentUsage";
import { cn } from "../lib/utils";

/**
 * 一个助手的额度：每个计量窗口一条，已用多少、几时回满。
 *
 * 数字取自助手自己写在本机的记账文件，不联网也不需要再登录一次，代价是只和它上次
 * 记账一样新。两件事必须说清楚，否则摆出来的数会骗人：
 * - 窗口已过重置点：额度早回满了，那条百分比是上一轮的残值，不画条也不报数；
 * - 整份记账过旧：窗口没过期也不代表数还准，这期间用掉的都没算进去，标灰加问号。
 */

const TONE_BAR: Record<string, string> = {
  ok: "bg-success",
  warn: "bg-warning",
  critical: "bg-destructive",
};

const TONE_TEXT: Record<string, string> = {
  ok: "text-foreground",
  warn: "text-warning",
  critical: "text-destructive",
};

export function AgentUsageBar({ usage, testId }: { usage: AgentUsage; testId?: string }) {
  const freshness = freshnessLabel(usage.checked_at);
  const stale = isStale(usage.checked_at);
  const credits = creditsLabel(usage.credits);
  const allExpired = usage.windows.length > 0 && usage.windows.every((w) => isWindowExpired(w.resets_at));

  return (
    <div data-testid={testId} className="space-y-1.5">
      {allExpired ? (
        <p className="text-xs text-muted-foreground">
          这些窗口都过了重置点，额度已回满；下次用过这个助手才会有新数。
        </p>
      ) : (
        usage.windows.map((window) => {
          const expired = isWindowExpired(window.resets_at);
          const tone = usageTone(window.used_percent);
          const reset = resetLabel(window.resets_at);
          return (
            <div key={window.window_minutes} className="flex items-center gap-2 text-xs">
              <span className="w-14 shrink-0 text-muted-foreground">{windowLabel(window.window_minutes)}</span>
              {expired ? (
                <span className="text-muted-foreground">已重置，等下次用过才有新数</span>
              ) : (
                <>
                  <span className="h-1.5 w-24 shrink-0 overflow-hidden rounded bg-muted">
                    <span
                      className={cn("block h-full rounded", stale ? "bg-muted-foreground/40" : TONE_BAR[tone])}
                      style={{ width: `${Math.min(100, Math.max(0, window.used_percent))}%` }}
                    />
                  </span>
                  <span
                    className={cn(
                      "w-12 shrink-0 tabular-nums",
                      stale ? "text-muted-foreground" : TONE_TEXT[tone],
                    )}
                    title={stale ? "这个数是上次记账时的，之后用掉的没算进去" : undefined}
                  >
                    {Math.round(window.used_percent)}%{stale ? "?" : ""}
                  </span>
                  {reset && <span className="text-muted-foreground">{reset}</span>}
                </>
              )}
            </div>
          );
        })
      )}
      <p className="text-[11px] text-muted-foreground">
        {credits && <span className="mr-2">余额 {credits}</span>}
        {usage.plan && <span className="mr-2">套餐 {usage.plan}</span>}
        <span className={cn(stale && "text-warning")} title={`读自 ${usage.source}`}>
          {freshness ?? "不知道什么时候更新的"}
        </span>
      </p>
    </div>
  );
}
