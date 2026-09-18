import { describe, expect, it, vi } from "vitest";
import { fireEvent, waitFor } from "@testing-library/react";
import { toast } from "sonner";
import SkillsPage from "../pages/SkillsPage";
import type { Envelope } from "../lib/contract";
import { makeApplyResult, makeElement, makeEnvelope } from "./fixtures";
import { renderWithEngine, screen, userEvent, within } from "./render-with-engine";

type Routes = Record<string, Envelope | ((args: string[]) => Promise<Envelope>)>;

/**
 * 批量测试矩阵：
 * - alpha: claude-code 可启用、codex 可禁用
 * - beta: claude-code 与 codex 均可启用
 * - gamma: claude-code 无动作、codex 无 cell（均不可勾选）
 */
function batchRoutes(): Routes {
  return {
    "skills:status": makeEnvelope(
      {
        domain: "skills",
        elements: [
          makeElement({
            id: "skill:agent-skills",
            domain: "skills",
            spec: {
              url: "https://github.com/example/agent-skills.git",
              skill_paths: ["alpha", "beta", "gamma"],
              agents: ["claude-code", "codex"],
            },
            observed: {
              repo_cache: "/repo/agent-skills",
              repo_cached: true,
              owner_marked: true,
              discovered: ["alpha", "beta", "gamma"],
              discovery_inbox: [],
              extra_links: [],
              actions: ["update", "disable", "remove"],
              blocked_reason: null,
            },
            state: "managed-ok",
          }),
          makeElement({
            id: "skill:agent-skills/alpha",
            domain: "skills",
            spec: { collection: "agent-skills", name: "alpha", source_path: "/repo/.../alpha" },
            policy: { enabled: true, agents: { "claude-code": true, codex: true } },
            observed: {
              agents: {
                "claude-code": { state: "disabled", link_path: null, target_dir_exists: true, assigned: true, enabled: false, actions: ["enable"] },
                codex: { state: "linked", link_path: "/home/.codex/skills/alpha", target_dir_exists: true, assigned: true, enabled: true, actions: ["disable"] },
              },
              discovery_inbox: false,
            },
            state: "managed-ok",
          }),
          makeElement({
            id: "skill:agent-skills/beta",
            domain: "skills",
            spec: { collection: "agent-skills", name: "beta", source_path: "/repo/.../beta" },
            policy: { enabled: true, agents: { "claude-code": true, codex: true } },
            observed: {
              agents: {
                "claude-code": { state: "missing-link", link_path: "/home/.claude/skills/beta", target_dir_exists: true, assigned: true, enabled: true, actions: ["enable"] },
                codex: { state: "disabled", link_path: null, target_dir_exists: false, assigned: true, enabled: false, actions: ["enable"] },
              },
              discovery_inbox: false,
            },
            state: "managed-ok",
          }),
          makeElement({
            id: "skill:agent-skills/gamma",
            domain: "skills",
            spec: { collection: "agent-skills", name: "gamma", source_path: "/repo/.../gamma" },
            policy: { enabled: true, agents: {} },
            observed: {
              agents: {
                "claude-code": { state: "unsupported", assigned: false, enabled: false, actions: [] },
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

/** 计划外链接批量矩阵：alpha 外部链接（managed=false）、beta 受管残留链接（managed=true），均可 add/remove。 */
function batchExtraLinkRoutes(): Routes {
  return {
    "skills:status": makeEnvelope(
      {
        domain: "skills",
        elements: [
          makeElement({
            id: "skill:agent-skills",
            domain: "skills",
            spec: { url: "https://github.com/example/agent-skills.git", skill_paths: ["alpha", "beta"], agents: ["codex"] },
            observed: {
              repo_cache: "/repo/agent-skills",
              repo_cached: true,
              owner_marked: true,
              discovered: ["alpha", "beta"],
              discovery_inbox: [],
              extra_links: [],
              actions: ["update", "disable", "remove"],
              blocked_reason: null,
            },
            state: "managed-ok",
          }),
          makeElement({
            id: "skill:agent-skills/alpha",
            domain: "skills",
            spec: { collection: "agent-skills", name: "alpha", source_path: "/repo/.../alpha" },
            policy: { enabled: true, agents: { codex: false } },
            observed: {
              agents: {
                codex: { state: "extra-link", link_path: "/home/.codex/skills/alpha", target_dir_exists: true, assigned: false, enabled: false, managed: false, version: null, actions: ["add", "remove"] },
              },
              discovery_inbox: false,
            },
            state: "managed-ok",
          }),
          makeElement({
            id: "skill:agent-skills/beta",
            domain: "skills",
            spec: { collection: "agent-skills", name: "beta", source_path: "/repo/.../beta" },
            policy: { enabled: true, agents: { codex: false } },
            observed: {
              agents: {
                codex: { state: "extra-link", link_path: "/home/.codex/skills/beta", target_dir_exists: true, assigned: false, enabled: false, managed: true, version: null, actions: ["add", "remove"] },
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

async function openSkillsTab() {
  await userEvent.click(await screen.findByRole("tab", { name: "全部技能" }));
}

describe("SkillsPage 全部技能批量操作", () => {
  it("勾选列：仅有可用动作的行可勾选；全选当前可见行与 indeterminate 态", async () => {
    renderWithEngine(<SkillsPage />, batchRoutes());
    await openSkillsTab();

    const alphaClaude = await screen.findByTestId("select-skill:agent-skills/alpha-claude-code");
    expect(alphaClaude).toHaveAttribute("aria-label", "选择 alpha @ claude-code");
    // 无动作行与 cell 缺失行均不可勾选
    expect(screen.getByTestId("select-skill:agent-skills/gamma-claude-code")).toBeDisabled();
    expect(screen.getByTestId("select-skill:agent-skills/gamma-codex")).toBeDisabled();

    const selectAll = screen.getByTestId("select-all-visible") as HTMLInputElement;
    expect(selectAll).toHaveAttribute("aria-label", "全选当前可见行");
    expect(selectAll).not.toBeChecked();
    expect(selectAll.indeterminate).toBe(false);

    // 勾选一行 → 表头 indeterminate
    await userEvent.click(alphaClaude);
    expect(alphaClaude).toBeChecked();
    expect(selectAll.indeterminate).toBe(true);
    expect(screen.getByTestId("batch-toolbar")).toHaveTextContent("已选 1 项");

    // 全选 → 4 个可勾选行全部选中，indeterminate 消除
    await userEvent.click(selectAll);
    expect(selectAll).toBeChecked();
    expect(selectAll.indeterminate).toBe(false);
    expect(screen.getByTestId("batch-toolbar")).toHaveTextContent("已选 4 项");
    expect(screen.getByTestId("select-skill:agent-skills/beta-codex")).toBeChecked();
    expect(screen.getByTestId("select-skill:agent-skills/gamma-claude-code")).not.toBeChecked();

    // 再次点击全选 → 清空选择，工具栏消失
    await userEvent.click(selectAll);
    expect(screen.queryByTestId("batch-toolbar")).toBeNull();
    expect(screen.getByTestId("select-skill:agent-skills/alpha-claude-code")).not.toBeChecked();
  });

  it("筛选/搜索变化时剔除不再可见的选中项", async () => {
    renderWithEngine(<SkillsPage />, batchRoutes());
    await openSkillsTab();
    await userEvent.click(await screen.findByTestId("select-skill:agent-skills/alpha-claude-code"));
    await userEvent.click(screen.getByTestId("select-skill:agent-skills/alpha-codex"));
    expect(screen.getByTestId("batch-toolbar")).toHaveTextContent("已选 2 项");

    // 助手筛选为 codex：claude-code 行选中项被剔除
    fireEvent.change(screen.getByTestId("filter-agent-select"), { target: { value: "codex" } });
    await waitFor(() => expect(screen.getByTestId("batch-toolbar")).toHaveTextContent("已选 1 项"));
    expect(screen.getByTestId("select-skill:agent-skills/alpha-codex")).toBeChecked();

    // 搜索到无匹配：选中清空，工具栏消失
    fireEvent.change(screen.getByTestId("skills-search"), { target: { value: "不存在的技能" } });
    await waitFor(() => expect(screen.queryByTestId("batch-toolbar")).toBeNull());
  });

  it("批量工具栏：按钮按选中集合动作子集显示并标注适用行数，可清除选择", async () => {
    renderWithEngine(<SkillsPage />, batchRoutes());
    await openSkillsTab();
    await userEvent.click(await screen.findByTestId("select-all-visible"));

    const toolbar = screen.getByTestId("batch-toolbar");
    // alpha@claude-code + beta@claude-code + beta@codex 可启用；仅 alpha@codex 可禁用
    expect(within(toolbar).getByTestId("batch-enable")).toHaveTextContent("批量启用 (3)");
    expect(within(toolbar).getByTestId("batch-disable")).toHaveTextContent("批量禁用 (1)");
    // 选中集合无计划外链接行：纳入管理/取消链接不显示
    expect(within(toolbar).queryByTestId("batch-add")).toBeNull();
    expect(within(toolbar).queryByTestId("batch-remove")).toBeNull();

    await userEvent.click(within(toolbar).getByTestId("batch-clear"));
    expect(screen.queryByTestId("batch-toolbar")).toBeNull();
    expect(screen.getByTestId("select-all-visible")).not.toBeChecked();
  });

  it("批量启用：逐行顺序调用 skills:apply，参数精确、独立 run_id、不弹确认，只发汇总 toast，结束后清空选择", async () => {
    const successSpy = vi.spyOn(toast, "success");
    const errorSpy = vi.spyOn(toast, "error");
    try {
      const { calls } = renderWithEngine(<SkillsPage />, batchRoutes());
      await openSkillsTab();
      await userEvent.click(await screen.findByTestId("select-skill:agent-skills/alpha-claude-code"));
      await userEvent.click(screen.getByTestId("select-skill:agent-skills/beta-claude-code"));
      await userEvent.click(screen.getByTestId("batch-enable"));

      await screen.findByText(/批量启用完成：成功 2，失败 0/);
      const applyCalls = calls.filter((call) => call.command === "skills:apply");
      expect(applyCalls).toHaveLength(2);
      expect(applyCalls[0].args).toEqual(["--only", "skill:agent-skills/alpha", "--action", "enable", "--agent", "claude-code"]);
      expect(applyCalls[1].args).toEqual(["--only", "skill:agent-skills/beta", "--action", "enable", "--agent", "claude-code"]);
      expect(applyCalls[0].args).not.toContain("--allow-takeover");
      expect(applyCalls[0].options?.runId).toMatch(/^gui-/);
      expect(applyCalls[1].options?.runId).toMatch(/^gui-/);
      expect(applyCalls[0].options?.runId).not.toBe(applyCalls[1].options?.runId);
      expect(screen.queryByRole("dialog")).toBeNull();
      expect(screen.queryByTestId("batch-toolbar")).toBeNull();
      // 仅汇总 toast：两行单行结果 toast（成功/失败）均被压掉
      expect(successSpy).toHaveBeenCalledTimes(1);
      expect(successSpy).toHaveBeenCalledWith(expect.stringContaining("批量启用完成：成功 2，失败 0"));
      expect(errorSpy).not.toHaveBeenCalled();
    } finally {
      successSpy.mockRestore();
      errorSpy.mockRestore();
    }
  });

  it("批量纳入管理含外部链接行：一次汇总确认后，仅外部链接行携带 --allow-takeover", async () => {
    const { calls } = renderWithEngine(<SkillsPage />, batchExtraLinkRoutes());
    await openSkillsTab();
    await userEvent.click(await screen.findByTestId("select-skill:agent-skills/alpha-codex"));
    await userEvent.click(screen.getByTestId("select-skill:agent-skills/beta-codex"));

    await userEvent.click(screen.getByTestId("batch-add"));
    const dialog = await screen.findByRole("dialog", { name: "批量纳入管理（2 行）" });
    expect(dialog).toHaveTextContent("将对 2 行执行纳入管理。其中 1 行存在外部链接内容，确认后将被替换");

    await userEvent.click(screen.getByTestId("confirm-modal-confirm"));
    await screen.findByText(/批量纳入管理完成：成功 2，失败 0/);
    const applyCalls = calls.filter((call) => call.command === "skills:apply");
    expect(applyCalls).toHaveLength(2);
    expect(applyCalls[0].args).toEqual(["--only", "skill:agent-skills/alpha", "--action", "add", "--agent", "codex", "--allow-takeover"]);
    expect(applyCalls[1].args).toEqual(["--only", "skill:agent-skills/beta", "--action", "add", "--agent", "codex"]);
    expect(applyCalls[1].args).not.toContain("--allow-takeover");
  });

  it("批量取消链接：取消汇总确认则零 apply 且保留选择；仅受管残留行时不弹确认直执", async () => {
    const { calls } = renderWithEngine(<SkillsPage />, batchExtraLinkRoutes());
    await openSkillsTab();

    // 外部链接行 → 弹汇总确认；取消后零 apply、选择保留
    await userEvent.click(await screen.findByTestId("select-skill:agent-skills/alpha-codex"));
    await userEvent.click(screen.getByTestId("batch-remove"));
    const dialog = await screen.findByRole("dialog", { name: "批量取消链接（1 行）" });
    expect(dialog).toHaveTextContent(/其中 1 行存在外部链接内容，确认后将被移除/);
    await userEvent.click(screen.getByRole("button", { name: "取消" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(calls.some((call) => call.command === "skills:apply")).toBe(false);
    expect(screen.getByTestId("batch-toolbar")).toHaveTextContent("已选 1 项");

    // 改选受管残留行（managed=true）：不弹确认直执，不带 --allow-takeover
    await userEvent.click(screen.getByTestId("select-skill:agent-skills/alpha-codex"));
    await userEvent.click(screen.getByTestId("select-skill:agent-skills/beta-codex"));
    await userEvent.click(screen.getByTestId("batch-remove"));
    await screen.findByText(/批量取消链接完成：成功 1，失败 0/);
    expect(screen.queryByRole("dialog")).toBeNull();
    const applyCalls = calls.filter((call) => call.command === "skills:apply");
    expect(applyCalls).toHaveLength(1);
    expect(applyCalls[0].args).toEqual(["--only", "skill:agent-skills/beta", "--action", "remove", "--agent", "codex"]);
  });

  it("批量启用部分失败：汇总 toast 报告成功/失败数并 console.error 每行原因，不中断后续行", async () => {
    const routes = batchRoutes();
    routes["skills:apply"] = async (args: string[]) => {
      if (args.includes("skill:agent-skills/alpha")) {
        return makeEnvelope(makeApplyResult({ domain: "skills" }), {
          command: "skills.apply",
          ok: false,
          errors: [{ code: "EXTERNAL_MODIFICATION", element: "skill:agent-skills/alpha", message: "目标文件被外部修改" }],
        });
      }
      return makeEnvelope(makeApplyResult({ domain: "skills" }), { command: "skills.apply" });
    };
    const consoleSpy = vi.spyOn(console, "error").mockImplementation(() => {});
    const toastErrorSpy = vi.spyOn(toast, "error");
    try {
      const { calls } = renderWithEngine(<SkillsPage />, routes);
      await openSkillsTab();
      await userEvent.click(await screen.findByTestId("select-skill:agent-skills/alpha-claude-code"));
      await userEvent.click(screen.getByTestId("select-skill:agent-skills/beta-claude-code"));
      await userEvent.click(screen.getByTestId("batch-enable"));

      await screen.findByText(/批量启用完成：成功 1，失败 1/);
      expect(screen.getByText(/失败详情见任务记录/)).toBeInTheDocument();
      // 首行失败不中断后续行
      const applyCalls = calls.filter((call) => call.command === "skills:apply");
      expect(applyCalls).toHaveLength(2);
      expect(applyCalls[1].args).toContain("skill:agent-skills/beta");
      expect(consoleSpy).toHaveBeenCalledWith(expect.stringContaining("alpha @ claude-code 失败：目标文件被外部修改"));
      // 单行失败 toast 被压掉，仅汇总 error toast 一次
      expect(toastErrorSpy).toHaveBeenCalledTimes(1);
      expect(toastErrorSpy).toHaveBeenCalledWith(expect.stringContaining("批量启用完成：成功 1，失败 1"));
      expect(screen.queryByTestId("batch-toolbar")).toBeNull();
    } finally {
      consoleSpy.mockRestore();
      toastErrorSpy.mockRestore();
    }
  });

  it("批量执行期间：工具栏按钮、行勾选框与全选框禁用，结束后恢复", async () => {
    let release: ((envelope: Envelope) => void) | undefined;
    const routes = batchRoutes();
    routes["skills:apply"] = () =>
      new Promise<Envelope>((resolve) => {
        release = resolve;
      });
    renderWithEngine(<SkillsPage />, routes);
    await openSkillsTab();
    await userEvent.click(await screen.findByTestId("select-skill:agent-skills/alpha-claude-code"));
    await userEvent.click(screen.getByTestId("batch-enable"));

    await waitFor(() => expect(screen.getByTestId("batch-enable")).toBeDisabled());
    expect(screen.getByTestId("select-skill:agent-skills/alpha-claude-code")).toBeDisabled();
    expect(screen.getByTestId("select-all-visible")).toBeDisabled();
    expect(screen.getByTestId("batch-clear")).toBeDisabled();

    release?.(makeEnvelope(makeApplyResult({ domain: "skills" }), { command: "skills.apply" }));
    await screen.findByText(/批量启用完成：成功 1，失败 0/);
    expect(screen.queryByTestId("batch-toolbar")).toBeNull();
    expect(screen.getByTestId("select-skill:agent-skills/alpha-claude-code")).toBeEnabled();
  });

  it("单行操作（非批量）：执行通道默认行为不变，照常发单行结果 toast", async () => {
    const successSpy = vi.spyOn(toast, "success");
    try {
      const { calls } = renderWithEngine(<SkillsPage />, batchRoutes());
      await openSkillsTab();
      await userEvent.click(await screen.findByTestId("action-enable-skill:agent-skills/alpha-claude-code"));

      await waitFor(() => expect(successSpy.mock.calls[successSpy.mock.calls.length - 1][0]).toBe("启用 alpha @ claude-code完成"));
      expect(calls.filter((call) => call.command === "skills:apply")).toHaveLength(1);
    } finally {
      successSpy.mockRestore();
    }
  });
});
