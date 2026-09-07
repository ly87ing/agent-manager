import { describe, expect, it } from "vitest";
import { fireEvent } from "@testing-library/react";
import McpPage from "../pages/McpPage";
import { makeApplyResult, makeElement, makeEnvelope, makeFreshness, makePlanAction, makePolicyData } from "./fixtures";
import { renderWithEngine, screen, userEvent, within } from "./render-with-engine";
import type { Envelope } from "../lib/contract";

/**
 * MCP 定义覆盖层：新增（两键写入）、编辑基线（覆盖片段）、移除（overlay=policy:delete /
 * 基线=tombstone {"_delete": true}）。
 */

const CHECKED = "2026-09-06T00:00:00Z";

function mcpElement(id: string, state: string): ReturnType<typeof makeElement> {
  return makeElement({
    id,
    domain: "mcp",
    spec: { command: "npx", args: ["-y", `${id.replace("mcp:", "")}-pkg`], startup_timeout_sec: 120, agents: null, env: {} },
    policy: { enabled: true, agents: null },
    observed: {
      agents: { "claude-code": { state: "managed-ok", target: "/tmp/demo/.claude.json", target_exists: true } },
      env: {},
    },
    state: state as never,
    freshness: makeFreshness({ state: "current", current: "1.0.0", upstream: "1.0.0", checked_at: CHECKED, probe: "npm-view", cache_age_sec: 60 }),
  });
}

function routes(): Record<string, Envelope> {
  return {
    "mcp:status": makeEnvelope(
      { domain: "mcp", elements: [mcpElement("mcp:chrome-devtools", "managed-ok"), mcpElement("mcp:my-proxy", "managed-ok")] },
      { command: "mcp.status" },
    ),
    // my-proxy 定义在 local 覆盖层；chrome-devtools 只在基线
    "policy:get": makeEnvelope(
      makePolicyData({
        layers: {
          baseline: { mcp: { "chrome-devtools": { enabled: true } } },
          synced: null,
          local: { mcp: { definitions: { "my-proxy": { command: "npx", args: ["-y", "my-proxy-pkg"] } }, "my-proxy": { enabled: true } } },
        },
      }),
      { command: "policy.get" },
    ),
    "policy:set": makeEnvelope(makePolicyData({ layers: null, effective: null, attribution: null, layer: "local", key: "mcp.definitions.x", value: null, path: "/repo/config/manager.local.json" }), { command: "policy.set" }),
    "policy:delete": makeEnvelope(makePolicyData({ layers: null, effective: null, attribution: null, layer: "local", key: "mcp.definitions.my-proxy", value: null, path: "/repo/config/manager.local.json" }), { command: "policy.delete" }),
    "mcp:plan": makeEnvelope(
      {
        domain: "mcp",
        actions: [
          makePlanAction({ op: "write-file", element: "mcp:my-proxy", target: "/tmp/demo/.claude.json", risk: "write-local", note: "注入 my-proxy" }),
          makePlanAction({ op: "write-file", element: "mcp:demo-srv", target: "/tmp/demo/.claude.json", risk: "write-local", note: "注入 demo-srv" }),
        ],
        summary: { read: 0, "write-local": 2, "exec-local": 0, network: 0, "install-remove": 0 },
      },
      { command: "mcp.plan" },
    ),
    "mcp:apply": makeEnvelope(makeApplyResult({ domain: "mcp" }), { command: "mcp.apply" }),
  };
}

