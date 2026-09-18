import type { AgentUsage } from "../lib/observed";
import { creditsLabel, freshnessLabel, isStale, resetLabel, usageTone, windowLabel } from "../lib/agentUsage";
import { cn } from "../lib/utils";

/**
 * 一个助手的额度：每个计量窗口一条，已用多少、几时回满。
 *
 * 数字取自助手自己写在本机的记账文件，不联网也不需要再登录一次，代价是只和它上次
 * 记账一样新——所以把「什么时候更新的」摆在旁边，过久了标出来，不冒充实时。
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

  return (
    <div data-testid={testId} className="space-y-1.5">
      {usage.windows.map((window) => {
        const tone = usageTone(window.used_percent);
        const reset = resetLabel(window.resets_at);
        return (
          <div key={window.window_minutes} className="flex items-center gap-2 text-xs">
            <span className="w-14 shrink-0 text-muted-foreground">{windowLabel(window.window_minutes)}</span>
            <span className="h-1.5 w-24 shrink-0 overflow-hidden rounded bg-muted">
              <span
                className={cn("block h-full rounded", TONE_BAR[tone])}
                style={{ width: `${Math.min(100, Math.max(0, window.used_percent))}%` }}
              />
            </span>
            <span className={cn("w-12 shrink-0 tabular-nums", TONE_TEXT[tone])}>
              {Math.round(window.used_percent)}%
            </span>
            {reset && <span className="text-muted-foreground">{reset}</span>}
          </div>
        );
      })}
      <p className="text-[11px] text-muted-foreground">
        {credits && <span className="mr-2">余额 {credits}</span>}
        {usage.plan && <span className="mr-2">套餐 {usage.plan}</span>}
        <span className={cn(stale && "text-warning")} title={`读自 ${usage.source}`}>
          {freshness ?? "不知道什么时候更新的"}
          {stale && "（可能已经不准）"}
        </span>
      </p>
    </div>
  );
}
