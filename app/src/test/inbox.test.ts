import { describe, expect, it } from "vitest";
import type { McpTarget } from "../lib/contract";
import { buildInbox, countByPage, type InboxInput, type InboxItem } from "../lib/inbox";
import { makeElement, makeFreshness } from "./fixtures";

function input(overrides: Partial<InboxInput>): InboxInput {
  return { skills: [], mcp: [], mcpTargets: [], rules: [], agents: [], profiles: [], aliases: [], ...overrides };
}

const byId = (items: InboxItem[]) => Object.fromEntries(items.map((item) => [item.id, item]));

const collection = (name: string, observed: Record<string, unknown> = {}, extra: Record<string, unknown> = {}) =>
  makeElement({ id: `skill:${name}`, domain: "skills", spec: { url: `https://example.com/${name}.git` }, policy: { enabled: true }, observed: { repo_cached: true, actions: ["update"], ...observed }, ...extra });
const skill = (name: string, agents: Record<string, Record<string, unknown>>, extra: Record<string, unknown> = {}) =>
  makeElement({ id: `skill:c/${name}`, domain: "skills", spec: { collection: "c", name }, observed: { agents, ...extra } });

function target(agent: string, format: string): McpTarget {
  return { agent, path: `/${agent}`, format, exists: true, agent_enabled: true, disable_mode: "snapshot", error: null };
}
const server = (id: string, agents: Record<string, Record<string, unknown>>) =>
  makeElement({ id: `mcp:${id}`, domain: "mcp", observed: { agents: Object.fromEntries(Object.entries(agents).map(([agent, cell]) => [agent, { base: `${agent}-b`, actions: ["copy"], source: "file", ...cell }])) } });

