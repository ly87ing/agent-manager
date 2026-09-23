import { describe, expect, it } from "vitest";
import { buildOverview, syncableOf } from "../lib/overviewModel";
import type { Envelope, StatusData } from "../lib/contract";
import { makeAction, makeElement, makeEnvelope, makeFreshness } from "./fixtures";

function status(domain: string, elements: ReturnType<typeof makeElement>[]): Envelope<StatusData> {
  return makeEnvelope({ domain, elements }, { command: `${domain}.status` });
}

describe("buildOverview", () => {
  it("助手版本与上游状态不进入总览，统一留给客户端诊断", () => {
    const groups = buildOverview({
      agents: status("agents", [
        makeElement({
          id: "agents:claude-code",
          domain: "agents",
          spec: { display_name: "Claude Code" },
          freshness: makeFreshness({ state: "behind", current: "2.0.0", upstream: "2.1.3" }),
        }),
        makeElement({
          id: "agents:codex",
          domain: "agents",
          spec: { display_name: "Codex" },
          freshness: makeFreshness({ state: "not-applicable" }),
        }),
      ]),
    });

    const agents = groups.find((group) => group.domain === "agents");
    expect(agents?.items.map((item) => [item.name, item.update, item.version, item.upstream])).toEqual([
      ["Claude Code", "none", null, null],
      ["Codex", "none", null, null],
    ]);
  });

  it("MCP 没有待同步：有助手启用着算正常，全部停用算已停用，不进一键同步", () => {
    const groups = buildOverview({
      mcp: status("mcp", [
        makeElement({
          id: "mcp:fs",
          domain: "mcp",
          spec: { name: "fs", saved: null },
          observed: { agents: { claude: { state: "enabled", actions: ["edit", "disable"] }, codex: { state: "disabled" } } },
        }),
        makeElement({
          id: "mcp:old",
          domain: "mcp",
          state: "disabled",
          spec: { name: "old", saved: null },
          observed: { agents: { claude: { state: "disabled", source: "saved", actions: ["enable"] } } },
        }),
        makeElement({
          id: "mcp:broken",
          domain: "mcp",
          state: "unsupported",
          spec: { name: "broken", saved: null },
          observed: { agents: { claude: { state: "invalid", actions: ["remove"] }, codex: { state: "enabled" } } },
        }),
      ]),
    });
    const mcp = groups.find((group) => group.domain === "mcp");
    expect(mcp?.items.map((item) => [item.name, item.sync])).toEqual([
      ["fs", "ok"],
      ["old", "disabled"],
      ["broken", "ok"],
    ]);
    expect(mcp?.pending).toBe(0);
    expect(syncableOf(groups)).toEqual([]);
  });

  it("技能只列单个技能，集合本身不占一行", () => {
    const groups = buildOverview({
      skills: status("skills", [
        makeElement({ id: "skill:agent-skills", domain: "skills", spec: { name: "agent-skills" } }),
        makeElement({ id: "skill:agent-skills/find-skills", domain: "skills", spec: { name: "find-skills" } }),
      ]),
    });

    expect(groups.find((group) => group.domain === "skills")?.items.map((item) => item.name)).toEqual(["find-skills"]);
  });

  it("快捷命令以 rc 文件实际区块为准：按名字列出、从不算待同步，读写不了的文件要你决定", () => {
    const groups = buildOverview({
      aliases: makeEnvelope(
        {
          domain: "aliases",
          elements: [
            makeElement({ id: "aliases:zshrc", domain: "aliases", spec: { target: "~/.zshrc" }, observed: { blocked_reason: null } }),
            makeElement({
              id: "aliases:zprofile",
              domain: "aliases",
              state: "unmanaged-detected",
              spec: { target: "~/.zprofile" },
              observed: { blocked_reason: "目标路径是符号链接，拒绝操作以保证安全" },
            }),
          ],
          // 只在 zshrc 里的 yolo 也照样是「已到位」：实际有它就是当前配置
          aliases: [
            { name: "cc", value: "claude", line: "alias cc='claude'", targets: ["aliases:zshrc", "aliases:zprofile"], consistent: true },
            { name: "yolo", value: "claude -y", line: "alias yolo='claude -y'", targets: ["aliases:zshrc"], consistent: true },
          ],
          alias_recommendations: [{ name: "codex", value: "codex", line: "alias codex='codex'", source: "agent-skills" }],
          alias_legacy: [{ id: "legacy-0123456789abcdef", name: "gp", value: "git push", line: "alias gp='git push'", actual: null }],
        },
        { command: "aliases.status" },
      ),
    });

    const aliases = groups.find((group) => group.domain === "aliases");
    expect(aliases?.items.map((item) => [item.name, item.sync, item.reason ?? null])).toEqual([
      ["cc", "ok", null],
      ["yolo", "ok", null],
      ["~/.zprofile", "decision", "目标路径是符号链接，拒绝操作以保证安全"],
    ]);
    expect(aliases?.pending).toBe(1);
    expect(syncableOf(groups)).toEqual([]);
  });

  it("没有任何区块时：推荐的内置快捷命令不进总览，待同步为 0", () => {
    const groups = buildOverview({
      aliases: makeEnvelope(
        {
          domain: "aliases",
          elements: [
            makeElement({ id: "aliases:zshrc", domain: "aliases", state: "missing", observed: { blocked_reason: null } }),
            makeElement({ id: "aliases:zprofile", domain: "aliases", state: "missing", observed: { blocked_reason: null } }),
          ],
          aliases: [],
          alias_recommendations: ["chrome", "claude", "codex", "grok"].map((name) => ({ name, value: name, line: `alias ${name}='${name}'`, source: "agent-skills" })),
          alias_legacy: [],
        },
        { command: "aliases.status" },
      ),
    });
    expect(groups.find((group) => group.domain === "aliases")).toBeUndefined();
    expect(groups.reduce((sum, group) => sum + group.pending, 0)).toBe(0);
    expect(syncableOf(groups)).toEqual([]);
  });

  it("状态读不出来的域不占位置", () => {
    expect(buildOverview({ agents: undefined, mcp: status("mcp", []) })).toEqual([]);
  });
});

