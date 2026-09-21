import { describe, expect, it } from "vitest";
import { mockModeFromSearch } from "../lib/engine";
import { createMockEngineClient } from "./mock-client";

describe("dev-only mock 开关", () => {
  it("仅在 DEV 且 ?mock=1|fresh 时激活；生产（dev=false）一律为 null", () => {
    expect(mockModeFromSearch("?mock=1", true)).toBe("demo");
    expect(mockModeFromSearch("?mock=fresh", true)).toBe("fresh");
    // 生产门控：即使带参数也不激活
    expect(mockModeFromSearch("?mock=1", false)).toBeNull();
    expect(mockModeFromSearch("?mock=fresh", false)).toBeNull();
    // 无参数 / 其它值
    expect(mockModeFromSearch("", true)).toBeNull();
    expect(mockModeFromSearch("?mock=0", true)).toBeNull();
    expect(mockModeFromSearch("?other=1", true)).toBeNull();
  });

  it("mock client 覆盖全部页面命令，fixture 含五态 freshness 与七态收敛", async () => {
    const client = createMockEngineClient();
    for (const command of [
      "system:audit",
      "agents:status",
      "mcp:status",
      "skills:status",
      "rules:status",
      "secrets:status",
      "profiles:status",
      "usage:status",
      "usage:scan",
      "agents:apply",
      "mcp:apply",
      "skills:apply",
      "rules:apply",
      "system:refresh-upstream",
      "agents:check",
      "config:export",
      "runs:clear",
    ]) {
      const envelope = await client.run(command);
      expect(envelope.ok, command).toBe(true);
    }

    const mcp = await client.run("mcp:status");
    const agents = await client.run("agents:status");
    const skills = await client.run("skills:status");
    const rules = await client.run("rules:status");

    const all = [mcp, agents, skills, rules].flatMap(
      (env) => (env.data as { elements: Array<{ freshness: { state: string }; state: string }> }).elements,
    );
    const freshnessStates = new Set(all.map((el) => el.freshness.state));
    for (const state of ["current", "behind", "ahead", "unknown", "not-applicable"]) {
      expect(freshnessStates.has(state), `freshness ${state}`).toBe(true);
    }
    const convergenceStates = new Set(all.map((el) => el.state));
    for (const state of [
      "managed-ok",
      "drifted",
      "missing",
      "not-materialized",
      "unmanaged-detected",
      "unsupported",
      "disabled",
    ]) {
      expect(convergenceStates.has(state), `convergence ${state}`).toBe(true);
    }
  });

  it("mock client 可在无账号环境演示接入测试、切换与状态回读", async () => {
    const client = createMockEngineClient();
    const checked = await client.run("agents:check", ["--only", "agents:codex", "--use", "kimi"]);
    expect(checked.ok).toBe(true);
    expect((checked.data as { ok: boolean }).ok).toBe(true);

    const switched = await client.run(
      "agents:configure",
      ["--only", "agents:codex", "--use", "kimi"],
      { runId: "gui-mock-switch" },
    );
    expect(switched.ok).toBe(true);
    expect(switched.run_id).toBe("gui-mock-switch");
    const active = await client.run("agents:status");
    const codex = (active.data as { elements: Array<{ id: string; spec: { settings?: { active?: string } } }> }).elements
      .find((element) => element.id === "agents:codex");
    expect(codex?.spec.settings?.active).toBe("kimi");

    await client.run("agents:configure", ["--only", "agents:codex", "--use-default"]);
    const restored = await client.run("agents:status");
    const restoredCodex = (restored.data as { elements: Array<{ id: string; spec: { settings?: { active?: string } }; observed: { current_config?: { config?: { provider?: string | null; base_url?: string | null } } } }> }).elements
      .find((element) => element.id === "agents:codex");
    expect(restoredCodex?.spec.settings?.active).toBeUndefined();
    expect(restoredCodex?.observed.current_config?.config?.provider).toBe("openai");
    expect(restoredCodex?.observed.current_config?.config?.base_url).toBeNull();
  });

  it("mock client 支持 MCP 页就地填写缺失变量并回读状态", async () => {
    const client = createMockEngineClient();

    const before = await client.run("mcp:status");
    const beforeGrok = (before.data as { elements: Array<{ id: string; observed: { env?: Record<string, { resolved?: boolean }> } }> }).elements
      .find((element) => element.id === "mcp:grok-search");
    expect(beforeGrok?.observed.env?.TAVILY_API_KEY.resolved).toBe(false);

    const saved = await client.run(
      "secrets:set",
      ["--var", "TAVILY_API_KEY", "--value", "test-only-value"],
      { runId: "gui-mock-secret" },
    );
    expect(saved.ok).toBe(true);
    expect(saved.run_id).toBe("gui-mock-secret");

    const after = await client.run("mcp:status");
    const afterGrok = (after.data as { elements: Array<{ id: string; observed: { env?: Record<string, { resolved?: boolean }>; agents?: Record<string, { actions?: string[]; blocked_reason?: string | null }> } }> }).elements
      .find((element) => element.id === "mcp:grok-search");
    expect(afterGrok?.observed.env?.TAVILY_API_KEY.resolved).toBe(true);
    expect(afterGrok?.observed.agents?.codex.actions).toEqual(["add"]);
    expect(afterGrok?.observed.agents?.codex.blocked_reason).toBeNull();
  });
});
