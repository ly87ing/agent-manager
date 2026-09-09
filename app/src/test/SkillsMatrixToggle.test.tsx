import { describe, expect, it } from "vitest";
import SkillsPage from "../pages/SkillsPage";
import { makeApplyResult, makeElement, makeEnvelope, makePlanAction } from "./fixtures";
import { renderWithEngine, screen, userEvent } from "./render-with-engine";
import type { Envelope } from "../lib/contract";

/** 逐技能操作链路：observed.actions 按钮 → skills:plan（--only/--action/--agent）→ 预览确认 → skills:apply（带 run_id）。
 * 新契约下没有 policy:set 预写 enabled：动作完全由 plan/apply 携带 --action 表达。 */
function routes(): Record<string, Envelope> {
  return {
    "skills:status": makeEnvelope(
      {
        domain: "skills",
        elements: [
          makeElement({
            id: "skill:agent-skills",
            domain: "skills",
            spec: { url: "https://github.com/example/agent-skills.git", skill_paths: ["alpha"], agents: ["claude-code", "codex"], matrix_mode: true, new_skill_default: "disabled" },
            observed: { repo_cache: "/repo/x", repo_cached: true, owner_marked: true, discovered: ["alpha"], discovery_inbox: [], extra_links: [], actions: ["update", "remove"] },
            state: "managed-ok",
          }),
          makeElement({
            id: "skill:agent-skills/alpha",
            domain: "skills",
            spec: { collection: "agent-skills", name: "alpha", source_path: "/repo/x/alpha" },
            policy: { enabled: true, agents: { "claude-code": true, codex: false } },
            observed: {
              agents: {
                "claude-code": { state: "linked", link_path: "/home/.claude/skills/alpha", target_dir_exists: true, assigned: true, enabled: true, actions: ["disable", "remove"] },
                codex: { state: "disabled", link_path: "/home/.codex/skills/alpha", target_dir_exists: true, assigned: true, enabled: false, actions: ["enable"] },
              },
              discovery_inbox: false,
            },
            state: "managed-ok",
          }),
        ],
      },
      { command: "skills.status" },
    ),
    "skills:plan": makeEnvelope(
      {
        domain: "skills",
        actions: [makePlanAction({ op: "symlink", element: "skill:agent-skills/alpha", target: "/home/.codex/skills/alpha", risk: "exec-local", note: "链接到集合缓存" })],
        summary: { read: 0, "write-local": 0, "exec-local": 1, network: 0, "install-remove": 0 },
      },
      { command: "skills.plan" },
    ),
    "skills:apply": makeEnvelope(makeApplyResult({ domain: "skills" }), { command: "skills.apply" }),
  };
}

describe("Skills 逐技能操作链路", () => {
  it("codex 行点「启用」→ skills:plan 带 --action enable --agent codex → 确认 → skills:apply 带 run_id，全程零 policy:set", async () => {
    const { calls } = renderWithEngine(<SkillsPage />, routes());

    await userEvent.click(await screen.findByRole("tab", { name: "全部技能" }));
    await userEvent.click(await screen.findByTestId("action-enable-skill:agent-skills/alpha-codex"));

    // 无 policy:set 预写 enabled
    expect(calls.some((call) => call.command === "policy:set")).toBe(false);

    // plan 预览弹窗：限定本 skill 本助手
    const dialog = await screen.findByRole("dialog", { name: /将要执行的操作/ });
    expect(dialog).toHaveTextContent("启用 alpha @ codex");
    expect(dialog).toHaveTextContent("/home/.codex/skills/alpha");
    const planCall = calls.find((call) => call.command === "skills:plan");
    expect(planCall).toBeDefined();
    expect(planCall!.args).toEqual(["--only", "skill:agent-skills/alpha", "--action", "enable", "--agent", "codex"]);

    // 确认 → skills:apply 同参数，携带 GUI 生成的 run_id
    await userEvent.click(screen.getByTestId("plan-confirm-button"));
    const applyCall = calls.find((call) => call.command === "skills:apply");
    expect(applyCall).toBeDefined();
    expect(applyCall!.args).toEqual(["--only", "skill:agent-skills/alpha", "--action", "enable", "--agent", "codex"]);
    expect(applyCall!.options?.runId).toMatch(/^gui-/);

    // 链路顺序：skills:plan → skills:apply
    const order = ["skills:plan", "skills:apply"].map((command) => calls.findIndex((call) => call.command === command));
    expect(order[0]).toBeGreaterThanOrEqual(0);
    expect(order[0]).toBeLessThan(order[1]);
  });

  it("plan 预览点取消：零写入（无 policy:set、无 skills:apply）", async () => {
    const { calls } = renderWithEngine(<SkillsPage />, routes());

    await userEvent.click(await screen.findByRole("tab", { name: "全部技能" }));
    await userEvent.click(await screen.findByTestId("action-enable-skill:agent-skills/alpha-codex"));
    const dialog = await screen.findByRole("dialog", { name: /将要执行的操作/ });
    expect(dialog).toHaveTextContent("启用 alpha @ codex");

    await userEvent.click(screen.getByRole("button", { name: "取消" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(calls.some((call) => call.command === "policy:set")).toBe(false);
    expect(calls.some((call) => call.command === "skills:apply")).toBe(false);
  });

  it("plan 被引擎拒绝时预览弹窗内展示错误且确认按钮不可用，不执行 apply", async () => {
    const rejected = routes();
    rejected["skills:plan"] = makeEnvelope(
      {},
      {
        command: "skills.plan",
        ok: false,
        errors: [{ code: "PLAN_FAILED", element: null, message: "引擎拒绝生成计划" }],
      },
    );
    const { calls } = renderWithEngine(<SkillsPage />, rejected);

    await userEvent.click(await screen.findByRole("tab", { name: "全部技能" }));
    await userEvent.click(await screen.findByTestId("action-enable-skill:agent-skills/alpha-codex"));

    const dialog = await screen.findByRole("dialog", { name: /将要执行的操作/ });
    expect(dialog).toHaveTextContent("引擎拒绝生成计划");
    expect(screen.getByTestId("plan-confirm-button")).toBeDisabled();
    expect(calls.some((call) => call.command === "skills:apply")).toBe(false);
    expect(calls.some((call) => call.command === "policy:set")).toBe(false);
  });
});
