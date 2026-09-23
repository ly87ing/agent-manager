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
      // 规则写入必须精确到一个目标并带写入基准，裸 rules:apply 不再是合法命令
      const args = command === "rules:apply"
        ? ["--only", "rules:codex_global", "--base", "aaaa3c8ae2687d47a8e7543d3c8c43ef8dd9f744e3a3e2ce9aa9ad7a90de12a2", "--recommended"]
        : [];
      const envelope = await client.run(command, args);
      expect(envelope.ok, command).toBe(true);
    }
    for (const [command, args] of [
      ["rules:save-template", ["--name", "演示模板", "--value", "- x\n"]],
      ["rules:delete-template", ["--template", "tpl-demo-team"]],
    ] as const) {
      const envelope = await client.run(command, [...args]);
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

    // 每次写入都带 status 给出的写入基准
    const baseOf = async () => {
      const status = await client.run("agents:status");
      return (status.data as { elements: Array<{ id: string; observed: { current_config?: { base?: string } } }> }).elements
        .find((element) => element.id === "agents:codex")?.observed.current_config?.base ?? "";
    };
    const stale = await client.run("agents:configure", ["--only", "agents:codex", "--base", "0".repeat(64), "--use", "kimi"]);
    expect(stale.ok).toBe(false);
    expect(stale.errors[0]?.code).toBe("AGENT_CONFIG_CHANGED");

    const switched = await client.run(
      "agents:configure",
      ["--only", "agents:codex", "--base", await baseOf(), "--use", "kimi"],
      { runId: "gui-mock-switch" },
    );
    expect(switched.ok).toBe(true);
    expect(switched.run_id).toBe("gui-mock-switch");
    const active = await client.run("agents:status");
    const codex = (active.data as { elements: Array<{ id: string; spec: { settings?: { active?: string } } }> }).elements
      .find((element) => element.id === "agents:codex");
    expect(codex?.spec.settings?.active).toBe("kimi");

    await client.run("agents:configure", ["--only", "agents:codex", "--base", await baseOf(), "--use-default"]);
    const restored = await client.run("agents:status");
    const restoredCodex = (restored.data as { elements: Array<{ id: string; spec: { settings?: { active?: string } }; observed: { current_config?: { config?: { provider?: string | null; base_url?: string | null } } } }> }).elements
      .find((element) => element.id === "agents:codex");
    expect(restoredCodex?.spec.settings?.active).toBeUndefined();
    expect(restoredCodex?.observed.current_config?.config?.provider).toBe("openai");
    expect(restoredCodex?.observed.current_config?.config?.base_url).toBeNull();
  });

  it("mock client 按实际配置为准演示：未保存当前项、运行参数补丁、外部修改被拒、令牌未保存", async () => {
    const client = createMockEngineClient();
    type Current = { base: string; current: { profile: string | null; default: boolean }; config: { runtime: Record<string, unknown>; credential: { saved_var: string | null } } };
    const claude = async () => ((await client.run("agents:status")).data as { elements: Array<{ id: string; observed: { current_config: Current } }> })
      .elements.find((element) => element.id === "agents:claude-code")!.observed.current_config;
    const configure = async (...rest: string[]) =>
      client.run("agents:configure", ["--only", "agents:claude-code", "--base", (await claude()).base, ...rest]);

    const before = await claude();
    expect(before.current).toMatchObject({ profile: null, default: false });
    expect(before.config.credential.saved_var).toBeNull();
    expect(before.config.runtime).toMatchObject({ automation_mode: "custom", claude_effort_level: "high" });

    // 运行参数补丁只改提交的字段
    expect((await configure("--runtime", JSON.stringify({ claude_effort_level: "low" }))).ok).toBe(true);
    expect((await claude()).config.runtime).toMatchObject({ automation_mode: "custom", claude_effort_level: "low" });

    // 本工具以外改了配置：界面手里的旧基准被拒
    const stale = (await claude()).base;
    const hooks = (window as unknown as { __agentManagerMock: { externalEdit: (id?: string) => boolean; setSecretSourceReady: (ready: boolean) => void } }).__agentManagerMock;
    expect(hooks.externalEdit()).toBe(true);
    const refused = await client.run("agents:configure", ["--only", "agents:claude-code", "--base", stale, "--use-default"]);
    expect(refused.errors[0]?.code).toBe("AGENT_CONFIG_CHANGED");
    expect((await claude()).config.runtime.claude_effort_level).toBe("low");

    // 没选密钥文件：实际令牌未保存，切换被拒；显式放弃后成功，导入项不带令牌
    hooks.setSecretSourceReady(false);
    const tokenRefused = await configure("--use-default");
    expect(tokenRefused.errors[0]?.code).toBe("AGENT_LIVE_TOKEN_UNSAVED");
    const discarded = await configure("--use-default", "--discard-live-token");
    expect(discarded.ok).toBe(true);
    expect((discarded.data as { backfill: { token: string } }).backfill.token).toBe("discarded");
    expect((await claude()).current).toMatchObject({ profile: null, default: true });
  });

  it("mock client 在已选密钥文件时把实际令牌写进变量，并支持保存当前实际配置", async () => {
    const client = createMockEngineClient();
    const status = async () => ((await client.run("agents:status")).data as { elements: Array<{ id: string; observed: { current_config: { base: string; current: { profile: string | null } } } }> })
      .elements.find((element) => element.id === "agents:claude-code")!.observed.current_config;
    const saved = await client.run("agents:configure", ["--only", "agents:claude-code", "--base", (await status()).base, "--save-current"]);
    expect(saved.ok).toBe(true);
    const backfill = (saved.data as { backfill: { name: string; token: string; token_var: string } }).backfill;
    expect(backfill).toMatchObject({ token: "adopted" });
    expect(backfill.token_var).toMatch(/_TOKEN$/);
    expect((await status()).current.profile).toBe(backfill.name);
  });

  it("mock client 支持 MCP 页就地填写缺失变量并回读状态", async () => {
    const client = createMockEngineClient();
    type Cells = { elements: Array<{ id: string; observed: { agents?: Record<string, { vars?: Record<string, { resolved?: boolean }> }> } }> };

    const before = await client.run("mcp:status");
    const beforeGrok = (before.data as Cells).elements.find((element) => element.id === "mcp:grok-search");
    expect(beforeGrok?.observed.agents?.["claude-code"].vars?.TAVILY_API_KEY.resolved).toBe(false);

    const saved = await client.run(
      "secrets:set",
      ["--var", "TAVILY_API_KEY", "--value", "test-only-value"],
      { runId: "gui-mock-secret" },
    );
    expect(saved.ok).toBe(true);
    expect(saved.run_id).toBe("gui-mock-secret");

    const after = await client.run("mcp:status");
    const afterGrok = (after.data as Cells).elements.find((element) => element.id === "mcp:grok-search");
    expect(afterGrok?.observed.agents?.["claude-code"].vars?.TAVILY_API_KEY.resolved).toBe(true);
  });
});
