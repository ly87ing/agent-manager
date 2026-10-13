import { describe, expect, it } from "vitest";
import { listTimeZones } from "../lib/launch";

describe("Launch time zones", () => {
  it("时区列表把常用时区排在前面", () => {
    const zones = listTimeZones();
    expect(zones.length).toBeGreaterThan(5);
    expect(zones).toContain("Asia/Shanghai");
    expect(zones).toContain("UTC");
    expect(zones.indexOf("Asia/Shanghai")).toBeLessThan(10);
    // 不重复
    expect(new Set(zones).size).toBe(zones.length);
  });
});
