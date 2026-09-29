import { describe, expect, it } from "vitest";
import type { McpTarget } from "../lib/contract";
import { mcpFillArgs, mcpFillRows } from "../lib/mcpFill";
import { makeElement } from "./fixtures";

function target(agent: string, format: string, extra: Partial<McpTarget> = {}): McpTarget {
  return { agent, path: `/${agent}`, format, exists: true, agent_enabled: true, disable_mode: "snapshot", error: null, ...extra };
}

const targets = [
  target("claude-code", "claude-json"),
  target("work", "claude-json"),
  target("ds", "claude-json"),
  target("codex", "codex-toml"),
  target("broken", "claude-json", { error: "解析失败" }),
];

function server(name: string, agents: Record<string, { state: string; source?: string; base?: string }>) {
  return makeElement({
    id: `mcp:${name}`,
    domain: "mcp",
    observed: {
      agents: Object.fromEntries(Object.entries(agents).map(([agent, cell]) => [agent, { source: "file", base: `${agent}-base`, actions: ["edit", "copy", "disable", "remove"], ...cell }])),
    },
  });
}

describe("MCP 补齐", () => {
  it("在用服务补到同格式缺它的账号；停用的账号、读不了的文件、另一种助手都不动", () => {
    const rows = mcpFillRows([server("serena", { "claude-code": { state: "enabled" }, ds: { state: "disabled" } })], targets);
    expect(rows).toEqual([{ elementId: "mcp:serena", name: "serena", from: "claude-code", base: "claude-code-base", to: "work" }]);
    expect(mcpFillArgs(rows[0])).toEqual(["--only", "mcp:serena", "--agent", "claude-code", "--to", "work", "--action", "copy", "--base", "claude-code-base"]);
  });

  it("只有停用或已保存快照的服务不算在用", () => {
    expect(mcpFillRows([server("off", { "claude-code": { state: "disabled" } })], targets)).toEqual([]);
    expect(mcpFillRows([server("saved", { "claude-code": { state: "disabled", source: "saved" } })], targets)).toEqual([]);
  });

  it("Codex 的服务只补给 Codex 账号；可以只补指定账号", () => {
    const codexTargets = [...targets, target("cx2", "codex-toml")];
    const rows = mcpFillRows([server("x", { codex: { state: "enabled" }, "claude-code": { state: "enabled" } })], codexTargets, ["cx2"]);
    expect(rows.map((row) => [row.from, row.to])).toEqual([["codex", "cx2"]]);
  });
});
