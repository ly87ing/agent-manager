import { describe, expect, it, vi } from "vitest";
import { waitFor } from "@testing-library/react";
import { toast } from "sonner";
import McpPage from "../pages/McpPage";
import { makeApplyResult, makeAuditData, makeElement, makeEnvelope, makeFreshness } from "./fixtures";
import { renderWithEngine, screen, userEvent } from "./render-with-engine";
import type { Envelope } from "../lib/contract";

/**
 * MCP 库级启停链路：服务卡片按钮（按 observed.actions 提供）→ 直接 mcp:apply（--only/--action，
 * 无 agent），不再先取 plan、不开预览弹窗。启停没有任何 policy:set 预写，
 * 动作完全由 apply 的 --action 表达。
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
    "mcp:apply": makeEnvelope(makeApplyResult({ domain: "mcp" }), { command: "mcp.apply" }),
  };
}

describe("MCP 库级启停", () => {
  it("已停用服务：「启用」点击直接 mcp:apply --action enable（带 run_id），不取 plan、不开弹窗、零 policy:set", async () => {
    const disabledRoutes = routes();
    disabledRoutes["mcp:status"] = makeEnvelope(
      {
        domain: "mcp",
        elements: [
          makeElement({
            id: "mcp:chrome-devtools",
            domain: "mcp",
            spec: { command: "npx", args: ["chrome-devtools-mcp"], agents: null, env: {} },
            policy: { enabled: false },
            observed: {
              agents: {
                "claude-code": {
                  state: "agent-disabled",
                  target: "/tmp/demo.json",
                  target_exists: true,
                  assigned: true,
                  enabled: false,
                },
              },
              env: {},
              actions: ["enable", "remove"],
            },
            state: "disabled",
            freshness: makeFreshness(),
          }),
        ],
      },
      { command: "mcp.status" },
    );
    const { calls } = renderWithEngine(<McpPage />, disabledRoutes);

    await userEvent.click(await screen.findByTestId("enable-server-mcp:chrome-devtools"));
    await waitFor(() => expect(calls.some((call) => call.command === "mcp:apply")).toBe(true));

    const applyCalls = calls.filter((call) => call.command === "mcp:apply");
    expect(applyCalls).toHaveLength(1);
    expect(applyCalls[0].args).toEqual(["--only", "mcp:chrome-devtools", "--action", "enable"]);
    expect(applyCalls[0].args).not.toContain("--dry-run");
    expect(applyCalls[0].options?.runId).toMatch(/^gui-/);
    expect(calls.some((call) => call.command.endsWith(":plan"))).toBe(false);
    expect(calls.some((call) => call.command === "policy:set")).toBe(false);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("已启用服务只提供「停用」：点击直接 mcp:apply --action disable（带 run_id），零 policy:set", async () => {
    const { calls } = renderWithEngine(<McpPage />, routes());

    // 已启用：只提供「停用」，不渲染「启用」
    expect(await screen.findByTestId("disable-server-mcp:chrome-devtools")).toBeInTheDocument();
    expect(screen.queryByTestId("enable-server-mcp:chrome-devtools")).toBeNull();
    await userEvent.click(screen.getByTestId("disable-server-mcp:chrome-devtools"));

    await waitFor(() => expect(calls.some((call) => call.command === "mcp:apply")).toBe(true));
    const applyCalls = calls.filter((call) => call.command === "mcp:apply");
    expect(applyCalls).toHaveLength(1);
    expect(applyCalls[0].args).toEqual(["--only", "mcp:chrome-devtools", "--action", "disable"]);
    expect(applyCalls[0].args).not.toContain("--dry-run");
    expect(applyCalls[0].options?.runId).toMatch(/^gui-/);
    expect(calls.some((call) => call.command.endsWith(":plan"))).toBe(false);
    expect(calls.some((call) => call.command === "policy:set")).toBe(false);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("apply 被引擎拒绝：既有执行 toast 报告失败，不重试、不开弹窗", async () => {
    const rejected = routes();
    rejected["mcp:apply"] = makeEnvelope(
      {},
      {
        command: "mcp.apply",
        ok: false,
        errors: [{ code: "APPLY_FAILED", element: null, message: "目标被外部修改，拒绝覆盖" }],
      },
    );
    const toastErrorSpy = vi.spyOn(toast, "error");
    const { calls } = renderWithEngine(<McpPage />, rejected);

    await userEvent.click(await screen.findByTestId("disable-server-mcp:chrome-devtools"));

    await waitFor(() => expect(toastErrorSpy).toHaveBeenCalled());
    expect(toastErrorSpy).toHaveBeenCalledWith(expect.stringContaining("目标被外部修改，拒绝覆盖"));
    const applyCalls = calls.filter((call) => call.command === "mcp:apply");
    expect(applyCalls).toHaveLength(1);
    expect(calls.some((call) => call.command.endsWith(":plan"))).toBe(false);
    expect(calls.some((call) => call.command === "policy:set")).toBe(false);
    expect(screen.queryByRole("dialog")).toBeNull();
  });
});
