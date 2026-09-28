import { describe, expect, it } from "vitest";
import {
  creditsLabel,
  freshnessLabel,
  isStale,
  isWindowExpired,
  resetLabel,
  tightestWindow,
  trayUsageSummary,
  usageTone,
  windowLabel,
} from "../lib/agentUsage";
import type { AgentUsage } from "../lib/observed";

const NOW = new Date("2026-09-18T04:00:00Z");

describe("额度展示口径", () => {
  it("窗口长度说成人话，非常见档位按小时/天折算", () => {
    expect(windowLabel(300)).toBe("5 小时");
    expect(windowLabel(10080)).toBe("7 天");
    expect(windowLabel(1440)).toBe("每天");
    expect(windowLabel(4320)).toBe("3 天");
    expect(windowLabel(120)).toBe("2 小时");
    expect(windowLabel(45)).toBe("45 分钟");
  });

  it("还剩多久重置；已过重置点的窗口不给倒计时", () => {
    expect(resetLabel("2026-09-18T04:30:00Z", NOW)).toBe("30 分钟后重置");
    expect(resetLabel("2026-09-18T09:00:00Z", NOW)).toBe("5 小时后重置");
    expect(resetLabel("2026-09-21T04:00:00Z", NOW)).toBe("3 天后重置");
    // 过了重置点交给 isWindowExpired 单独处理：那一行整条不画，不在这里给说法
    expect(resetLabel("2026-09-18T03:00:00Z", NOW)).toBeNull();
    expect(resetLabel(null, NOW)).toBeNull();
  });

  it("过了重置点的窗口算作废：百分比是上一轮残值，不参与「最吃紧」", () => {
    expect(isWindowExpired("2026-09-18T03:00:00Z", NOW)).toBe(true);
    expect(isWindowExpired("2026-09-18T04:30:00Z", NOW)).toBe(false);
    // 没有重置时刻的窗口无从判断，按仍在计量处理
    expect(isWindowExpired(null, NOW)).toBe(false);

    const usage = {
      source: "/tmp/x.json",
      checked_at: "2026-09-18T03:00:00Z",
      windows: [
        // 已过重置点、数字更大，但已经作废
        { window_minutes: 300, used_percent: 90, resets_at: "2026-09-18T03:00:00Z" },
        { window_minutes: 10080, used_percent: 13, resets_at: "2026-09-21T04:00:00Z" },
      ],
    };
    expect(tightestWindow(usage, NOW)?.window_minutes).toBe(10080);

    // 全部作废时没有可报的窗口
    const allExpired = { ...usage, windows: [usage.windows[0]] };
    expect(tightestWindow(allExpired, NOW)).toBeNull();
  });

  it("数字的新鲜度照实说；超过一天标为可能不准", () => {
    expect(freshnessLabel("2026-09-18T03:59:30Z", NOW)).toBe("刚刚更新");
    expect(freshnessLabel("2026-09-18T01:00:00Z", NOW)).toBe("3 小时前更新");
    expect(freshnessLabel("2026-09-15T04:00:00Z", NOW)).toBe("3 天前更新");
    expect(isStale("2026-09-18T03:00:00Z", NOW)).toBe(false);
    expect(isStale("2026-09-16T03:00:00Z", NOW)).toBe(true);
    // 压根没有时刻时按「不知道多旧」处理，不当成新的
    expect(isStale(null, NOW)).toBe(true);
  });

  it("总览取最吃紧的那个窗口", () => {
    const usage: AgentUsage = {
      source: ".claude.json",
      checked_at: null,
      windows: [
        { window_minutes: 300, used_percent: 8, resets_at: null },
        { window_minutes: 10080, used_percent: 71, resets_at: null },
      ],
    };
    expect(tightestWindow(usage)?.used_percent).toBe(71);
    expect(tightestWindow({ ...usage, windows: [] })).toBeNull();
    expect(tightestWindow(null)).toBeNull();
  });

  it("托盘只显示有效窗口的缓存额度，旧记录有标记", () => {
    const usage: AgentUsage = {
      source: "local-cache",
      checked_at: "2026-09-18T03:00:00Z",
      windows: [
        { window_minutes: 300, used_percent: 82, resets_at: "2026-09-18T05:00:00Z" },
        { window_minutes: 10080, used_percent: 25, resets_at: "2026-09-21T04:00:00Z" },
      ],
    };
    expect(trayUsageSummary(usage, NOW)).toBe("5 小时已用 82%（缓存）");
    expect(trayUsageSummary(usage, new Date("2026-09-19T04:00:00Z"))).toBe("7 天已用 25%（旧记录）");
    expect(trayUsageSummary(usage, new Date("2026-09-22T04:00:00Z"))).toBeNull();
    expect(trayUsageSummary(null, NOW)).toBeNull();
  });

  it("余额只显示两位；颜色档在 80/90 分界", () => {
    expect(creditsLabel("2443.0204250000")).toBe("2443.02");
    expect(creditsLabel(undefined)).toBeNull();
    expect(usageTone(79)).toBe("ok");
    expect(usageTone(80)).toBe("warn");
    expect(usageTone(90)).toBe("critical");
  });
});
