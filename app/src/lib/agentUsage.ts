import type { AgentUsage, AgentUsageWindow } from "./observed";

/**
 * 额度的展示口径。
 *
 * 引擎只给窗口长度和已用百分比，说法留在界面：一个人看的是「5 小时用了 8%，
 * 几点回满」，不是「window_minutes=300」。
 */

/** 窗口长度 → 人话。不在常见档位上的按小时/天折算。 */
export function windowLabel(minutes: number): string {
  if (minutes === 300) return "5 小时";
  if (minutes === 10080) return "7 天";
  if (minutes === 1440) return "每天";
  if (minutes % 1440 === 0) return `${minutes / 1440} 天`;
  if (minutes % 60 === 0) return `${minutes / 60} 小时`;
  return `${minutes} 分钟`;
}

/**
 * 还剩多久重置。重置时刻已经过去说明这个窗口早就翻篇了——多半是助手很久没记账，
 * 那条百分比已经作废，得说清楚，不能含糊成「随时会重置」让人以为还在计。
 */
export function resetLabel(resetsAt: string | null, now: Date = new Date()): string | null {
  if (!resetsAt) return null;
  const target = new Date(resetsAt).getTime();
  if (Number.isNaN(target)) return null;
  const minutes = Math.round((target - now.getTime()) / 60000);
  if (minutes <= 0) return "已过重置点，这个数作废";
  if (minutes < 60) return `${minutes} 分钟后重置`;
  const hours = Math.round(minutes / 60);
  if (hours < 48) return `${hours} 小时后重置`;
  return `${Math.round(hours / 24)} 天后重置`;
}

/** 数字有多旧。助手不写就不更新，所以过旧要说出来，别让人以为是实时的。 */
export function freshnessLabel(checkedAt: string | null, now: Date = new Date()): string | null {
  if (!checkedAt) return null;
  const at = new Date(checkedAt).getTime();
  if (Number.isNaN(at)) return null;
  const minutes = Math.round((now.getTime() - at) / 60000);
  if (minutes < 2) return "刚刚更新";
  if (minutes < 60) return `${minutes} 分钟前更新`;
  const hours = Math.round(minutes / 60);
  if (hours < 48) return `${hours} 小时前更新`;
  return `${Math.round(hours / 24)} 天前更新`;
}

/** 超过这个天数就提醒数字可能已经不准了。 */
const STALE_HOURS = 24;

export function isStale(checkedAt: string | null, now: Date = new Date()): boolean {
  if (!checkedAt) return true;
  const at = new Date(checkedAt).getTime();
  if (Number.isNaN(at)) return true;
  return now.getTime() - at > STALE_HOURS * 3600_000;
}

/** 最吃紧的那个窗口：总览一行只放得下一个数字，放最接近用完的那个。 */
export function tightestWindow(usage: AgentUsage | null | undefined): AgentUsageWindow | null {
  if (!usage || usage.windows.length === 0) return null;
  return usage.windows.reduce((worst, item) => (item.used_percent > worst.used_percent ? item : worst));
}

/** 余额是长小数串（"2443.0204250000"），显示两位就够。 */
export function creditsLabel(credits: string | undefined): string | null {
  if (!credits) return null;
  const value = Number(credits);
  if (Number.isNaN(value)) return credits;
  return value.toFixed(2);
}

/** 用量的颜色档：只分「正常 / 快满了 / 基本用光」三档。 */
export function usageTone(usedPercent: number): "ok" | "warn" | "critical" {
  if (usedPercent >= 90) return "critical";
  if (usedPercent >= 80) return "warn";
  return "ok";
}
