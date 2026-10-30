import { describe, expect, it } from "vitest";
import { ClaudeConnections } from "../pages/ClaudeConnections";
import { connectionInspectionSchema, connectionResultSchema } from "../lib/claude-connections";
import { makeEnvelope } from "./fixtures";
import { renderWithEngine, screen, userEvent } from "./render-with-engine";

export const connectionFixture = () => ({ scope: "claude_connections", root: "/YOUR_HOME/.claude", sources: [
  { id: "settings", path: "/YOUR_HOME/.claude/settings.json", status: "inspected", keys: ["ANTHROPIC_API_KEY"], items: [{ id: "settings:ANTHROPIC_API_KEY", key: "ANTHROPIC_API_KEY", line: null, editable: true }], kind: "settings" },
  { id: "caller", path: null, status: "inspected", keys: ["HTTP_PROXY"], items: [], kind: "environment" },
  { id: ".zshrc", path: "/YOUR_HOME/.zshrc", status: "unavailable", keys: [], items: [], kind: "references" },
], cc_switch: "not_running", custom_config_present: false, clear_supported: false });
const previewFixture = () => ({ scope: "claude_connections", base: "a".repeat(64), item: "settings:ANTHROPIC_API_KEY", path: "/YOUR_HOME/.claude/settings.json", key: "ANTHROPIC_API_KEY", kind: "settings", line: null, root: null, shared: false, can_clear: true, blockers: [] });

