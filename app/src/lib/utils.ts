import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

/** 提案 §3.3：探测缓存 TTL 默认 24h，超过即视为陈旧（stale）。 */
export const FRESHNESS_STALE_THRESHOLD_SEC = 24 * 60 * 60;

/** 把 cache_age_sec 人类化为「N 秒/分钟/小时/天前」。 */
export function humanizeCacheAge(ageSec: number): string {
  if (ageSec < 0) return "刚刚";
  if (ageSec < 60) return `${ageSec} 秒前`;
  if (ageSec < 3600) return `${Math.floor(ageSec / 60)} 分钟前`;
  if (ageSec < 86400) return `${Math.floor(ageSec / 3600)} 小时前`;
  return `${Math.floor(ageSec / 86400)} 天前`;
}

export function isCacheStale(ageSec: number | null): boolean {
  return ageSec !== null && ageSec > FRESHNESS_STALE_THRESHOLD_SEC;
}

export function formatCheckedAt(iso: string | null): string {
  if (!iso) return "从未检查";
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return iso;
  return date.toLocaleString("zh-CN", { hour12: false });
}

/** 引擎回读的 JSON 对象键顺序可能与表单不同，按值比较完整字段。 */
export function sameJson(left: unknown, right: unknown): boolean {
  if (Object.is(left, right)) return true;
  if (Array.isArray(left) || Array.isArray(right)) {
    return Array.isArray(left) && Array.isArray(right) && left.length === right.length
      && left.every((item, index) => sameJson(item, right[index]));
  }
  if (left && right && typeof left === "object" && typeof right === "object") {
    const a = left as Record<string, unknown>;
    const b = right as Record<string, unknown>;
    return Object.keys(a).length === Object.keys(b).length
      && Object.keys(a).every((key) => Object.prototype.hasOwnProperty.call(b, key) && sameJson(a[key], b[key]));
  }
  return false;
}
