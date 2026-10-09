import { describe, expect, it, onTestFinished, vi } from "vitest";
import { waitFor } from "@testing-library/react";
import { TrayPanel } from "../components/TrayPanel";
import type { Envelope } from "../lib/contract";
import { agentManagementFixture, makeAliasStatus, makeElement, makeEnvelope } from "./fixtures";
import { renderWithEngine, screen, userEvent } from "./render-with-engine";

function routes(): Record<string, Envelope> {
  const soon = new Date(Date.now() + 40 * 60_000).toISOString();
  return {
    "agents:status": makeEnvelope({
      domain: "agents",
      elements: [makeElement({
        id: "agents:claude-code",
        domain: "agents",
        spec: { display_name: "Claude Code", bin: "claude" },
        observed: { management: agentManagementFixture("claude") },
      })],
    }, { command: "agents.status" }),
    "profiles:status": makeEnvelope({
      domain: "profiles",
      elements: ["work", "relay"].map((id) => makeElement({
        id: `profiles:${id}`,
        domain: "profiles",
        spec: { display_name: id === "work" ? "工作账号" : "中转", base_agent: "claude-code", kind: "claude", config_dir: `~/.claude-${id}`, base_url: null, model: null },
        observed: {
          config_dir: `~/.claude-${id}`,
          blocked_reason: null,
          ...(id === "work"
            ? { usage: { source: "statusline", checked_at: new Date().toISOString(), windows: [{ window_minutes: 300, used_percent: 38, resets_at: soon }] } }
            : {}),
        },
        state: "managed-ok",
      })),
    }, { command: "profiles.status" }),
    "aliases:status": makeEnvelope(makeAliasStatus({ cw: 'CLAUDE_CONFIG_DIR="$HOME/.claude-work" claude' }), { command: "aliases.status" }),
  };
}

describe("托盘面板", () => {
  it("按助手列出账号、启动命令与额度倒计时；复制只写剪贴板，不调用任何写命令", async () => {
    const writeText = vi.fn(async () => undefined);
    const original = navigator.clipboard;
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText } });
    onTestFinished(() => { Object.defineProperty(navigator, "clipboard", { configurable: true, value: original }); });

    const { calls } = renderWithEngine(<TrayPanel />, routes());
    const work = await screen.findByTestId("tray-panel-row-profiles:work");
    expect(work).toHaveTextContent("工作账号");
    expect(work).toHaveTextContent("cw");
    expect(work).toHaveTextContent("38%");
    expect(work).toHaveTextContent("40 分钟后重置");
    expect(screen.getByTestId("tray-panel-row-profiles:relay")).toHaveTextContent("没有启动命令");
    expect(screen.getByTestId("tray-panel-copy-profiles:relay")).toBeDisabled();

    await userEvent.click(screen.getByTestId("tray-panel-copy-profiles:work"));
    await waitFor(() => expect(writeText).toHaveBeenCalledWith("cw"));
    expect(screen.getByTestId("tray-panel-copy-profiles:work")).toHaveTextContent("已复制");
    expect(calls.every((call) => call.command.endsWith(":status"))).toBe(true);
  });
});
