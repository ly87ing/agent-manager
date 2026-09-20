import { describe, expect, it } from "vitest";
import { buildMcpSyncPlan } from "../lib/mcpSync";
import { makeElement } from "./fixtures";

describe("buildMcpSyncPlan", () => {
  it("启用服务时按真实状态选择 update/add/enable，并把外部冲突标为显式接管", () => {
    const plan = buildMcpSyncPlan(makeElement({
      id: "mcp:demo",
      spec: { command: "demo", agents: null },
      policy: { enabled: true },
      observed: {
        agents: {
          current: { state: "managed-ok", assigned: true, enabled: true, actions: ["disable", "remove"] },
          stale: { state: "drifted", assigned: true, enabled: true, actions: ["update", "disable", "remove"] },
          fresh: { state: "missing", assigned: false, enabled: false, actions: ["add"] },
          paused: { state: "disabled", assigned: true, enabled: false, actions: ["enable", "remove"] },
          conflict: { state: "drifted", assigned: true, enabled: true, actions: [], blocked_reason: "存在外部非受管同名条目，拒绝操作以防止冲突" },
          blocked: { state: "not-materialized", assigned: true, enabled: true, actions: [], blocked_reason: "凭证变量未配置: DEMO_KEY" },
        },
      },
    }));

    expect(plan.rows.map(({ agent, action, takeover }) => ({ agent, action, takeover }))).toEqual([
      { agent: "stale", action: "update", takeover: false },
      { agent: "fresh", action: "add", takeover: false },
      { agent: "paused", action: "enable", takeover: false },
      { agent: "conflict", action: "update", takeover: true },
    ]);
    expect(plan.blocked).toEqual([{ agent: "blocked", reason: "凭证变量未配置: DEMO_KEY" }]);
    expect(plan.takeoverCount).toBe(1);
  });

  it("停用服务时移除已启用配置并清理无绑定的受管残留", () => {
    const plan = buildMcpSyncPlan(makeElement({
      id: "mcp:demo",
      spec: { command: "demo", agents: null },
      policy: { enabled: false },
      observed: {
        agents: {
          active: { state: "managed-ok", assigned: true, enabled: true, actions: ["disable", "remove"] },
          paused: { state: "disabled", assigned: true, enabled: false, actions: ["enable", "remove"] },
          old: { state: "leftover", assigned: false, enabled: false, actions: ["remove"] },
        },
      },
    }));

    expect(plan.rows.map(({ agent, action }) => ({ agent, action }))).toEqual([
      { agent: "active", action: "disable" },
      { agent: "old", action: "remove" },
    ]);
    expect(plan.removeCount).toBe(1);
    expect(plan.blocked).toEqual([]);
  });

  it("限定助手时重新纳管范围内残留，并停用范围外旧绑定", () => {
    const plan = buildMcpSyncPlan(makeElement({
      id: "mcp:demo",
      spec: { command: "demo", agents: ["codex"] },
      policy: { enabled: true },
      observed: {
        agents: {
          codex: { state: "leftover", assigned: false, enabled: false, actions: ["remove"] },
          "claude-code": { state: "drifted", assigned: true, enabled: true, actions: ["disable", "remove"] },
        },
      },
    }));

    expect(plan.rows.map(({ agent, action, takeover }) => ({ agent, action, takeover }))).toEqual([
      { agent: "codex", action: "add", takeover: true },
      { agent: "claude-code", action: "disable", takeover: false },
    ]);
  });
});