describe("buildOverview 真实形态", () => {
  it("助手版本不进入总览，没装的仍说「没装」而不是「要你决定」", () => {
    const groups = buildOverview({
      agents: status("agents", [
        makeElement({
          id: "agents:claude-code",
          domain: "agents",
          spec: { display_name: "Claude Code" },
          observed: { installed: true, version: "2.1.276" },
          state: "managed-ok",
          actions: [makeAction("upgrade", { risk: "network", offline_ok: false })],
        }),
        makeElement({
          id: "agents:opencode",
          domain: "agents",
          spec: { display_name: "OpenCode" },
          observed: { installed: false },
          state: "missing",
          actions: [],
        }),
        // npm / Homebrew / 手动放的二进制：安装来源不归本工具管，但装着就是到位，不是「要你决定」
        makeElement({
          id: "agents:codex",
          domain: "agents",
          spec: { display_name: "Codex" },
          observed: { installed: true, install_source: "npm" },
          state: "unmanaged-detected",
          actions: [],
        }),
      ]),
    });

    expect(groups[0].items.map((item) => [item.name, item.version, item.sync])).toEqual([
      ["Claude Code", null, "ok"],
      ["OpenCode", null, "absent"],
      ["Codex", null, "ok"],
    ]);
    expect(groups[0].pending).toBe(0);
    // 没装不是待办，不进一键同步
    expect(syncableOf(groups)).toEqual([]);
  });

  it("技能来源还没拉下来时保留来源那一行、算「没装」，不进一键同步；拉下来后只列单个技能", () => {
    // 原预期是「待同步 + 一键同步去 clone」：那来自内置仓库固化进库、被当成生效配置的旧模型。
    // 现在没拉取的集合只是还没生效，不是待办；用户在集合卡片上自己点「更新资源库」。
    const bare = makeElement({
      id: "skill:agent-skills",
      domain: "skills",
      spec: { name: "agent-skills" },
      state: "not-materialized",
      actions: [makeAction("update"), makeAction("remove")],
    });
    const groups = buildOverview({ skills: status("skills", [bare]) });
    expect(groups[0].items.map((item) => [item.name, item.sync])).toEqual([["agent-skills", "absent"]]);
    expect(groups[0].pending).toBe(0);
    expect(syncableOf(groups)).toEqual([]);

    const expanded = buildOverview({
      skills: status("skills", [bare, makeElement({ id: "skill:agent-skills/find-skills", domain: "skills", spec: { name: "find-skills" } })]),
    });
    expect(expanded[0].items.map((item) => item.name)).toEqual(["find-skills"]);
  });

  it("推荐来源不是元素：空库只有推荐时技能不占位置、待同步为 0", () => {
    const envelope = makeEnvelope(
      {
        domain: "skills",
        elements: [],
        recommendations: [
          {
            name: "agent-skills",
            description: "",
            url: "https://github.com/ly87ing/agent-skills.git",
            skill_paths: ["verification"],
            added: false,
            definition: { url: "https://github.com/ly87ing/agent-skills.git", skill_paths: ["verification"] },
          },
        ],
        installed: [],
      },
      { command: "skills.status" },
    );
    const groups = buildOverview({ skills: envelope });
    expect(groups).toEqual([]);
    expect(syncableOf(groups)).toEqual([]);
  });

  it("技能只在集合有可用更新、已启用链接缺失时算待同步；未分配与外部同名链接不算，被外部改过要你决定", () => {
    const behind = makeElement({
      id: "skill:mine",
      domain: "skills",
      spec: { name: "mine" },
      state: "managed-ok",
      actions: [makeAction("update"), makeAction("disable"), makeAction("remove")],
      freshness: makeFreshness({ state: "behind", current: "a".repeat(40), upstream: "b".repeat(40) }),
    });
    const cell = (extra: Record<string, unknown>) => ({ link_path: "/x", target_dir_exists: true, ...extra });
    const skill = (name: string, agents: Record<string, unknown>) =>
      makeElement({ id: `skill:mine/${name}`, domain: "skills", spec: { collection: "mine", name }, observed: { agents } });
    const groups = buildOverview({
      skills: status("skills", [
        behind,
        skill("linked", { codex: cell({ state: "linked", assigned: true, enabled: true, actions: ["update", "disable", "remove"], version: "v1", available_version: "v1" }) }),
        skill("missing", {
          "claude-code": cell({ state: "not-assigned", assigned: false, enabled: false, actions: ["add"] }),
          codex: cell({ state: "missing-link", assigned: true, enabled: true, actions: ["update", "disable", "remove"] }),
        }),
        skill("unassigned", { codex: cell({ state: "not-assigned", assigned: false, enabled: false, actions: ["add"] }) }),
        skill("external-same-name", { codex: cell({ state: "extra-link", assigned: false, enabled: false, managed: false, actions: ["add", "remove"] }) }),
        skill("changed", { codex: cell({ state: "drifted", assigned: true, enabled: true, actions: ["update", "disable", "remove"], blocked_reason: "目标入口存在外部内容" }) }),
        skill("off", { codex: cell({ state: "disabled", assigned: true, enabled: false, actions: ["enable", "remove"] }) }),
      ]),
    });
    const skills = groups.find((group) => group.domain === "skills");
    expect(skills?.items.map((item) => [item.name, item.sync])).toEqual([
      ["mine", "pending"],
      ["linked", "ok"],
      ["missing", "pending"],
      ["unassigned", "absent"],
      ["external-same-name", "absent"],
      ["changed", "decision"],
      ["off", "disabled"],
    ]);
    expect(skills?.pending).toBe(3);
    expect(syncableOf(groups)).toEqual([
      { domain: "skills", name: "mine", args: ["--only", "skill:mine", "--action", "update"] },
      { domain: "skills", name: "missing", args: ["--only", "skill:mine/missing", "--agent", "codex", "--action", "update"] },
    ]);
  });
});

