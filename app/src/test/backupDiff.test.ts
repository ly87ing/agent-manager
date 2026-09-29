import { describe, expect, it } from "vitest";
import { diffBackup, sourceLabel } from "../lib/backupDiff";

const current = {
  policy: {
    agents: {
      definitions: {
        "claude-code": { bin: "claude" },
        "cc-ly": { bin: "claude", profile: { base_agent: "claude-code", config_dir: "~/.claude-ly" } },
        "will-go": { bin: "gone" },
      },
    },
    mcp: { definitions: {} },
    skills: { repositories: { "agent-skills": { url: "https://example.com/a.git" } } },
  },
  // MCP 已保存项不在策略里（原先是 policy.mcp.definitions 服务库定义，该模型已退役）
  mcp_saved: [
    { server: "keep", agent: "", entry: { command: "npx" }, resolve: { env: [], headers: [] }, origin: "migrated" },
    { server: "edit", agent: "claude-code", entry: { command: "old" }, resolve: { env: [], headers: [] }, origin: "disabled" },
  ],
  sources: { "rules.shared": "旧规则\n", "aliases.shell": "{}" },
};

const incoming = {
  policy: {
    agents: {
      definitions: {
        "claude-code": { bin: "claude" },
        "cc-ly": { bin: "claude", profile: { base_agent: "claude-code", config_dir: "~/.claude-ly" } },
        "cx-work": { bin: "codex", profile: { base_agent: "codex", config_dir: "~/.codex-work" } },
        newcomer: { bin: "new" },
      },
    },
    // 旧备份的服务库定义按未分配的已保存项计入
    mcp: { definitions: { legacy: { command: "uvx" } } },
    skills: { repositories: { "agent-skills": { url: "https://example.com/a.git" } } },
  },
  mcp_saved: [
    { server: "keep", agent: "", entry: { command: "npx" }, resolve: { env: [], headers: [] }, origin: "migrated" },
    { server: "edit", agent: "claude-code", entry: { command: "new" }, resolve: { env: [], headers: [] }, origin: "disabled" },
  ],
  sources: { "rules.shared": "新规则\n", "aliases.shell": "{}" },
};

describe("备份导入差异", () => {
  it("按类别分出新增、改动与消失的条目", () => {
    const diff = diffBackup(current, incoming);
    const byLabel = Object.fromEntries(diff.sections.map((s) => [s.label, s]));

    expect(byLabel["助手定义"]).toMatchObject({ added: ["newcomer"], changed: [], removed: ["will-go"] });
    // 带 profile 段的定义单列为实例，不和普通助手混在一起
    expect(byLabel["实例"]).toMatchObject({ added: ["cx-work"], changed: [], removed: [] });
    expect(byLabel["MCP 已保存项"]).toMatchObject({ added: ["legacy"], changed: ["edit @ claude-code"], removed: [] });
    // 完全没变的类别不出现
    expect(byLabel["技能集合"]).toBeUndefined();
  });

  it("只列出内容真的变了的来源", () => {
    const diff = diffBackup(current, incoming);
    expect(diff.sourcesChanged).toEqual(["rules.shared"]);
    expect(diff.identical).toBe(false);
  });

  it("两边一样时明确标出「不会改变任何东西」", () => {
    const diff = diffBackup(current, current);
    expect(diff.sections).toEqual([]);
    expect(diff.sourcesChanged).toEqual([]);
    expect(diff.identical).toBe(true);
  });

  it("空备份导入会清空现有定义，如实报为消失；本机的 MCP 已保存项不会消失", () => {
    const diff = diffBackup(current, { policy: {}, sources: {} });
    const byLabel = Object.fromEntries(diff.sections.map((s) => [s.label, s]));
    expect(byLabel["助手定义"].removed).toEqual(["claude-code", "will-go"]);
    expect(byLabel["实例"].removed).toEqual(["cc-ly"]);
    // 导入对 MCP 已保存项只增改不删（原先断言服务库定义会被清空，该模型已退役）
    expect(byLabel["MCP 已保存项"]).toBeUndefined();
  });

  it("来源键有可读名称；旧备份的规则来源标明导入后转为模板", () => {
    // 快捷命令以 rc 文件为准：备份里的 aliases.shell 只是旧版保存、还没生效的那些
    expect(sourceLabel("aliases.shell")).toContain("未生效");
    expect(sourceLabel("unknown.key")).toBe("unknown.key");
    expect(sourceLabel("rules.shared")).toContain("导入后转为模板");
    // Hooks 已不再管理：旧备份里的 hooks.* 导入时跳过
    expect(sourceLabel("hooks.claude")).toContain("导入时跳过");
  });

  it("备份带规则模板时按名称比正文，报出新增、改动与消失", () => {
    const withTemplates = {
      ...current,
      rule_templates: [
        { name: "团队规则", body: "旧\n", origin: "user" },
        { name: "会消失", body: "x\n", origin: "backup" },
      ],
    };
    const incomingTemplates = {
      ...current,
      rule_templates: [
        { name: "团队规则", body: "新\n", origin: "user" },
        { name: "新模板", body: "y\n", origin: "user" },
      ],
    };
    const diff = diffBackup(withTemplates, incomingTemplates);
    const section = diff.sections.find((s) => s.label === "规则模板");
    expect(section).toMatchObject({ added: ["新模板"], changed: ["团队规则"], removed: ["会消失"] });
    expect(diff.identical).toBe(false);
  });

  it("备份没带规则模板（旧格式）时不动现有模板，也不报消失", () => {
    const withTemplates = { ...current, rule_templates: [{ name: "团队规则", body: "旧\n", origin: "user" }] };
    const diff = diffBackup(withTemplates, current);
    expect(diff.sections.find((s) => s.label === "规则模板")).toBeUndefined();
    expect(diff.identical).toBe(true);
  });
});
