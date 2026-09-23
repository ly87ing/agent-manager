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
    mcp: { definitions: { keep: { command: "npx" }, edit: { command: "old" } } },
    skills: { repositories: { "agent-skills": { url: "https://example.com/a.git" } } },
  },
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
    mcp: { definitions: { keep: { command: "npx" }, edit: { command: "new" } } },
    skills: { repositories: { "agent-skills": { url: "https://example.com/a.git" } } },
  },
  sources: { "rules.shared": "新规则\n", "aliases.shell": "{}" },
};

describe("备份导入差异", () => {
  it("按类别分出新增、改动与消失的条目", () => {
    const diff = diffBackup(current, incoming);
    const byLabel = Object.fromEntries(diff.sections.map((s) => [s.label, s]));

    expect(byLabel["助手定义"]).toMatchObject({ added: ["newcomer"], changed: [], removed: ["will-go"] });
    // 带 profile 段的定义单列为实例，不和普通助手混在一起
    expect(byLabel["实例"]).toMatchObject({ added: ["cx-work"], changed: [], removed: [] });
    expect(byLabel["MCP 服务"]).toMatchObject({ added: [], changed: ["edit"], removed: [] });
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

  it("空备份导入会清空现有定义，如实报为消失", () => {
    const diff = diffBackup(current, { policy: {}, sources: {} });
    const byLabel = Object.fromEntries(diff.sections.map((s) => [s.label, s]));
    expect(byLabel["助手定义"].removed).toEqual(["claude-code", "will-go"]);
    expect(byLabel["实例"].removed).toEqual(["cc-ly"]);
    expect(byLabel["MCP 服务"].removed).toEqual(["edit", "keep"]);
  });

  it("来源键有可读名称；旧备份的规则来源标明导入后转为模板", () => {
    expect(sourceLabel("aliases.shell")).toBe("终端快捷命令");
    expect(sourceLabel("unknown.key")).toBe("unknown.key");
    expect(sourceLabel("rules.shared")).toContain("导入后转为模板");
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