describe("Claude connection inspection", () => {
  it("inspects only after an explicit click and exposes fields without values", async () => {
    const callsBusy: boolean[] = [];
    const { calls } = renderWithEngine(<ClaudeConnections disabled={false} onBusy={value => callsBusy.push(value)} />, {
      "claude-data:inspect-connections": makeEnvelope(connectionFixture()),
    });
    expect(calls).toHaveLength(0);
    await userEvent.click(screen.getByRole("button", { name: "检查连接配置" }));
    await screen.findByText("ANTHROPIC_API_KEY");
    expect(calls.map(call => call.command)).toEqual(["claude-data:inspect-connections"]);
    expect(callsBusy).toEqual([true, false]);
    expect(screen.getByText(/路径、权限、格式或内容/)).toBeVisible();
    expect(screen.getByText(/Codex 数据/)).toBeVisible();
  });
  it("retains unknown and redirected coverage instead of claiming clean", async () => {
    renderWithEngine(<ClaudeConnections disabled={false} onBusy={() => {}} />, {
      "claude-data:inspect-connections": makeEnvelope({ ...connectionFixture(), custom_config_present: true, cc_switch: "unknown" }),
    });
    await userEvent.click(screen.getByRole("button", { name: "检查连接配置" }));
    await screen.findByText(/当前进程设置了 CLAUDE_CONFIG_DIR/);
    expect(screen.getByText("无法确认 CC Switch 是否运行。")).toBeVisible();
  });
  it("rejects injected values and malformed source states", () => {
    const fixture = connectionFixture();
    expect(connectionInspectionSchema.safeParse(fixture).success).toBe(true);
    expect(connectionInspectionSchema.safeParse({ ...fixture, sources: [{ ...fixture.sources[0], value: "PRIVATE_SENTINEL" }] }).success).toBe(false);
    expect(connectionInspectionSchema.safeParse({ ...fixture, cc_switch: "clean" }).success).toBe(false);
    expect(connectionResultSchema.safeParse({ scope: "claude_connections", status: "complete", item: "settings:ANTHROPIC_API_KEY", removed: false, retained_verified: true, message: "" }).success).toBe(false);
  });
  it("requires a fresh one-field preview and explicit confirmation, with zero-write cancellation", async () => {
    const preview = previewFixture();
    const { calls } = renderWithEngine(<ClaudeConnections disabled={false} onBusy={() => {}} />, {
      "claude-data:inspect-connections": makeEnvelope({ ...connectionFixture(), clear_supported: true }),
      "claude-data:preview": makeEnvelope(preview),
      "claude-data:clear": makeEnvelope({ scope: "claude_connections", status: "complete", item: preview.item, removed: true, retained_verified: true, message: "Only the selected field was removed." }),
    });
    await userEvent.click(screen.getByRole("button", { name: "检查连接配置" }));
    await userEvent.click(await screen.findByRole("button", { name: "预览移除 ANTHROPIC_API_KEY" }));
    await screen.findByRole("dialog", { name: "移除选定连接字段" });
    expect(screen.getByRole("button", { name: "移除此字段" })).toBeDisabled();
    await userEvent.click(screen.getByTestId("confirm-modal-cancel"));
    expect(calls.filter(call => call.command === "claude-data:clear")).toHaveLength(0);
    await userEvent.click(screen.getByRole("button", { name: "预览移除 ANTHROPIC_API_KEY" }));
    await screen.findByRole("dialog", { name: "移除选定连接字段" });
    await userEvent.type(screen.getByTestId("confirm-modal-input"), "REMOVE");
    await userEvent.click(screen.getByRole("button", { name: "移除此字段" }));
    await screen.findByRole("heading", { name: "Claude 连接配置" });
    await screen.findAllByText("选定字段已移除");
    expect(calls.filter(call => call.command === "claude-data:preview")).toHaveLength(2);
    expect(calls.find(call => call.command === "claude-data:clear")?.args).toEqual(["--connection-items", '["settings:ANTHROPIC_API_KEY"]', "--base", preview.base, "--confirm"]);
    expect(calls.some(call => call.args.includes("--files-only"))).toBe(false);
  });
  it("keeps blocked previews unconfirmable even after typing the confirmation word", async () => {
    renderWithEngine(<ClaudeConnections disabled={false} onBusy={() => {}} />, {
      "claude-data:inspect-connections": makeEnvelope({ ...connectionFixture(), clear_supported: true }),
      "claude-data:preview": makeEnvelope({ ...previewFixture(), can_clear: false, blockers: ["Close Claude before cleanup."] }),
    });
    await userEvent.click(screen.getByRole("button", { name: "检查连接配置" }));
    await userEvent.click(await screen.findByRole("button", { name: "预览移除 ANTHROPIC_API_KEY" }));
    await screen.findByText("Close Claude before cleanup.");
    await userEvent.type(screen.getByTestId("confirm-modal-input"), "REMOVE");
    expect(screen.getByRole("button", { name: "移除此字段" })).toBeDisabled();
  });
  it("requires shared-source confirmation and forwards the explicitly selected custom root", async () => {
    const item = "shell:.zshrc:3:ANTHROPIC_API_KEY";
    const root = "/YOUR_CUSTOM_CLAUDE";
    const { calls } = renderWithEngine(<ClaudeConnections disabled={false} onBusy={() => {}} />, {
      "claude-data:inspect-connections": makeEnvelope({ ...connectionFixture(), root, clear_supported: true, sources: [{ id: ".zshrc", path: "/YOUR_HOME/.zshrc", status: "inspected", keys: ["ANTHROPIC_API_KEY", "HTTP_PROXY"], kind: "references", items: [
        { id: item, key: "ANTHROPIC_API_KEY", line: 3, editable: true },
        { id: "shell:.zshrc:4:HTTP_PROXY", key: "HTTP_PROXY", line: 4, editable: false },
      ] }] }),
      "claude-data:preview": makeEnvelope({ ...previewFixture(), item, root, kind: "startup", path: "/YOUR_HOME/.zshrc", line: 3, shared: true }),
      "claude-data:clear": makeEnvelope({ scope: "claude_connections", status: "complete", item, removed: true, retained_verified: true, message: "Verified." }),
    });
    await userEvent.type(screen.getByRole("textbox", { name: "自定义 Claude 目录" }), root);
    await userEvent.click(screen.getByRole("button", { name: "检查连接配置" }));
    await userEvent.click(await screen.findByRole("button", { name: "预览移除 ANTHROPIC_API_KEY" }));
    await screen.findByText(/这是共享来源/);
    expect(screen.getByRole("button", { name: "预览移除 HTTP_PROXY" })).toBeDisabled();
    await userEvent.type(screen.getByTestId("confirm-modal-input"), "REMOVE");
    expect(screen.getByRole("button", { name: "移除此字段" })).toBeDisabled();
    await userEvent.type(screen.getByTestId("confirm-modal-input"), " SHARED");
    await userEvent.click(screen.getByRole("button", { name: "移除此字段" }));
    await screen.findAllByText("选定字段已移除");
    expect(calls[0].args).toEqual(["--connection-root", root]);
    expect(calls.find(call => call.command === "claude-data:clear")?.args).toEqual(["--connection-items", JSON.stringify([item]), "--connection-root", root, "--base", "a".repeat(64), "--confirm"]);
  });
  it("invalidates inspection when the custom directory changes", async () => {
    const { calls } = renderWithEngine(<ClaudeConnections disabled={false} onBusy={() => {}} />, { "claude-data:inspect-connections": makeEnvelope(connectionFixture()) });
    await userEvent.click(screen.getByRole("button", { name: "检查连接配置" }));
    await screen.findByText("ANTHROPIC_API_KEY");
    await userEvent.type(screen.getByRole("textbox", { name: "自定义 Claude 目录" }), "/YOUR_CUSTOM_CLAUDE");
    expect(screen.queryByText("ANTHROPIC_API_KEY")).not.toBeInTheDocument();
    expect(calls).toHaveLength(1);
  });
});
