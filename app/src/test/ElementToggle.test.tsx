import { describe, expect, it } from "vitest";
import AgentsPage from "../pages/AgentsPage";
import { makeApplyResult, makeAuditData, makeElement, makeEnvelope, makeFreshness, makePlanAction, makePolicyData } from "./fixtures";
import { renderWithEngine, screen, userEvent } from "./render-with-engine";
import type { Envelope } from "../lib/contract";

/** 元素启停开关链路：Switch → policy.set enabled 键 → toast「立即同步」→ plan 预览 → apply。 */
function routes(): Record<string, Envelope> {
  return {
    "system:audit": makeEnvelope(makeAuditData(), { command: "system.audit" }),
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
            freshness: makeFreshness({ state: "current", current: "2.1.3", upstream: "2.1.3", checked_at: "2026-09-06T00:00:00Z", probe: "npm-view", cache_age_sec: 60 }),
          }),
        ],
      },
      { command: "agents.status" },
    ),
    "policy:set": makeEnvelope(makePolicyData({ layers: null, effective: null, attribution: null, layer: "local", key: "agents.claude-code.enabled", value: false, path: "/repo/config/manager.local.json" }), { command: "policy.set" }),
    "agents:plan": makeEnvelope(
      {
        domain: "agents",
        actions: [makePlanAction({ op: "write-file", element: "agents:claude-code", target: "/tmp/demo.json", risk: "write-local", note: "停用标记" })],
        summary: { read: 0, "write-local": 1, "exec-local": 0, network: 0, "install-remove": 0 },
      },
      { command: "agents.plan" },
    ),
    "agents:apply": makeEnvelope(makeApplyResult({ domain: "agents" }), { command: "agents.apply" }),
  };
}

describe("元素启停开关", () => {
  it("关 → policy.set enabled=false（默认层）→ toast 带「立即同步」→ 预览 → agents:apply", async () => {
    const { calls } = renderWithEngine(<AgentsPage />, routes());

    const toggle = await screen.findByTestId("toggle-agents:claude-code");
    expect(toggle).toHaveAttribute("aria-checked", "true");
    await userEvent.click(toggle);

    // policy.set 参数正确（默认仅这台电脑层）
    const setCall = calls.find((call) => call.command === "policy:set");
    expect(setCall!.args).toEqual(["--layer", "local", "--key", "agents.claude-code.enabled", "--value", "false"]);

    // toast 出现且带「立即同步」动作
    const toastAction = await screen.findByText("立即同步");
    await userEvent.click(toastAction);

    // plan 预览打开（过滤到本元素），确认后触发 apply
    const dialog = await screen.findByRole("dialog");
    expect(dialog).toHaveTextContent("停用 Claude Code");
    expect(dialog).toHaveTextContent("/tmp/demo.json");
    await userEvent.click(screen.getByTestId("plan-confirm-button"));

    const applyCall = calls.find((call) => call.command === "agents:apply");
    expect(applyCall).toBeDefined();
    expect(applyCall!.options?.runId).toMatch(/^gui-/);

    const order = ["policy:set", "agents:plan", "agents:apply"].map((cmd) => calls.findIndex((call) => call.command === cmd));
    expect(order[0]).toBeLessThan(order[1]);
    expect(order[1]).toBeLessThan(order[2]);
  });

  it("policy.set 被拒绝时不出现「立即同步」动作", async () => {
    const rejected = routes();
    rejected["policy:set"] = makeEnvelope({}, {
      command: "policy.set",
      ok: false,
      errors: [{ code: "POLICY_SECRET_GATE", element: null, message: "疑似密钥，拒绝写入" }],
    });
    const { calls } = renderWithEngine(<AgentsPage />, rejected);
    await userEvent.click(await screen.findByTestId("toggle-agents:claude-code"));
    expect(await screen.findByText(/保存被拒绝/)).toBeInTheDocument();
    expect(screen.queryByText("立即同步")).toBeNull();
    expect(calls.some((call) => call.command === "agents:apply")).toBe(false);
  });
});
