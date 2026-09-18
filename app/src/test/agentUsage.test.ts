import { describe, expect, it } from "vitest";
import {
  creditsLabel,
  freshnessLabel,
  isStale,
  resetLabel,
  tightestWindow,
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

  it("已经过了重置点不显示负数", () => {
    expect(resetLabel("2026-09-18T04:30:00Z", NOW)).toBe("30 分钟后重置");
    expect(resetLabel("2026-09-18T09:00:00Z", NOW)).toBe("5 小时后重置");
    expect(resetLabel("2026-09-21T04:00:00Z", NOW)).toBe("3 天后重置");
    // 重置点已过说明这条百分比早就翻篇了，得说作废而不是「随时会重置」
    expect(resetLabel("2026-09-18T03:00:00Z", NOW)).toBe("已过重置点，这个数作废");
    expect(resetLabel(null, NOW)).toBeNull();
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

  it("余额只显示两位；颜色档在 80/90 分界", () => {
    expect(creditsLabel("2443.0204250000")).toBe("2443.02");
    expect(creditsLabel(undefined)).toBeNull();
    expect(usageTone(79)).toBe("ok");
    expect(usageTone(80)).toBe("warn");
    expect(usageTone(90)).toBe("critical");
  });
});
