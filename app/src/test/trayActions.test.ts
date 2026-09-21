import { describe, expect, it } from "vitest";
import { isTrayAction, subscribeTrayActions } from "../lib/trayActions";

describe("托盘动作", () => {
  it("只认识约定的动作 id", () => {
    expect(isTrayAction("search")).toBe(true);
    expect(isTrayAction("takeover")).toBe(true);
    expect(isTrayAction({
      type: "switch-connection",
      agentId: "claude-code",
      agentName: "Claude Code",
      profile: "kimi",
      profileLabel: "Kimi Code",
    })).toBe(true);
    expect(isTrayAction({ type: "switch-connection", agentId: "claude-code" })).toBe(false);
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
