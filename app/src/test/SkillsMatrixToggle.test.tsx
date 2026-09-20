import { describe, expect, it, vi } from "vitest";
import { waitFor } from "@testing-library/react";
import { toast } from "sonner";
import SkillsPage from "../pages/SkillsPage";
import { makeApplyResult, makeElement, makeEnvelope } from "./fixtures";
import { renderWithEngine, screen, userEvent } from "./render-with-engine";
import type { Envelope } from "../lib/contract";

/** 逐技能操作链路：observed.actions 按钮 → 直接 skills:apply（--only/--action/--agent，带 run_id），
 * 不再先取 plan、不开预览弹窗。没有 policy:set 预写 enabled：动作完全由 apply 携带 --action 表达。 */
function routes(): Record<string, Envelope> {
  return {
    "skills:status": makeEnvelope(
      {
        domain: "skills",
        elements: [
          makeElement({
            id: "skill:agent-skills",
            domain: "skills",
            spec: { url: "https://github.com/example/agent-skills.git", skill_paths: ["alpha"], agents: ["claude-code", "codex"] },
            observed: { repo_cache: "/repo/agent-skills", repo_cached: true, owner_marked: true, discovered: ["alpha"], discovery_inbox: [], actions: ["update", "enable", "disable"] },
            state: "managed-ok",
          }),
          makeElement({
            id: "skill:agent-skills/alpha",
            domain: "skills",
            spec: { collection: "agent-skills", name: "alpha", source_path: "/repo/.../alpha" },
            policy: { enabled: true, agents: { "claude-code": true, codex: false } },
            observed: {
              agents: {
                "claude-code": { state: "linked", link_path: "/home/.claude/skills/alpha", target_dir_exists: true, assigned: true, enabled: true, actions: ["update", "disable", "remove"] },
                codex: { state: "disabled", link_path: "/home/.codex/skills/alpha", target_dir_exists: true, assigned: true, enabled: false, actions: ["enable", "remove"] },
              },
              discovery_inbox: false,
            },
            state: "managed-ok",
          }),
        ],
      },
      { command: "skills.status" },
    ),
    "skills:apply": makeEnvelope(makeApplyResult({ domain: "skills" }), { command: "skills.apply" }),
  };
}

describe("Skills 逐技能操作链路", () => {
  it("codex 行点「启用」→ 直接 skills:apply 带 --action enable --agent codex 与 run_id，不取 plan、不开弹窗、零 policy:set", async () => {
    const { calls } = renderWithEngine(<SkillsPage />, routes());

    await userEvent.click(await screen.findByRole("tab", { name: "全部技能" }));
    await userEvent.click(await screen.findByTestId("action-enable-skill:agent-skills/alpha-codex"));

    await waitFor(() => expect(calls.some((call) => call.command === "skills:apply")).toBe(true));
    const applyCalls = calls.filter((call) => call.command === "skills:apply");
    expect(applyCalls).toHaveLength(1);
    expect(applyCalls[0].args).toEqual(["--only", "skill:agent-skills/alpha", "--action", "enable", "--agent", "codex"]);
    expect(applyCalls[0].args).not.toContain("--dry-run");
    expect(applyCalls[0].options?.runId).toMatch(/^gui-/);
    expect(calls.some((call) => call.command.endsWith(":plan"))).toBe(false);
    expect(calls.some((call) => call.command === "policy:set")).toBe(false);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("apply 被引擎拒绝：既有执行 toast 报告失败，不重试、零 policy:set", async () => {
    const rejected = routes();
    rejected["skills:apply"] = makeEnvelope(
      {},
      {
        command: "skills.apply",
        ok: false,
        errors: [{ code: "APPLY_FAILED", element: null, message: "目标被外部修改，拒绝覆盖" }],
      },
    );
    const toastErrorSpy = vi.spyOn(toast, "error");
    const { calls } = renderWithEngine(<SkillsPage />, rejected);

    await userEvent.click(await screen.findByRole("tab", { name: "全部技能" }));
    await userEvent.click(await screen.findByTestId("action-enable-skill:agent-skills/alpha-codex"));

    await waitFor(() => expect(toastErrorSpy).toHaveBeenCalled());
    expect(toastErrorSpy).toHaveBeenCalledWith(expect.stringContaining("目标被外部修改，拒绝覆盖"));
    const applyCalls = calls.filter((call) => call.command === "skills:apply");
    expect(applyCalls).toHaveLength(1);
    expect(calls.some((call) => call.command.endsWith(":plan"))).toBe(false);
    expect(calls.some((call) => call.command === "policy:set")).toBe(false);
    expect(screen.queryByRole("dialog")).toBeNull();
  });
});
