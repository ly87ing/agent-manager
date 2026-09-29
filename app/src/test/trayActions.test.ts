import { describe, expect, it } from "vitest";
import { isTrayAction, subscribeTrayActions } from "../lib/trayActions";

describe("托盘动作", () => {
  it("只认识约定的动作：搜索与打开账号", () => {
    expect(isTrayAction("search")).toBe(true);
    expect(isTrayAction({ type: "open-account", agentName: "Claude Code", accountId: "profiles:work", accountName: "工作账号" })).toBe(true);
    expect(isTrayAction({ type: "open-account", accountId: "profiles:work" })).toBe(false);
    // 旧的切换主配置与环境动作已退役
    expect(isTrayAction("takeover")).toBe(false);
    expect(isTrayAction({ type: "switch-connection", agentId: "claude-code", agentName: "Claude Code", profile: "", profileLabel: "官方账号" })).toBe(false);
    expect(isTrayAction("quit")).toBe(false);
    expect(isTrayAction(undefined)).toBe(false);
  });

  it("没有事件通道时静默跳过，取消订阅也不报错", () => {
    const unsubscribe = subscribeTrayActions(() => {
      throw new Error("不应被调用");
    });
    expect(() => unsubscribe()).not.toThrow();
  });
});