describe("MCP 服务定义覆盖层", () => {
  it("新增服务：policy.set 写两键（definitions 片段 + enabled true）→ 预览 → apply", async () => {
    const { calls } = renderWithEngine(<McpPage />, routes());

    await userEvent.click(await screen.findByTestId("add-server-button"));
    const dialog = await screen.findByRole("dialog");
    fireEvent.change(within(dialog).getByTestId("server-name-input"), { target: { value: "demo-srv" } });
    fireEvent.change(within(dialog).getByTestId("server-command-input"), { target: { value: "npx" } });
    fireEvent.change(within(dialog).getByTestId("server-args-input"), { target: { value: "-y\ndemo-srv-mcp" } });
    fireEvent.change(within(dialog).getByTestId("server-env-input"), { target: { value: "DEMO_TOKEN" } });
    await userEvent.click(within(dialog).getByTestId("server-agent-claude-code"));
    fireEvent.change(within(dialog).getByTestId("server-timeout-input"), { target: { value: "90" } });
    await userEvent.click(within(dialog).getByTestId("server-form-submit"));

    const setCalls = calls.filter((call) => call.command === "policy:set");
    expect(setCalls).toHaveLength(2);
    expect(setCalls[0].args[1]).toBe("local");
    expect(setCalls[0].args[3]).toBe("mcp.definitions.demo-srv");
    const definition = JSON.parse(setCalls[0].args[5]);
    expect(definition).toEqual({
      command: "npx",
      args: ["-y", "demo-srv-mcp"],
      startup_timeout_sec: 90,
      agents: ["claude-code"],
      env: { DEMO_TOKEN: "${DEMO_TOKEN}" },
    });
    expect(setCalls[1].args[3]).toBe("mcp.demo-srv.enabled");
    expect(setCalls[1].args[5]).toBe("true");

    // 预览（过滤到 mcp:demo-srv）→ 确认 → mcp:apply
    await screen.findByRole("dialog", { name: /将要执行的操作/ });
    await userEvent.click(screen.getByTestId("plan-confirm-button"));
    expect(calls.some((call) => call.command === "mcp:apply")).toBe(true);
  });

  it("编辑基线条目：写覆盖片段（含改动字段），表单注明仓库默认值不受影响", async () => {
    const { calls } = renderWithEngine(<McpPage />, routes());
    await userEvent.click(await screen.findByText("mcp:chrome-devtools"));
    await userEvent.click(await screen.findByTestId("edit-server-mcp:chrome-devtools"));

    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByTestId("baseline-overlay-note")).toHaveTextContent("仓库默认值不受影响");
    // 名称不可改
    expect(within(dialog).getByTestId("server-name-input")).toBeDisabled();
    fireEvent.change(within(dialog).getByTestId("server-timeout-input"), { target: { value: "99" } });
    await userEvent.click(within(dialog).getByTestId("server-form-submit"));

    const setCalls = calls.filter((call) => call.command === "policy:set");
    // 编辑只写定义片段，不动 enabled 键
    expect(setCalls).toHaveLength(1);
    expect(setCalls[0].args[3]).toBe("mcp.definitions.chrome-devtools");
    expect(JSON.parse(setCalls[0].args[5]).startup_timeout_sec).toBe(99);
  });

  it("移除覆盖层条目 → policy:delete local 层；移除基线条目 → tombstone", async () => {
    const { calls } = renderWithEngine(<McpPage />, routes());

    // overlay：my-proxy 定义在 local 层
    await userEvent.click(await screen.findByText("mcp:my-proxy"));
    await userEvent.click(await screen.findByTestId("remove-server-mcp:my-proxy"));
    let dialog = await screen.findByRole("dialog");
    expect(dialog).toHaveTextContent("会直接删除这段定义");
    fireEvent.change(screen.getByTestId("confirm-modal-input"), { target: { value: "my-proxy" } });
    await userEvent.click(screen.getByTestId("confirm-modal-confirm"));
    const deleteCall = calls.find((call) => call.command === "policy:delete");
    expect(deleteCall!.args).toEqual(["--layer", "local", "--key", "mcp.definitions.my-proxy"]);

    // 移除后会打开「将要执行的操作」预览——关掉它再进行下一次移除
    await screen.findByRole("dialog", { name: /将要执行的操作/ });
    await userEvent.click(screen.getByText("取消"));

    // 基线：chrome-devtools 只在基线 → tombstone
    await userEvent.click(await screen.findByText("mcp:chrome-devtools"));
    await userEvent.click(await screen.findByTestId("remove-server-mcp:chrome-devtools"));
    dialog = await screen.findByRole("dialog");
    expect(dialog).toHaveTextContent("移除标记");
    expect(dialog).toHaveTextContent("删除该标记即可恢复");
    fireEvent.change(screen.getByTestId("confirm-modal-input"), { target: { value: "chrome-devtools" } });
    await userEvent.click(screen.getByTestId("confirm-modal-confirm"));
    const tombstone = calls.filter((call) => call.command === "policy:set").pop();
    expect(tombstone!.args[3]).toBe("mcp.definitions.chrome-devtools");
    expect(JSON.parse(tombstone!.args[5])).toEqual({ _delete: true });
  });
});
