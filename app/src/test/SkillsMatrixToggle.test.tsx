import { describe, expect, it } from "vitest";
import SkillsPage from "../pages/SkillsPage";
import { makeApplyResult, makeElement, makeEnvelope, makePlanAction, makePolicyData } from "./fixtures";
import { renderWithEngine, screen, userEvent } from "./render-with-engine";
import type { Envelope } from "../lib/contract";

/** 矩阵点击切换的完整链路：policy.set 参数 → plan 预览 → skills:apply（带 run_id）。 */
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
            observed: { repo_cache: "/repo/x", repo_cached: true, owner_marked: true, discovered: ["alpha"], discovery_inbox: [], extra_links: [] },
            state: "managed-ok",
          }),
          makeElement({
            id: "skill:agent-skills/alpha",
            domain: "skills",
            spec: { collection: "agent-skills", name: "alpha", source_path: "/repo/x/alpha" },
            policy: { enabled: true, agents: { "claude-code": true, codex: false } },
            observed: {
              agents: {
                "claude-code": { state: "linked", link_path: "/home/.claude/skills/alpha", target_dir_exists: true },
                codex: { state: "disabled", link_path: "/home/.codex/skills/alpha", target_dir_exists: true },
              },
              discovery_inbox: false,
            },
            state: "managed-ok",
          }),
        ],
      },
      { command: "skills.status" },
    ),
    "policy:set": makeEnvelope(makePolicyData({ layers: null, effective: null, attribution: null, layer: "local", key: "skills.agent-skills.skills.alpha.agents.codex", value: true, path: "/repo/config/manager.local.json" }), { command: "policy.set" }),
    "skills:plan": makeEnvelope(
      {
        domain: "skills",
        actions: [
          makePlanAction({ op: "symlink", element: "skill:agent-skills/alpha", target: "/home/.codex/skills/alpha", risk: "exec-local", note: "链接到集合缓存" }),
          makePlanAction({ op: "symlink", element: "skill:agent-skills/other", target: "/home/.codex/skills/other", risk: "exec-local" }),
        ],
        summary: { read: 0, "write-local": 0, "exec-local": 2, network: 0, "install-remove": 0 },
      },
      { command: "skills.plan" },
    ),
    "skills:apply": makeEnvelope(makeApplyResult({ domain: "skills" }), { command: "skills.apply" }),
  };
}

describe("Skills 矩阵点击切换", () => {
  it("disabled 格点击 → policy.set enable=true（默认 local 层）→ plan 预览过滤到本 skill → 确认 → skills:apply 带 run_id", async () => {
    const { calls } = renderWithEngine(<SkillsPage />, routes());

    // 点击 codex 列的 disabled 格
    const cell = await screen.findByTestId("skill-cell-skill:agent-skills/alpha-codex");
    await userEvent.click(cell);

    // policy:set 先行，参数正确
    const setCall = calls.find((call) => call.command === "policy:set");
    expect(setCall).toBeDefined();
    expect(setCall!.args).toEqual(["--layer", "local", "--key", "skills.agent-skills.skills.alpha.agents.codex", "--value", "true"]);

    // plan 预览弹窗：动作过滤到本 skill（other 被滤掉）
    const dialog = await screen.findByRole("dialog");
    expect(dialog).toHaveTextContent("启用 alpha @ codex");
    expect(dialog).toHaveTextContent("/home/.codex/skills/alpha");
    expect(dialog).not.toHaveTextContent("/home/.codex/skills/other");

    // 确认 → skills:apply，携带 GUI 生成的 run_id
    await userEvent.click(screen.getByTestId("plan-confirm-button"));
    const applyCall = calls.find((call) => call.command === "skills:apply");
    expect(applyCall).toBeDefined();
    expect(applyCall!.args).toEqual(["--only", "skill:agent-skills/alpha", "--agent", "codex"]);
    expect(applyCall!.options?.runId).toMatch(/^gui-/);

    // 链路顺序：policy:set → skills:plan → skills:apply
    const order = ["policy:set", "skills:plan", "skills:apply"].map((command) => calls.findIndex((call) => call.command === command));
    expect(order[0]).toBeGreaterThanOrEqual(0);
    expect(order[0]).toBeLessThan(order[1]);
    expect(order[1]).toBeLessThan(order[2]);
  });

  it("policy.set 被引擎拒绝时不出预览弹窗", async () => {
    const rejected = routes();
    rejected["policy:set"] = makeEnvelope({}, {
      command: "policy.set",
      ok: false,
      errors: [{ code: "POLICY_SECRET_GATE", element: null, message: "疑似明文敏感信息，拒绝写入策略层" }],
    });
    const { calls } = renderWithEngine(<SkillsPage />, rejected);

    await userEvent.click(await screen.findByTestId("skill-cell-skill:agent-skills/alpha-codex"));
    expect(await screen.findByText(/保存被拒绝/)).toBeInTheDocument();
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(calls.some((call) => call.command === "skills:apply")).toBe(false);
  });
});
