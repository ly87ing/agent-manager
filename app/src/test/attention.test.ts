import { describe, expect, it } from "vitest";
import { commandsAttention, mcpAttention, rulesAttention, skillsAttention } from "../lib/attention";
import { makeElement } from "./fixtures";

describe("导航角标：各页待处理数", () => {
  it("技能：黄色开关与新发现各算一个，正常的不算，集合本身不算", () => {
    const skill = (name: string, observed: Record<string, unknown>) =>
      makeElement({ id: `skill:c/${name}`, domain: "skills", spec: { collection: "c", name }, observed });
    expect(skillsAttention([
      makeElement({ id: "skill:c", domain: "skills", spec: { url: "x" }, observed: {} }),
      skill("ok", { agents: { a: { state: "linked", actions: ["disable"] } } }),
      skill("broken", { agents: { a: { state: "dead-link", actions: ["update"] } } }),
      skill("new", { agents: {}, discovery_inbox: true }),
    ])).toBe(2);
  });

  it("MCP：无法识别、找不到命令、缺变量的服务各算一个", () => {
    const server = (id: string, cell: Record<string, unknown>) => makeElement({ id: `mcp:${id}`, domain: "mcp", observed: { agents: { a: { base: "b", actions: [], source: "file", ...cell } } } });
    expect(mcpAttention([
      server("ok", { state: "enabled", command_available: true }),
      server("bad", { state: "invalid" }),
      server("nocmd", { state: "enabled", command_available: false }),
      server("vars", { state: "enabled", vars: { K: { placeholder: "${K}", resolved: false } } }),
    ])).toBe(3);
  });

  it("规则：有新版本与可以补齐的账号", () => {
    const rule = (id: string, observed: Record<string, unknown>, spec: Record<string, unknown> = {}) =>
      makeElement({ id, domain: "rules", spec: { group: "global-rules", kind: "claude", ...spec }, observed: { content: "x", base: "b", ...observed }, actions: [{ id: "write" }] as never });
    expect(rulesAttention([
      rule("rules:claude_global", { source_follow: { rule: "r", applied_revision: "1", current_revision: "2", file_unchanged: true, update_available: true } }),
      rule("rules:profile:w", { content: null, base: "missing" }, { group: "profile-rules", kind: "claude" }),
      rule("rules:codex_global", {}, { kind: "codex" }),
    ])).toBe(2);
  });

  it("启动命令：两个文件写法不同或缺一处的", () => {
    const target = (id: string, entries: { name: string; line: string }[]) =>
      makeElement({ id, domain: "aliases", observed: { base: "b", entries: entries.map((entry) => ({ ...entry, value: null })), external: [] } });
    expect(commandsAttention([
      target("aliases:zshrc", [{ name: "a", line: "alias a='x'" }, { name: "b", line: "alias b='1'" }, { name: "c", line: "alias c='c'" }]),
      target("aliases:zprofile", [{ name: "a", line: "alias a='x'" }, { name: "b", line: "alias b='2'" }]),
    ])).toBe(2);
  });
});
