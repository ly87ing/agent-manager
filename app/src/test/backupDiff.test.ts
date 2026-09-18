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

  it("来源键有可读名称", () => {
    expect(sourceLabel("aliases.shell")).toBe("终端快捷命令");
    expect(sourceLabel("unknown.key")).toBe("unknown.key");
  });
});

describe("凭据取值的差异", () => {
  it("只报变量名与变动类别，两边的取值都不出现", () => {
    const diff = diffBackup(
      { policy: {}, sources: {}, secrets: { KEEP: "a", CHANGE: "old-value", GONE: "bye" } },
      { policy: {}, sources: {}, secrets: { KEEP: "a", CHANGE: "new-value", FRESH: "hi" } },
    );
    expect(diff.secrets.added).toEqual(["FRESH"]);
    expect(diff.secrets.changed).toEqual(["CHANGE"]);
    expect(diff.secrets.removed).toEqual(["GONE"]);
    const text = JSON.stringify(diff);
    for (const value of ["old-value", "new-value", "bye", "hi"]) {
      expect(text).not.toContain(value);
    }
  });

  it("取值变了就不算「完全一样」", () => {
    const same = diffBackup({ policy: {}, sources: {}, secrets: { T: "x" } }, { policy: {}, sources: {}, secrets: { T: "x" } });
    expect(same.identical).toBe(true);
    const differs = diffBackup({ policy: {}, sources: {}, secrets: { T: "x" } }, { policy: {}, sources: {}, secrets: { T: "y" } });
    expect(differs.identical).toBe(false);
  });
});
