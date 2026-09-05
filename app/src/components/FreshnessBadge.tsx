import { cva } from "class-variance-authority";
import { ArrowRight } from "lucide-react";
import type { Freshness } from "../lib/contract";
import { cn, formatCheckedAt, humanizeCacheAge, isCacheStale } from "../lib/utils";

/**
 * 上游轴五态徽章（提案 §3.3）。
 * - not-applicable：无上游概念，不渲染徽章；
 * - unknown：一等状态，灰显，不用红色制造焦虑；
 * - 缓存超过 TTL（24h）标 stale 样式，不自动联网。
 */
const freshnessBadgeVariants = cva(
  "inline-flex items-center gap-1 rounded-full border px-2 py-0.5 text-xs font-medium whitespace-nowrap",
  {
    variants: {
      state: {
        current: "border-emerald-200 bg-emerald-50 text-emerald-700",
        behind: "border-amber-200 bg-amber-50 text-amber-700",
        ahead: "border-sky-200 bg-sky-50 text-sky-700",
        unknown: "border-slate-200 bg-slate-100 text-slate-500",
        "not-applicable": "",
      },
    },
  },
);

function shortVersion(value: string | null): string {
  if (!value) return "?";
  // git commit 全 hash 只显示短形（对齐提案 §4.3 短 hash 约定）
  return /^[0-9a-f]{20,}$/i.test(value) ? value.slice(0, 7) : value;
}

function freshnessLabel(freshness: Freshness): string {
  switch (freshness.state) {
    case "current":
      return "已是最新";
    case "behind":
      return `可更新 ${shortVersion(freshness.current)} → ${shortVersion(freshness.upstream)}`;
    case "ahead":
      return freshness.current ? `领先上游 ${freshness.current}` : "领先上游";
    case "unknown":
      return freshness.checked_at ? "未知" : "未知·未探测";
    case "not-applicable":
      return "";
  }
}

export function FreshnessBadge({ freshness, className }: { freshness: Freshness; className?: string }) {
  if (freshness.state === "not-applicable") {
    return null;
  }
  const stale = isCacheStale(freshness.cache_age_sec);
  const tooltip = [
    `状态: ${freshness.state}`,
    freshness.probe ? `探测源: ${freshness.probe}` : null,
    freshness.checked_at ? `上次检查: ${formatCheckedAt(freshness.checked_at)}` : "从未探测",
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
          className={cn("text-xs", stale ? "text-amber-600" : "text-slate-400")}
          data-testid="freshness-cache-age"
        >
          {humanizeCacheAge(freshness.cache_age_sec)}检查
          {stale && "（缓存陈旧）"}
        </span>
      )}
    </span>
  );
}