describe("buildOverview 来源里已删除的技能", () => {
  it("链接已丢的要你决定，并说明可以移除；链接还在的不催", () => {
    const cell = (state: string) => ({
      state,
      link_path: "/x",
      target_dir_exists: true,
      assigned: true,
      enabled: true,
      actions: ["remove"],
      blocked_reason: "来源里已没有这个技能，可以移除",
      source_missing: true,
    });
    const groups = buildOverview({
      skills: status("skills", [
        makeElement({
          id: "skill:mine/gone",
          domain: "skills",
          spec: { collection: "mine", name: "gone", source_path: null },
          observed: { agents: { "claude-code": cell("missing-link"), codex: cell("linked") } },
        }),
      ]),
    });
    const item = groups.find((group) => group.domain === "skills")?.items[0];
    expect(item?.sync).toBe("decision");
    expect(item?.reason).toBe("来源里已没有这个技能，可以移除");
    expect(syncableOf(groups)).toEqual([]);
  });
});

describe("buildOverview 规则", () => {
  it("规则以实际文件为准，永不算待同步：有文件已同步、没文件没装、读写不了要你决定，也不进一键同步", () => {
    const write = [makeAction("write")];
    const groups = buildOverview({
      rules: status("rules", [
        makeElement({ id: "rules:codex_global", domain: "rules", spec: { name: "Codex 全局规则" }, state: "managed-ok", actions: write }),
        makeElement({ id: "rules:claude_global", domain: "rules", spec: { name: "Claude Code 全局规则" }, state: "missing", actions: write }),
        makeElement({
          id: "rules:profile:cc-link",
          domain: "rules",
          spec: { name: "Claude · link 实例规则" },
          observed: { blocked_reason: "目标路径是符号链接，拒绝操作以保证安全" },
          state: "unmanaged-detected",
        }),
        makeElement({ id: "rules:profile:cc-off", domain: "rules", spec: { name: "Claude · off 实例规则" }, state: "disabled" }),
        // 旧形态（不一致 + sync 动作）在别的域会算待同步；规则不再有「应该写成什么样」，不能进一键同步
        makeElement({ id: "rules:profile:cc-odd", domain: "rules", spec: { name: "odd" }, state: "drifted", actions: [makeAction("sync")] }),
      ]),
    });

    const rules = groups.find((group) => group.domain === "rules");
    expect(rules?.items.map((item) => [item.id, item.sync, item.reason ?? null])).toEqual([
      ["rules:codex_global", "ok", null],
      ["rules:claude_global", "absent", null],
      ["rules:profile:cc-link", "decision", "目标路径是符号链接，拒绝操作以保证安全"],
      ["rules:profile:cc-off", "disabled", null],
      ["rules:profile:cc-odd", "decision", "规则文件不能安全读写"],
    ]);
    expect(rules?.items.some((item) => item.sync === "pending")).toBe(false);
    expect(syncableOf(groups)).toEqual([]);
  });
});

