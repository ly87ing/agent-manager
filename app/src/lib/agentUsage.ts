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
 * 这个窗口的计量区间是不是已经结束。
 *
 * 结束了就说明额度早已回满、那条百分比是上一轮的残值——Claude Code 自己的状态栏
 * 也是一到重置点就把这个窗口丢掉，不会继续摆一个作废的数。
 */
export function isWindowExpired(resetsAt: string | null, now: Date = new Date()): boolean {
  if (!resetsAt) return false;
  const target = new Date(resetsAt).getTime();
  if (Number.isNaN(target)) return false;
  return target <= now.getTime();
}

/** 还剩多久重置。已经过了重置点的窗口由 isWindowExpired 单独处理，这里不给说法。 */
export function resetLabel(resetsAt: string | null, now: Date = new Date()): string | null {
  if (!resetsAt) return null;
  const target = new Date(resetsAt).getTime();
  if (Number.isNaN(target)) return null;
  const minutes = Math.round((target - now.getTime()) / 60000);
  if (minutes <= 0) return null;
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

/**
 * 最吃紧的那个窗口：总览一行只放得下一个数字，放最接近用完的那个。
 * 已经过了重置点的窗口不参与——那条百分比是上一轮的残值。
 */
export function tightestWindow(
  usage: AgentUsage | null | undefined,
  now: Date = new Date(),
): AgentUsageWindow | null {
  if (!usage) return null;
  const live = usage.windows.filter((item) => !isWindowExpired(item.resets_at, now));
  if (live.length === 0) return null;
  return live.reduce((worst, item) => (item.used_percent > worst.used_percent ? item : worst));
}

/** 托盘只放最吃紧的有效窗口；旧记录明确标出，避免把缓存当实时额度。 */
export function trayUsageSummary(usage: AgentUsage | null | undefined, now: Date = new Date()): string | null {
  const window = tightestWindow(usage, now);
  if (!window) return null;
  return `${windowLabel(window.window_minutes)}已用 ${Math.round(window.used_percent)}%${isStale(usage?.checked_at ?? null, now) ? "（旧记录）" : "（缓存）"}`;
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
