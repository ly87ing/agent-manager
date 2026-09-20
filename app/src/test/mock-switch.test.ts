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
      "agents:apply",
      "mcp:apply",
      "skills:apply",
      "rules:apply",
      "system:refresh-upstream",
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
});
