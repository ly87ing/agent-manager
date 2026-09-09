import { describe, expect, it } from "vitest";
import McpPage from "../pages/McpPage";
import { makeApplyResult, makeAuditData, makeElement, makeEnvelope, makeFreshness, makePlanAction } from "./fixtures";
import { renderWithEngine, screen, userEvent } from "./render-with-engine";
import type { Envelope } from "../lib/contract";

/**
 * MCP 库级启停链路：服务卡片按钮（按 observed.actions 提供）→ mcp:plan（--only/--action，
 * 无 agent）→ 预览确认 → mcp:apply 同参。旧「Switch → policy.set 预写 enabled」链路已按
 * 新契约移除：启停不再有任何 policy:set 预写，动作完全由 plan/apply 的 --action 表达。
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
                "claude-code": { state: "managed-ok", target: "/tmp/demo/.claude.json", target_exists: true, assigned: true, enabled: true },
              },
              env: {},
              actions: ["enable", "disable", "remove"],
            },
            state: "managed-ok",
            freshness: makeFreshness({ state: "current", current: "1.8.0", upstream: "1.8.0", checked_at: "2026-09-06T00:00:00Z", probe: "npm-view", cache_age_sec: 60 }),
          }),
        ],
      },
      { command: "mcp.status" },
    ),
    "mcp:plan": makeEnvelope(
      {
        domain: "mcp",
        actions: [makePlanAction({ op: "write-file", element: "mcp:chrome-devtools", target: "/tmp/demo.json", risk: "write-local", note: "撤下助手入口" })],
        summary: { read: 0, "write-local": 1, "exec-local": 0, network: 0, "install-remove": 0 },
      },
      { command: "mcp.plan" },
    ),
    "mcp:apply": makeEnvelope(makeApplyResult({ domain: "mcp" }), { command: "mcp.apply" }),
  };
}

describe("MCP 库级启停", () => {
  it("已启用服务显示「停用」→ mcp:plan --action disable（无 agent）→ 确认 → mcp:apply 同参，全程零 policy:set", async () => {
    const { calls } = renderWithEngine(<McpPage />, routes());

    // 已启用：只提供「停用」，不渲染「启用」
    expect(await screen.findByTestId("disable-server-mcp:chrome-devtools")).toBeInTheDocument();
    expect(screen.queryByTestId("enable-server-mcp:chrome-devtools")).toBeNull();
    await userEvent.click(screen.getByTestId("disable-server-mcp:chrome-devtools"));

    // plan 预览打开（限定本服务、无 agent），零 policy:set 预写
    const dialog = await screen.findByRole("dialog", { name: /将要执行的操作/ });
    expect(dialog).toHaveTextContent("停用服务 chrome-devtools");
    expect(dialog).toHaveTextContent("撤下已配置的助手入口");
    expect(dialog).toHaveTextContent("/tmp/demo.json");
    const planCall = calls.find((call) => call.command === "mcp:plan");
    expect(planCall!.args).toEqual(["--only", "mcp:chrome-devtools", "--action", "disable"]);
    expect(calls.some((call) => call.command === "policy:set")).toBe(false);

    await userEvent.click(screen.getByTestId("plan-confirm-button"));
    const applyCall = calls.find((call) => call.command === "mcp:apply");
    expect(applyCall).toBeDefined();
    expect(applyCall!.args).toEqual(["--only", "mcp:chrome-devtools", "--action", "disable"]);
    expect(applyCall!.options?.runId).toMatch(/^gui-/);

    const order = ["mcp:plan", "mcp:apply"].map((cmd) => calls.findIndex((call) => call.command === cmd));
    expect(order[0]).toBeLessThan(order[1]);
  });

  it("plan 被引擎拒绝时预览内展示错误且确认不可用，零写入", async () => {
    const rejected = routes();
    rejected["mcp:plan"] = makeEnvelope(
      {},
      {
        command: "mcp.plan",
        ok: false,
        errors: [{ code: "PLAN_FAILED", element: null, message: "引擎拒绝生成计划" }],
      },
    );
    const { calls } = renderWithEngine(<McpPage />, rejected);

    await userEvent.click(await screen.findByTestId("disable-server-mcp:chrome-devtools"));
    const dialog = await screen.findByRole("dialog", { name: /将要执行的操作/ });
    expect(dialog).toHaveTextContent("引擎拒绝生成计划");
    expect(screen.getByTestId("plan-confirm-button")).toBeDisabled();
    expect(calls.some((call) => call.command === "mcp:apply")).toBe(false);
    expect(calls.some((call) => call.command === "policy:set")).toBe(false);
  });
});
