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

  it("mock client 覆盖扩展页面命令及其资源状态", async () => {
    const client = createMockEngineClient();
    for (const command of [
      "system:audit",
      "resources:targets",
      "mcp:status",
      "skills:status",
      "rules:status",
      "secrets:status",
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
    const skills = await client.run("skills:status");
    const rules = await client.run("rules:status");

    const all = [mcp, skills, rules].flatMap(
      (env) => (env.data as { elements: Array<{ freshness: { state: string }; state: string }> }).elements,
    );
    const freshnessStates = new Set(all.map((el) => el.freshness.state));
    for (const state of ["current", "behind", "unknown", "not-applicable"]) {
      expect(freshnessStates.has(state), `freshness ${state}`).toBe(true);
    }
    const convergenceStates = new Set(all.map((el) => el.state));
    for (const state of [
      "managed-ok",
      "drifted",
      "missing",
      "not-materialized",
      "disabled",
    ]) {
      expect(convergenceStates.has(state), `convergence ${state}`).toBe(true);
    }
  });

  it("mock client 支持 MCP 页就地填写缺失变量并回读状态", async () => {
    const client = createMockEngineClient();
    type Cells = { elements: Array<{ id: string; observed: { agents?: Record<string, { vars?: Record<string, { resolved?: boolean }> }> } }> };

    const before = await client.run("mcp:status");
    const beforeSearch = (before.data as Cells).elements.find((element) => element.id === "mcp:example-search");
    expect(beforeSearch?.observed.agents?.["claude-code"].vars?.CRAWL_API_KEY.resolved).toBe(false);

    const saved = await client.run(
      "secrets:set",
      ["--var", "CRAWL_API_KEY", "--value", "test-only-value"],
      { runId: "gui-mock-secret" },
    );
    expect(saved.ok).toBe(true);
    expect(saved.run_id).toBe("gui-mock-secret");

    const after = await client.run("mcp:status");
    const afterSearch = (after.data as Cells).elements.find((element) => element.id === "mcp:example-search");
    expect(afterSearch?.observed.agents?.["claude-code"].vars?.CRAWL_API_KEY.resolved).toBe(true);
  });

  it("mock rejects all retired account operations", async () => {
    const client = createMockEngineClient();
    for (const command of ["agents:status", "agents:configure", "profiles:preview", "profiles:create", "accounts:command", "aliases:apply", "secrets:migrate-legacy"]) {
      const result = await client.run(command);
      expect(result.ok).toBe(false);
      expect(result.errors[0].code).toBe("FEATURE_RETIRED");
    }
  });
});