describe("统一状态：各领域的实际状态映射成同一套待处理项", () => {
  it("技能：损坏直接修复、来源已删与被占用跳过去、外部链接算未纳管、缺的补齐、已取回的新版本直接换、新发现可一键添加", () => {
    const items = byId(buildInbox(input({
      skills: [
        collection("c"),
        skill("ok", { a: { state: "linked", actions: ["disable"] } }),
        skill("broken", { a: { state: "dead-link", actions: ["update"] } }),
        skill("gone", { a: { state: "linked", source_missing: true, actions: ["remove"] } }),
        skill("taken", { a: { state: "drifted", actions: ["disable"] } }),
        skill("extra", { a: { state: "extra-link", actions: ["add", "remove"] } }),
        skill("fill", { a: { state: "linked", actions: ["disable"] }, b: { state: "not-assigned", actions: ["add"] } }),
        skill("newer", { a: { state: "linked", actions: ["update", "disable"], version: "v1", available_version: "v2" } }),
        skill("found", { a: { state: "not-assigned", actions: ["add"] } }, { discovery_inbox: true }),
      ],
    })));
    expect(Object.keys(items).filter((id) => id.includes("/ok"))).toEqual([]);
    expect(items["skill:skill:c/broken:broken"]).toMatchObject({ state: "broken", fix: { kind: "skills", label: "修复", rows: [{ agent: "a", action: "update" }] } });
    expect(items["skill:skill:c/gone:dropped"]).toMatchObject({ state: "broken", fix: { kind: "open", target: "skills:gone" } });
    expect(items["skill:skill:c/taken:drifted"]).toMatchObject({ state: "drifted", fix: { kind: "open" } });
    expect(items["skill:skill:c/extra:extra"]).toMatchObject({ state: "unmanaged", subject: "skill:c/extra" });
    expect(items["skill:skill:c/fill:missing"]).toMatchObject({ state: "missing", fix: { kind: "skills", label: "补齐", rows: [{ agent: "b", action: "add" }] } });
    expect(items["skill:skill:c/newer:version"]).toMatchObject({ state: "update", fix: { kind: "skills", rows: [{ agent: "a", action: "update" }] } });
    expect(items["skill:skill:c/found:new"]).toMatchObject({ state: "update", fix: { kind: "skills", label: "添加到全部账号" } });
  });

  it("来源：有新版本与还没取回的去来源页（git 先预览），本机目录不存在、读取失败算损坏；停用的不算", () => {
    const items = byId(buildInbox(input({
      skills: [
        collection("behind", {}, { freshness: makeFreshness({ state: "behind", upstream: "v2" }) }),
        collection("fresh", { repo_cached: false }),
        makeElement({ id: "skill:local", domain: "skills", spec: { source_path: "~/gone" }, policy: { enabled: true }, observed: { repo_cached: false } }),
        collection("bad", { discovery_error: { message: "SKILL.md 解析失败" } }),
        collection("off", { repo_cached: false }, { policy: { enabled: false } }),
      ],
    })));
    expect(items["source:behind:behind"]).toMatchObject({ page: "sources", state: "update", detail: "可更新到 v2", fix: { kind: "open", target: "sources:behind" } });
    expect(items["source:fresh:fetch"]).toMatchObject({ state: "update", fix: { label: "取回" } });
    expect(items["source:local:fetch"]).toMatchObject({ state: "broken", detail: "本机目录不存在：~/gone" });
    expect(items["source:bad:broken"]).toMatchObject({ state: "broken", detail: "SKILL.md 解析失败" });
    expect(Object.keys(items).some((id) => id.startsWith("source:off"))).toBe(false);
  });

  it("未纳管：不来自任何来源的技能去来源页纳入；Codex 重名算已偏离", () => {
    const items = byId(buildInbox(input({
      unmanaged: {
        skills: [{ name: "archify", locations: [], source: { repository: "tt-a1i/archify" } } as never],
        shared_dirs: [],
        duplicates: [{ name: "dup", agent: "codex", paths: [] }],
      },
    })));
    expect(items["unmanaged:archify"]).toMatchObject({ page: "sources", state: "unmanaged", fix: { kind: "open", target: "sources:unmanaged" } });
    expect(items["duplicate:codex:dup"]).toMatchObject({ state: "drifted", fix: { label: "去清理" } });
  });

  it("MCP：无法识别、找不到命令、缺变量算损坏；同类账号缺的补齐", () => {
    const items = byId(buildInbox(input({
      mcp: [
        server("bad", { a: { state: "invalid" } }),
        server("nocmd", { a: { state: "enabled", command_available: false } }),
        server("vars", { a: { state: "enabled", vars: { K: { placeholder: "${K}", resolved: false } } } }),
        server("fill", { a: { state: "enabled", copy_safe: true } }),
      ],
      mcpTargets: [target("a", "claude-json"), target("b", "claude-json")],
    })));
    expect(items["mcp:bad:invalid"].state).toBe("broken");
    expect(items["mcp:nocmd:command"].state).toBe("broken");
    expect(items["mcp:vars:vars"]).toMatchObject({ state: "broken", fix: { target: "mcp:vars" } });
    expect(items["mcp:fill:missing"]).toMatchObject({ state: "missing", fix: { kind: "mcp-fill", rows: [{ from: "a", to: "b" }] } });
  });

  it("规则：没有规则文件的补齐，跟随的有新版本、改过且来源也更新的看差异", () => {
    const rule = (id: string, observed: Record<string, unknown>, spec: Record<string, unknown> = {}) =>
      makeElement({ id, domain: "rules", spec: { group: "global-rules", kind: "claude", name: id, ...spec }, observed: { content: "x", base: "b", ...observed }, actions: [{ id: "write" }] as never });
    const items = byId(buildInbox(input({
      rules: [
        rule("rules:claude_global", { source_follow: { rule: "r", applied_revision: "1", current_revision: "2", file_unchanged: true, update_available: true } }),
        rule("rules:profile:w", { content: null, base: "missing" }, { group: "profile-rules" }),
        rule("rules:codex_global", { source_follow: { rule: "r", applied_revision: "1", current_revision: "2", file_unchanged: false, update_available: false } }, { kind: "codex" }),
      ],
    })));
    expect(items["rules:rules:claude_global:update"]).toMatchObject({ state: "update", fix: { kind: "open", target: "rules" } });
    expect(items["rules:rules:profile:w:missing"]).toMatchObject({ state: "missing", fix: { kind: "rules-fill", rows: [{ from: "rules:claude_global" }] } });
    expect(items["rules:rules:codex_global:drifted"].state).toBe("drifted");
  });

  it("启动命令：只比较有受管区块的文件；~/.zprofile 没有区块（不新建）时只在 ~/.zshrc 的命令不算缺一处", () => {
    const aliasTarget = (id: string, base: string | null, entries: { name: string; line: string }[]) =>
      makeElement({ id, domain: "aliases", observed: { base, entries: entries.map((entry) => ({ ...entry, value: null })), external: [] } });
    expect(buildInbox(input({
      aliases: [aliasTarget("aliases:zshrc", "b", [{ name: "cc2", line: "alias cc2='x'" }]), aliasTarget("aliases:zprofile", "missing", [])],
    }))).toEqual([]);
    const items = buildInbox(input({
      aliases: [
        aliasTarget("aliases:zshrc", "b", [{ name: "a", line: "alias a='1'" }, { name: "b", line: "alias b='b'" }]),
        aliasTarget("aliases:zprofile", "c", [{ name: "a", line: "alias a='2'" }]),
      ],
    }));
    expect(items.map((item) => [item.id, item.state])).toEqual([["command:a", "drifted"], ["command:b", "drifted"]]);
  });

  it("导航角标按负责的页面数有待处理的对象（同一个技能两条只算一个），「待处理」数全部条目", () => {
    const counts = countByPage(buildInbox(input({
      skills: [
        collection("c", {}, { freshness: makeFreshness({ state: "behind" }) }),
        skill("broken", { a: { state: "dead-link", actions: ["update"] }, b: { state: "drifted", actions: [] } }),
      ],
    })));
    expect(counts).toEqual({ sources: 1, skills: 1, inbox: 3 });
  });
});
