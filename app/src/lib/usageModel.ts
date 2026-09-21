/**
 * 用量的展示口径。
 *
 * 引擎按 UTC 小时聚合，这里再按**本地时区**分组成天——存储里不掺时区，
 * 换个地方看同一份数据也不会错位。
 *
 * 费用是估算：按单价表算，不等于账单。没有单价的模型只报 token，费用留空——
 * 编一个价格比不报更糟，看的人不会发现算错了。
 */

export interface UsageRow {
  /** UTC 小时，形如 2026-09-20T02:00:00Z。 */
  hour: string;
  agent: string;
  model: string;
  input: number;
  output: number;
  cache_write: number;
  cache_read: number;
  requests: number;
}

/** 单价：美元 / 百万 token。 */
export interface ModelPrice {
  input?: number;
  output?: number;
  cache_write?: number;
  cache_read?: number;
}

export interface UsageData {
  rows: UsageRow[];
  pricing: Record<string, ModelPrice>;
  last_scan_at: string | null;
}

export interface UsageGroup {
  /** 本地日期，形如 2026-09-20。 */
  day: string;
  model: string;
  agent: string;
  input: number;
  output: number;
  cacheWrite: number;
  cacheRead: number;
  requests: number;
  /** 估算费用（美元）；没有单价时为 null。 */
  cost: number | null;
}

/** 本地日期串：用来分组，不带时区后缀。 */
export function localDay(hourUtc: string, now: (iso: string) => Date = (iso) => new Date(iso)): string {
  const date = now(hourUtc);
  if (Number.isNaN(date.getTime())) return hourUtc.slice(0, 10);
  const year = date.getFullYear();
  const month = `${date.getMonth() + 1}`.padStart(2, "0");
  const day = `${date.getDate()}`.padStart(2, "0");
  return `${year}-${month}-${day}`;
}

/**
 * 找这个模型的单价。
 *
 * 先精确匹配，再按最长前缀——模型 id 常带日期后缀（claude-haiku-4-5-20251001），
 * 不做前缀匹配就会整片算不出钱。
 */
export function priceOf(model: string, pricing: Record<string, ModelPrice>): ModelPrice | null {
  if (pricing[model]) return pricing[model];
  let best: { key: string; price: ModelPrice } | null = null;
  for (const [key, price] of Object.entries(pricing)) {
    if (model.startsWith(key) && (!best || key.length > best.key.length)) {
      best = { key, price };
    }
  }
  return best?.price ?? null;
}

/** 一桶用量的估算费用；缺任一相关单价就整体不报，不用 0 冒充。 */
export function costOf(group: Omit<UsageGroup, "cost" | "day">, price: ModelPrice | null): number | null {
  if (!price) return null;
  const per = (tokens: number, unit: number | undefined) =>
    unit === undefined ? null : (tokens / 1_000_000) * unit;
  const parts = [
    per(group.input, price.input),
    per(group.output, price.output),
    per(group.cacheWrite, price.cache_write),
    per(group.cacheRead, price.cache_read),
  ];
  // 有 token 却没有对应单价 → 这桶算不准，不给数
  const needed = [group.input, group.output, group.cacheWrite, group.cacheRead];
  for (let i = 0; i < parts.length; i += 1) {
    if (needed[i] > 0 && parts[i] === null) return null;
  }
  return parts.reduce<number>((sum, part) => sum + (part ?? 0), 0);
}

/** 按「本地日 + 助手 + 模型」汇总，新的在前。 */
export function groupUsage(data: UsageData | undefined): UsageGroup[] {
  if (!data) return [];
  const buckets = new Map<string, UsageGroup>();
  for (const row of data.rows) {
    const day = localDay(row.hour);
    const key = `${day}/${row.agent}/${row.model}`;
    const current =
      buckets.get(key) ??
      {
        day,
        agent: row.agent,
        model: row.model,
        input: 0,
        output: 0,
        cacheWrite: 0,
        cacheRead: 0,
        requests: 0,
        cost: null,
      };
    current.input += row.input;
    current.output += row.output;
    current.cacheWrite += row.cache_write;
    current.cacheRead += row.cache_read;
    current.requests += row.requests;
    buckets.set(key, current);
  }
  return [...buckets.values()]
    .map((group) => ({ ...group, cost: costOf(group, priceOf(group.model, data.pricing)) }))
    .sort((a, b) => (a.day === b.day ? a.model.localeCompare(b.model) : b.day.localeCompare(a.day)));
}

/** 合计：费用只把能算的加起来，另外报有多少桶算不出。 */
export function totalsOf(groups: UsageGroup[]): {
  input: number;
  output: number;
  cacheWrite: number;
  cacheRead: number;
  requests: number;
  cost: number;
  unpricedModels: string[];
} {
  const unpriced = new Set<string>();
  const totals = groups.reduce(
    (acc, group) => {
      acc.input += group.input;
      acc.output += group.output;
      acc.cacheWrite += group.cacheWrite;
      acc.cacheRead += group.cacheRead;
      acc.requests += group.requests;
      if (group.cost === null) unpriced.add(group.model);
      else acc.cost += group.cost;
      return acc;
    },
    { input: 0, output: 0, cacheWrite: 0, cacheRead: 0, requests: 0, cost: 0 },
  );
  return { ...totals, unpricedModels: [...unpriced].sort() };
}

/** 只保留最近 N 天（按本地日）；days 为 0 表示全部。 */
export function withinDays(groups: UsageGroup[], days: number, today = new Date()): UsageGroup[] {
  if (days <= 0) return groups;
  const cutoff = new Date(today);
  cutoff.setDate(cutoff.getDate() - (days - 1));
  const limit = localDay(cutoff.toISOString());
  return groups.filter((group) => group.day >= limit);
}

/** 助手与模型筛选共用同一份分组结果，汇总卡和明细表因此不会口径分叉。 */
export function filterUsage(groups: UsageGroup[], agent: string, model: string): UsageGroup[] {
  return groups.filter(
    (group) => (!agent || group.agent === agent) && (!model || group.model === model),
  );
}

/** token 数读起来太长，按千/百万缩写。 */
export function shortTokens(value: number): string {
  if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(1)}M`;
  if (value >= 1_000) return `${(value / 1_000).toFixed(1)}k`;
  return `${value}`;
}

/** 费用：小额也要看得出不是 0。 */
export function formatCost(value: number | null): string {
  if (value === null) return "—";
  if (value > 0 && value < 0.01) return "<$0.01";
  return `$${value.toFixed(2)}`;
}
