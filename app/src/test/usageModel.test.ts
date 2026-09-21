import { describe, expect, it } from "vitest";
import {
  costOf,
  formatCost,
  filterUsage,
  groupUsage,
  priceOf,
  totalsOf,
  withinDays,
  type UsageData,
} from "../lib/usageModel";

/**
 * 费用算错了没人会发现，所以这里锁的是「宁可不报也不报错的数」：
 * 缺单价就留空，不拿 0 冒充；带日期后缀的模型 id 也要能命中单价。
 */

const PRICING = {
  "claude-opus-5": { input: 5, output: 25, cache_write: 6.25, cache_read: 0.5 },
  "claude-haiku-4-5": { input: 1, output: 5, cache_write: 1.25, cache_read: 0.1 },
};

function data(rows: UsageData["rows"], pricing = PRICING): UsageData {
  return { rows, pricing, last_scan_at: null };
}

describe("用量口径", () => {
  it("单价按最长前缀匹配：带日期后缀的模型 id 也能命中", () => {
    expect(priceOf("claude-opus-5", PRICING)?.input).toBe(5);
    expect(priceOf("claude-haiku-4-5-20251001", PRICING)?.input).toBe(1);
    expect(priceOf("glm-5.2", PRICING)).toBeNull();
  });

  it("缺单价的模型不报费用，也不拿 0 冒充", () => {
    const group = { agent: "claude", model: "x", input: 100, output: 10, cacheWrite: 0, cacheRead: 0, requests: 1 };
    expect(costOf(group, null)).toBeNull();
    // 有 output token 却没有 output 单价 → 整桶算不准，不给数
    expect(costOf(group, { input: 5 })).toBeNull();
    // 那一项本来就是 0，缺它的单价不影响结果
    expect(costOf({ ...group, output: 0 }, { input: 5 })).toBeCloseTo(0.0005, 6);
    expect(formatCost(null)).toBe("—");
  });

  it("按百万 token 计价，四项各按各的单价", () => {
    const group = {
      agent: "claude",
      model: "claude-opus-5",
      input: 1_000_000,
      output: 1_000_000,
      cacheWrite: 1_000_000,
      cacheRead: 1_000_000,
      requests: 1,
    };
    // 5 + 25 + 6.25 + 0.5
    expect(costOf(group, PRICING["claude-opus-5"])).toBeCloseTo(36.75, 6);
  });

  it("同一天同模型的多个小时合成一行，请求数累加", () => {
    const groups = groupUsage(
      data([
        { hour: "2026-09-20T02:00:00Z", agent: "claude", model: "claude-opus-5", input: 10, output: 1, cache_write: 0, cache_read: 0, requests: 1 },
        { hour: "2026-09-20T03:00:00Z", agent: "claude", model: "claude-opus-5", input: 20, output: 2, cache_write: 0, cache_read: 0, requests: 2 },
      ]),
    );
    expect(groups).toHaveLength(1);
    expect(groups[0].input).toBe(30);
    expect(groups[0].requests).toBe(3);
  });

  it("合计只加能算的，另外点名哪些模型没单价", () => {
    const groups = groupUsage(
      data([
        { hour: "2026-09-20T02:00:00Z", agent: "claude", model: "claude-opus-5", input: 1_000_000, output: 0, cache_write: 0, cache_read: 0, requests: 1 },
        { hour: "2026-09-20T02:00:00Z", agent: "codex", model: "gpt-5.6-sol", input: 9_000_000, output: 0, cache_write: 0, cache_read: 0, requests: 1 },
      ]),
    );
    const totals = totalsOf(groups);
    expect(totals.input).toBe(10_000_000);
    expect(totals.cost).toBeCloseTo(5, 6);
    expect(totals.unpricedModels).toEqual(["gpt-5.6-sol"]);
  });

  it("时间范围按本地日筛，0 表示全部", () => {
    const today = new Date("2026-09-20T12:00:00");
    const groups = [
      { day: "2026-09-20", agent: "a", model: "m", input: 1, output: 0, cacheWrite: 0, cacheRead: 0, requests: 1, cost: null },
      { day: "2026-09-14", agent: "a", model: "m", input: 1, output: 0, cacheWrite: 0, cacheRead: 0, requests: 1, cost: null },
    ];
    expect(withinDays(groups, 1, today).map((g) => g.day)).toEqual(["2026-09-20"]);
    expect(withinDays(groups, 7, today).map((g) => g.day)).toEqual(["2026-09-20", "2026-09-14"]);
    expect(withinDays(groups, 0, today)).toHaveLength(2);
  });

  it("助手与模型筛选可组合，空值表示不限制", () => {
    const groups = groupUsage(data([
      { hour: "2026-09-20T02:00:00Z", agent: "claude", model: "claude-opus-5", input: 10, output: 1, cache_write: 0, cache_read: 0, requests: 1 },
      { hour: "2026-09-20T03:00:00Z", agent: "codex", model: "gpt-5.6-sol", input: 20, output: 2, cache_write: 0, cache_read: 0, requests: 2 },
    ]));

    expect(filterUsage(groups, "", "")).toHaveLength(2);
    expect(filterUsage(groups, "codex", "").map((group) => group.model)).toEqual(["gpt-5.6-sol"]);
    expect(filterUsage(groups, "claude", "gpt-5.6-sol")).toEqual([]);
  });

  it("小额费用看得出不是零", () => {
    expect(formatCost(0)).toBe("$0.00");
    expect(formatCost(0.0004)).toBe("<$0.01");
    expect(formatCost(12.345)).toBe("$12.35");
  });
});