describe("buildOverview Hooks", () => {
  it("Hooks 以实际配置为准：有清理建议或旧版未生效建议也不算待同步，不进一键同步", () => {
    const write = [makeAction("write")];
    const groups = buildOverview({
      hooks: status("hooks", [
        makeElement({
          id: "hooks:claude",
          domain: "hooks",
          spec: { name: "Claude Code Hooks" },
          observed: {
            cleanup: [{ id: "cleanup-0123456789abcdef", event: "Stop", matcher: null, command: "old.sh" }],
            legacy: [{ id: "legacy-0123456789abcdef", source_key: "hooks.claude", event: "Stop", matcher: null, command: "x" }],
          },
          state: "managed-ok",
          actions: write,
        }),
        makeElement({ id: "hooks:codex", domain: "hooks", spec: { name: "Codex Hooks" }, state: "missing", actions: write }),
        makeElement({
          id: "hooks:profile:cc-link",
          domain: "hooks",
          spec: { name: "Claude link 实例 Hooks" },
          observed: { blocked_reason: "目标路径是符号链接，拒绝操作以保证安全" },
          state: "unmanaged-detected",
        }),
        // 旧形态（drifted + sync）在别的域会算待同步；hooks 不再有库内期望，不能进一键同步
        makeElement({ id: "hooks:profile:cc-odd", domain: "hooks", spec: { name: "odd" }, state: "drifted", actions: [makeAction("sync")] }),
      ]),
    });
    const hooks = groups.find((group) => group.domain === "hooks");
    expect(hooks?.items.map((item) => [item.id, item.sync, item.reason ?? null])).toEqual([
      ["hooks:claude", "ok", null],
      ["hooks:codex", "absent", null],
      ["hooks:profile:cc-link", "decision", "目标路径是符号链接，拒绝操作以保证安全"],
      ["hooks:profile:cc-odd", "decision", "Hooks 配置文件不能安全读写"],
    ]);
    expect(syncableOf(groups)).toEqual([]);
  });
});

describe("syncableOf", () => {
  it("同一条命令的多个待同步项只执行一次", () => {
    const item = { domain: "demo", version: null, upstream: null, update: "none" as const, sync: "pending" as const, syncArgs: ["--only", "demo:x"] };
    expect(
      syncableOf([
        { domain: "demo", label: "示例", pending: 2, items: [{ ...item, id: "a", name: "a" }, { ...item, id: "b", name: "b" }] },
      ]),
    ).toEqual([{ domain: "demo", name: "示例 2 项", args: ["--only", "demo:x"] }]);
  });
});
