import { describe, expect, it } from "vitest";
import ClaudeDataPage from "../pages/ClaudeDataPage";
import { claudeDataResultSchema } from "../lib/claude-data";
import { renderWithEngine, screen, userEvent } from "./render-with-engine";
import { makeEnvelope } from "./fixtures";
import { makeClaudeDataPreview, makeClaudeDataResult } from "./claude-data-fixture";

describe("Claude local data maintenance", () => {
  it("does not inspect on entry and cancellation never sends a clear request", async () => {
    const { calls } = renderWithEngine(<ClaudeDataPage />, { "claude-data:preview": makeEnvelope(makeClaudeDataPreview()) });
    expect(calls).toEqual([]);
    expect(screen.getByText("清理 Claude Code 默认用户目录中的会话、账号、记忆、配置与缓存，包括遥测配置。")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "检查清理范围" }));
    await userEvent.click(await screen.findByRole("button", { name: "清理所选范围" }));
    expect(screen.getByTestId("confirm-modal-confirm")).toBeDisabled();
    expect(screen.getByText("将永久删除预览中的数据，包括会话、账号、记忆、恢复记录及遥测配置。请确认已关闭 Claude 及其集成。")).toBeInTheDocument();
    await userEvent.click(screen.getByTestId("confirm-modal-cancel"));
    expect(calls.map((call) => call.command)).toEqual(["claude-data:preview"]);
  });
  it("sends the confirmed base and retains partial progress from an unsuccessful envelope", async () => {
    const preview = makeClaudeDataPreview();
    const partial = { ...makeClaudeDataResult(), status: "partial" as const, deleted_files: 7, message: "A new file appeared; request a new preview." };
    const { calls } = renderWithEngine(<ClaudeDataPage />, {
      "claude-data:preview": makeEnvelope(preview),
      "claude-data:clear": makeEnvelope(partial, { ok: false, errors: [{ code: "PARTIAL", element: null, message: partial.message }] }),
    });
    await userEvent.click(screen.getByRole("button", { name: "检查清理范围" }));
    await userEvent.click(await screen.findByRole("button", { name: "清理所选范围" }));
    await userEvent.type(screen.getByRole("textbox"), "DELETE");
    await userEvent.click(screen.getByTestId("confirm-modal-confirm"));
    expect(await screen.findByRole("heading", { name: "清理未全部完成" })).toBeInTheDocument();
    expect(screen.getByText("已移除 7 个文件、3 个目录和 2 项系统凭据。")).toBeInTheDocument();
    expect(calls.find((call) => call.command === "claude-data:clear")?.args).toEqual(["--base", preview.base, "--confirm"]);
    expect(screen.queryByRole("button", { name: "清理所选范围" })).toBeNull();
  });
  it("requires another preview after an execution refusal", async () => {
    const preview = makeClaudeDataPreview();
    const { calls } = renderWithEngine(<ClaudeDataPage />, {
      "claude-data:preview": makeEnvelope(preview),
      "claude-data:clear": makeEnvelope({}, { ok: false, errors: [{ code: "STALE", element: null, message: "Preview is stale" }] }),
    });
    await userEvent.click(screen.getByRole("button", { name: "检查清理范围" }));
    await userEvent.click(await screen.findByRole("button", { name: "清理所选范围" }));
    await userEvent.type(screen.getByRole("textbox"), "DELETE");
    await userEvent.click(screen.getByTestId("confirm-modal-confirm"));
    expect(await screen.findByRole("alert")).toHaveTextContent("Preview is stale");
    expect(screen.queryByRole("button", { name: "清理所选范围" })).toBeNull();
    expect(calls.filter((call) => call.command === "claude-data:clear")).toHaveLength(1);
  });
  it("disables cleanup for a blocked preview", async () => {
    const { calls } = renderWithEngine(<ClaudeDataPage />, { "claude-data:preview": makeEnvelope({ ...makeClaudeDataPreview(), can_clear: false, blockers: ["Close Claude first"] }) });
    await userEvent.click(screen.getByRole("button", { name: "检查清理范围" }));
    expect(await screen.findByRole("button", { name: "清理所选范围" })).toBeDisabled();
    expect(screen.getByRole("alert")).toHaveTextContent("Close Claude first");
    expect(calls).toHaveLength(1);
  });
  it("clears a prior preview when credential scope changes", async () => {
    const { calls } = renderWithEngine(<ClaudeDataPage />, { "claude-data:preview": async (args) => makeEnvelope({ ...makeClaudeDataPreview(), files_only: args.includes("--files-only") }) });
    await userEvent.click(screen.getByRole("button", { name: "检查清理范围" }));
    expect(await screen.findByRole("button", { name: "清理所选范围" })).toBeEnabled();
    await userEvent.click(screen.getByRole("checkbox"));
    expect(screen.queryByRole("button", { name: "清理所选范围" })).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "检查清理范围" }));
    expect(calls[1].args).toEqual(["--files-only"]);
  });
  it("refuses a complete result without independent scope readback", () => {
    expect(claudeDataResultSchema.safeParse({ ...makeClaudeDataResult(), scope_verified: false }).success).toBe(false);
    expect(claudeDataResultSchema.safeParse({ ...makeClaudeDataResult(), telemetry_policy: "preserve" }).success).toBe(false);
    expect(claudeDataResultSchema.safeParse({ ...makeClaudeDataResult(), retained_verified: false }).success).toBe(false);
  });
  it("shows protected projects and does not describe unknown content as cleaned", async () => {
    const retained = [
      { path: "/YOUR_HOME/.claude/worktrees", reason: "project" as const },
      { path: "/YOUR_HOME/.claude/personal-source", reason: "unclassified" as const },
    ];
    renderWithEngine(<ClaudeDataPage />, { "claude-data:preview": makeEnvelope({ ...makeClaudeDataPreview(), retained, has_changes: false, can_clear: false }) });
    await userEvent.click(screen.getByRole("button", { name: "检查清理范围" }));
    expect(await screen.findByText(retained[0].path)).toBeInTheDocument();
    expect(screen.getByText(retained[1].path)).toBeInTheDocument();
    expect(screen.getByText("仍有归属未确认的内容，请先核对保留路径。")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "清理所选范围" })).toBeDisabled();
    expect(claudeDataResultSchema.safeParse({ ...makeClaudeDataResult(), retained }).success).toBe(false);
  });
});
