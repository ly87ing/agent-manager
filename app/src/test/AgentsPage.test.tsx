import { describe, expect, it } from "vitest";
import { waitFor } from "@testing-library/react";
import AgentsPage from "../pages/AgentsPage";
import { makeAuditData, makeElement, makeEnvelope, makeFreshness, makeMode, makePlanAction } from "./fixtures";
import { renderWithEngine, screen, userEvent } from "./render-with-engine";
import type { Envelope } from "../lib/contract";

function routes(offline: boolean): Record<string, Envelope> {
  return {
    "system:audit": makeEnvelope(makeAuditData(), { command: "system.audit", mode: makeMode({ offline }) }),
    "agents:status": makeEnvelope(
      {
        domain: "agents",
        elements: [
          makeElement({
            id: "agents:claude-code",
            domain: "agents",
            spec: { display_name: "Claude Code", bin: "claude", preferred_provider: "npm", providers: { npm: { managed: true } } },
            policy: { enabled: true, auto_install: true, auto_update: true, preferred_provider: "npm" },
            observed: { providers: ["npm"], classified: { managed: ["npm"], deprecated: [], system_only: [], unsupported: [] }, installed: true, preferred_active: true },
            state: "managed-ok",
            freshness: makeFreshness({ state: "behind", current: "2.0.0", upstream: "2.1.3", checked_at: "2026-09-06T00:00:00Z", probe: "npm-view", cache_age_sec: 60 }),
            actions: [
              { id: "upgrade", risk: "network", needs_lock: true, offline_ok: false },
              { id: "uninstall", risk: "install-remove", needs_lock: true, offline_ok: false },
            ],
          }),
        ],
      },
      { command: "agents.status" },
    ),
    "agents:plan": makeEnvelope(
      {
        domain: "agents",
        actions: [
          makePlanAction({ op: "run-cmd", element: "agents:claude-code", target: null, risk: "network", cmd: "npm install -g @anthropic-ai/claude-code", note: "升级到 2.1.3" }),
        ],
        summary: { read: 0, "write-local": 0, "exec-local": 0, network: 1, "install-remove": 0 },
      },
      { command: "agents.plan" },
    ),
  };
}

describe("Agents 页写动作", () => {
  it("升级按钮 → plan 预览（过滤到本元素）→ network 知情勾选 → agents:apply 带 run_id", async () => {
    const { calls } = renderWithEngine(<AgentsPage />, routes(false));
    await userEvent.click(await screen.findByTestId("action-upgrade-agents:claude-code"));

    const dialog = await screen.findByRole("dialog");
    expect(dialog).toHaveTextContent("升级 Claude Code");
    expect(dialog).toHaveTextContent("npm install -g @anthropic-ai/claude-code");
    // network 级：须勾选
    const confirm = screen.getByTestId("plan-confirm-button");
    expect(confirm).toBeDisabled();
    await userEvent.click(screen.getByTestId("network-ack-checkbox"));
    expect(confirm).toBeEnabled();
    await userEvent.click(confirm);

    const applyCall = calls.find((call) => call.command === "agents:apply");
    expect(applyCall).toBeDefined();
    expect(applyCall!.args).toEqual(["--only", "agents:claude-code"]);
    expect(applyCall!.options?.runId).toMatch(/^gui-/);
  });

  it("offline：network/install-remove 按钮全部禁用", async () => {
    renderWithEngine(<AgentsPage />, routes(true));
    // offline 来自 system:audit 查询，待其就位后按钮转为禁用
    const upgrade = await screen.findByTestId("action-upgrade-agents:claude-code");
    await waitFor(() => expect(upgrade).toBeDisabled());
    expect(screen.getByTestId("action-uninstall-agents:claude-code")).toBeDisabled();
  });
});
