import { cva } from "class-variance-authority";
import { ArrowRight } from "lucide-react";
import type { Freshness } from "../lib/contract";
import { FRESHNESS_STALE_NOTE, freshnessLabel } from "../lib/ui-copy";
import { cn, formatCheckedAt, humanizeCacheAge, isCacheStale } from "../lib/utils";

/**
 * 最新版本徽章（上游轴五态，提案 §3.3；用户文案见 lib/ui-copy.ts）。
 * - not-applicable：该元素没有「官方最新版」概念，不渲染徽章；
 * - unknown（未检查）：一等状态，灰显，不用红色制造焦虑；
 * - 检查结果超过 TTL（24h）标「检查结果较旧」，不自动联网。
 */
const freshnessBadgeVariants = cva(
  "inline-flex items-center gap-1 rounded-full border px-2 py-0.5 text-xs font-medium whitespace-nowrap",
  {
    variants: {
      state: {
        current: "border-success/30 bg-success/10 text-success",
        behind: "border-warning/30 bg-warning/10 text-warning",
        ahead: "border-info/30 bg-info/10 text-info",
        unknown: "border bg-muted text-muted-foreground",
        "not-applicable": "",
      },
    },
  },
);

export function FreshnessBadge({ freshness, className }: { freshness: Freshness; className?: string }) {
  if (freshness.state === "not-applicable") {
    return null;
  }
  const stale = isCacheStale(freshness.cache_age_sec);
  // tooltip 保留工程对照（state 原值/探测方式/检查时间）
  const tooltip = [
    `状态: ${freshness.state}`,
    freshness.probe ? `检查方式: ${freshness.probe}` : null,
    freshness.checked_at ? `上次检查: ${formatCheckedAt(freshness.checked_at)}` : "从未检查",
  ]
    .filter(Boolean)
    .join(" · ");

  return (
    <span className={cn("inline-flex flex-col items-start gap-0.5", className)}>
      <span className={cn(freshnessBadgeVariants({ state: freshness.state }))} title={tooltip}>
        {freshness.state === "behind" && <ArrowRight className="h-3 w-3" aria-hidden />}
        {freshnessLabel(freshness)}
      </span>
      {freshness.cache_age_sec !== null && (
        <span
          className={cn("text-xs", stale ? "text-warning" : "text-muted-foreground")}
          data-testid="freshness-cache-age"
        >
          {humanizeCacheAge(freshness.cache_age_sec)}检查
          {stale && `（${FRESHNESS_STALE_NOTE}）`}
        </span>
      )}
    </span>
  );
}
