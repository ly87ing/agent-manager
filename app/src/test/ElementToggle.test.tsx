import { describe, expect, it } from "vitest";
import McpPage from "../pages/McpPage";
import { makeApplyResult, makeAuditData, makeElement, makeEnvelope, makeFreshness, makePlanAction, makePolicyData } from "./fixtures";
import { renderWithEngine, screen, userEvent } from "./render-with-engine";
import type { Envelope } from "../lib/contract";

/**
 * 元素启停开关链路：Switch → policy.set enabled 键 → toast「立即同步」→ plan 预览 → apply。
 * Agents 页的普通 enabled 快捷开关已按设计移除（其实会触发卸载语义），
 * 本链路改经仍保留开关的 MCP 页验证；agents 安装/卸载仍由明确动作按钮覆盖。
 */
function routes(): Record<string, Envelope> {
  return {
    "system:audit": makeEnvelope(makeAuditData(), { command: "system.audit" }),
    "mcp:status": makeEnvelope(
      {
        domain: "mcp",
        elements: [
          makeElement({
            id: "mcp:chrome-devtools",
            domain: "mcp",
            spec: { command: "npx", args: ["-y", "chrome-devtools-mcp"], startup_timeout_sec: 120, agents: null, env: {} },
            policy: { enabled: true },
            observed: {
              agents: {
                "claude-code": { state: "managed-ok", target: "/tmp/demo/.claude.json", target_exists: true },
              },
              env: {},
            },
            state: "managed-ok",
            freshness: makeFreshness({ state: "current", current: "1.8.0", upstream: "1.8.0", checked_at: "2026-09-06T00:00:00Z", probe: "npm-view", cache_age_sec: 60 }),
          }),
        ],
      },
      { command: "mcp.status" },
    ),
    "policy:set": makeEnvelope(makePolicyData({ layers: null, effective: null, attribution: null, layer: "local", key: "mcp.chrome-devtools.enabled", value: false, path: "/repo/config/manager.local.json" }), { command: "policy.set" }),
    "mcp:plan": makeEnvelope(
      {
        domain: "mcp",
        actions: [makePlanAction({ op: "write-file", element: "mcp:chrome-devtools", target: "/tmp/demo.json", risk: "write-local", note: "停用标记" })],
        summary: { read: 0, "write-local": 1, "exec-local": 0, network: 0, "install-remove": 0 },
      },
      { command: "mcp.plan" },
    ),
    "mcp:apply": makeEnvelope(makeApplyResult({ domain: "mcp" }), { command: "mcp.apply" }),
  };
}

describe("元素启停开关", () => {
  it("关 → policy.set enabled=false（默认层）→ toast 带「立即同步」→ 预览 → mcp:apply", async () => {
    const { calls } = renderWithEngine(<McpPage />, routes());

    const toggle = await screen.findByTestId("toggle-mcp:chrome-devtools");
    expect(toggle).toHaveAttribute("aria-checked", "true");
    await userEvent.click(toggle);

    // policy.set 参数正确（默认仅这台电脑层）
    const setCall = calls.find((call) => call.command === "policy:set");
    expect(setCall!.args).toEqual(["--layer", "local", "--key", "mcp.chrome-devtools.enabled", "--value", "false"]);

    // toast 出现且带「立即同步」动作
    const toastAction = await screen.findByText("立即同步");
    await userEvent.click(toastAction);

    // plan 预览打开（过滤到本元素），确认后触发 apply
    const dialog = await screen.findByRole("dialog");
    expect(dialog).toHaveTextContent("停用 mcp:chrome-devtools");
    expect(dialog).toHaveTextContent("/tmp/demo.json");
    await userEvent.click(screen.getByTestId("plan-confirm-button"));

    const applyCall = calls.find((call) => call.command === "mcp:apply");
    expect(applyCall).toBeDefined();
    expect(applyCall!.args).toEqual(["--only", "mcp:chrome-devtools"]);
    expect(applyCall!.options?.runId).toMatch(/^gui-/);

    const order = ["policy:set", "mcp:plan", "mcp:apply"].map((cmd) => calls.findIndex((call) => call.command === cmd));
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
    const { calls } = renderWithEngine(<McpPage />, rejected);
    await userEvent.click(await screen.findByTestId("toggle-mcp:chrome-devtools"));
    expect(await screen.findByText(/保存被拒绝/)).toBeInTheDocument();
    expect(screen.queryByText("立即同步")).toBeNull();
    expect(calls.some((call) => call.command === "mcp:apply")).toBe(false);
  });
});
