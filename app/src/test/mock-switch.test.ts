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
      "mcp:apply",
      "skills:apply",
      "rules:apply",
      "system:refresh-upstream",
      "config:export",
      "runs:clear",
    ]) {
      // 规则写入必须精确到一个目标并带写入基准，裸 rules:apply 不再是合法命令
      const args = command === "rules:apply"
        ? ["--only", "rules:codex_global", "--base", "aaaa3c8ae2687d47a8e7543d3c8c43ef8dd9f744e3a3e2ce9aa9ad7a90de12a2", "--source-rule", "agent-skills/core"]
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

  it("mock 实例预览随定义变化且不写状态或回显凭据", async () => {
    const client = createMockEngineClient();
    const before = await client.run("profiles:status");
    const definition = { profile: { kind: "claude", config_dir: "~/.claude-preview-demo", model: "demo-one" } };
    const preview = async (model: string) => client.run("profiles:preview", [
      "--only", "profiles:preview-demo", "--definition",
      JSON.stringify({ ...definition, profile: { ...definition.profile, model } }),
      "--on-save", "--items", "permissions,mcp",
    ]);
    const first = await preview("demo-one");
    const second = await preview("demo-two");
    expect(first.data).toMatchObject({ path: "~/.claude-preview-demo/settings.json", exists: false, will_write: true });
    expect(JSON.stringify(first.data)).toContain("demo-one");
    expect(JSON.stringify(second.data)).toContain("demo-two");
    expect(JSON.stringify(second.data)).not.toContain("demo-one");

    const candidate = await client.run("profiles:preview", [
      "--only", "profiles:preview-demo", "--base", "missing", "--definition-base", "missing",
      "--candidate", JSON.stringify(definition), "--exact",
      "--preview-secret-var", "YOUR_DEMO_KEY", "--preview-secret-value", "YOUR_DEMO_TOKEN",
    ]);
    expect(candidate.data).toMatchObject({ file_base: "missing", content_base: "d".repeat(64) });
    expect(JSON.stringify(candidate)).not.toContain("YOUR_DEMO_TOKEN");
    expect(await client.run("profiles:status")).toEqual(before);
  });
});
