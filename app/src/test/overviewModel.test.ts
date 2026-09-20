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

  it("逐助手的域：有可执行动作的算待同步，被外部改过的算要你决定", () => {
    const groups = buildOverview({
      mcp: status("mcp", [
        makeElement({
          id: "mcp:chrome-devtools",
          domain: "mcp",
          spec: { name: "chrome-devtools" },
          observed: { agents: { claude: { state: "missing", actions: [{ id: "add" }] } } },
        }),
        makeElement({
          id: "mcp:grok-search",
          domain: "mcp",
          spec: { name: "grok-search" },
          observed: {
            agents: { claude: { state: "unmanaged-detected", blocked_reason: "本工具以外写过", actions: [] } },
          },
        }),
        makeElement({
          id: "mcp:serena",
          domain: "mcp",
          spec: { name: "serena" },
          observed: { agents: { claude: { state: "managed-ok", actions: [] }, codex: { state: "absent" } } },
        }),
      ]),
    });

    const mcp = groups.find((group) => group.domain === "mcp");
    expect(mcp?.items.map((item) => [item.name, item.sync])).toEqual([
      ["chrome-devtools", "pending"],
      ["grok-search", "decision"],
      ["serena", "ok"],
    ]);
    expect(mcp?.pending).toBe(2);
    // 只有待同步的进一键同步；要你决定的不替用户做主
    expect(syncableOf(groups)).toEqual([
      { domain: "mcp", name: "chrome-devtools", args: ["--only", "mcp:chrome-devtools", "--agent", "claude", "--action", "add"] },
    ]);
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

  it("快捷命令按名字合并两个 shell 文件，任一文件没写进去就算待同步", () => {
    const groups = buildOverview({
      aliases: status("aliases", [
        makeElement({
          id: "aliases:zshrc",
          domain: "aliases",
          observed: { entries: [{ name: "cc", state: "applied" }, { name: "yolo", state: "applied" }] },
        }),
        makeElement({
          id: "aliases:zprofile",
          domain: "aliases",
          observed: { entries: [{ name: "cc", state: "applied" }, { name: "yolo", state: "missing" }] },
        }),
      ]),
    });

    expect(groups.find((group) => group.domain === "aliases")?.items.map((item) => [item.name, item.sync])).toEqual([
      ["cc", "ok"],
      ["yolo", "pending"],
    ]);
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
      ]),
    });

    expect(groups[0].items.map((item) => [item.name, item.version, item.sync])).toEqual([
      ["Claude Code", null, "ok"],
      ["OpenCode", null, "absent"],
    ]);
    // 没装不是待办，不进一键同步
    expect(syncableOf(groups)).toEqual([]);
  });

  it("技能来源还没拉下来时保留来源那一行，拉下来后只列单个技能", () => {
    const bare = makeElement({
      id: "skill:agent-skills",
      domain: "skills",
      spec: { name: "agent-skills" },
      state: "not-materialized",
      actions: [makeAction("update"), makeAction("remove")],
    });
    expect(buildOverview({ skills: status("skills", [bare]) })[0].items.map((item) => [item.name, item.sync])).toEqual([
      ["agent-skills", "pending"],
    ]);
    expect(syncableOf(buildOverview({ skills: status("skills", [bare]) }))).toEqual([
      { domain: "skills", name: "agent-skills", args: ["--only", "skill:agent-skills", "--action", "update"] },
    ]);

    const expanded = buildOverview({
      skills: status("skills", [bare, makeElement({ id: "skill:agent-skills/find-skills", domain: "skills", spec: { name: "find-skills" } })]),
    });
    expect(expanded[0].items.map((item) => item.name)).toEqual(["find-skills"]);
  });
});

describe("syncableOf", () => {
  it("快捷命令整块写入，多条待同步只执行一次 apply", () => {
    const groups = buildOverview({
      aliases: status("aliases", [
        makeElement({
          id: "aliases:zshrc",
          domain: "aliases",
          observed: { entries: [{ name: "cc", state: "missing" }, { name: "yolo", state: "missing" }] },
        }),
      ]),
    });

    expect(syncableOf(groups)).toEqual([{ domain: "aliases", name: "终端快捷命令 2 项", args: [] }]);
  });
});
