import { describe, expect, it } from "vitest";
import { buildSyncPlan, groupByDomain } from "../lib/syncPlan";
import { makeElement, makeEnvelope } from "./fixtures";
import type { Envelope, StatusData } from "../lib/contract";

function status(domain: string, elements: ReturnType<typeof makeElement>[]): Envelope<StatusData> {
  return makeEnvelope({ domain, elements }, { command: `${domain}.status` }) as Envelope<StatusData>;
}

describe("全局同步计划", () => {
  it("整目标的域：给了 sync 动作才算可同步", () => {
    const plan = buildSyncPlan([
      status("rules", [
        makeElement({ id: "rules:claude_global", domain: "rules", state: "drifted", actions: [{ id: "sync", risk: "write-local", needs_lock: true, offline_ok: true }] }),
        makeElement({ id: "rules:codex_global", domain: "rules", state: "managed-ok", actions: [] }),
      ]),
    ]);
    expect(plan.ready.map((i) => i.id)).toEqual(["rules:claude_global"]);
    expect(plan.ready[0].args).toEqual(["--only", "rules:claude_global"]);
    expect(plan.decisions).toEqual([]);
  });

  it("被外部改过的进「要你决定」，绝不混进批量", () => {
    const plan = buildSyncPlan([
      status("hooks", [
        makeElement({
          id: "hooks:codex",
          domain: "hooks",
          state: "unmanaged-detected",
          actions: [],
          observed: { blocked_reason: "受管 hook 被外部修改，拒绝覆盖" },
        }),
      ]),
    ]);
    expect(plan.ready).toEqual([]);
    expect(plan.decisions[0].reason).toContain("外部修改");
  });

  it("逐助手的域按单元格算：只有该格给了动作的才进批量", () => {
    const plan = buildSyncPlan([
      status("mcp", [
        makeElement({
          id: "mcp:demo",
          domain: "mcp",
          state: "drifted",
          observed: {
            agents: {
              "claude-code": { state: "not-materialized", assigned: true, actions: ["update", "disable"] },
              codex: { state: "leftover", assigned: false, actions: ["remove"] },
              opencode: { state: "absent", assigned: false, actions: [] },
            },
          },
        }),
      ]),
    ]);
    expect(plan.ready.map((i) => i.id)).toEqual(["mcp:demo@claude-code"]);
    expect(plan.ready[0].args).toEqual(["--only", "mcp:demo", "--agent", "claude-code", "--action", "update"]);
    // leftover 要用户决定；absent 是本来就没有，不催
    expect(plan.decisions.map((i) => i.id)).toEqual(["mcp:demo@codex"]);
  });

  it("用户主动停用的不催", () => {
    const plan = buildSyncPlan([
      status("skills", [
        makeElement({
          id: "skill:repo/one",
          domain: "skills",
          state: "disabled",
          observed: { agents: { "claude-code": { state: "not-assigned", actions: ["add"] } } },
        }),
      ]),
    ]);
    expect(plan.ready).toEqual([]);
    expect(plan.decisions).toEqual([]);
  });

  it("查询失败的域跳过，不当作没事", () => {
    const failed = makeEnvelope({ domain: "rules", elements: [] }, { command: "rules.status", ok: false, errors: [{ code: "X", element: null, message: "boom" }] }) as Envelope<StatusData>;
    const plan = buildSyncPlan([failed, undefined]);
    expect(plan.ready).toEqual([]);
    expect(plan.decisions).toEqual([]);
  });

  it("按固定域顺序分组，结果可预期", () => {
    const groups = groupByDomain([
      { domain: "skills" },
      { domain: "rules" },
      { domain: "mcp" },
    ] as { domain: string }[]);
    expect(groups.map((g) => g.domain)).toEqual(["rules", "mcp", "skills"]);
  });
});
