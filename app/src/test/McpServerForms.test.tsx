import { describe, expect, it } from "vitest";
import { fireEvent, waitFor } from "@testing-library/react";
import McpPage from "../pages/McpPage";
import { makeApplyResult, makeElement, makeEnvelope, makeFreshness, makePlanAction, makePolicyData } from "./fixtures";
import { renderWithEngine, screen, userEvent, within } from "./render-with-engine";
import type { Envelope } from "../lib/contract";

/**
 * MCP 定义覆盖层：新增/编辑只保存整条定义（_replace，不写 enabled、不 plan/apply）、
 * 取消零写入、移除（overlay=policy:delete / 基线=tombstone {"_delete": true}）。
 */

const CHECKED = "2026-09-06T00:00:00Z";

function mcpElement(id: string, state: string): ReturnType<typeof makeElement> {
  return makeElement({
    id,
    domain: "mcp",
    spec: {
      command: "npx",
      args: ["-y", `${id.replace("mcp:", "")}-pkg`],
      startup_timeout_sec: 120,
      agents: null,
      // spec.env 是脱敏视图：${REF} 占位引用原样保留，普通字面值被掩码为 <literal>
      env: { GROK_API_KEY: "${SECRETS_GROK_API_KEY}", PLAIN_TOKEN: "<literal>" },
      // 表单未展示的定义字段，整条替换后必须保留
      extra_args_by_agent: { "claude-code": ["--verbose"] },
    },
    policy: { enabled: true, agents: null },
    observed: {
      agents: { "claude-code": { state: "managed-ok", target: "/tmp/demo/.claude.json", target_exists: true, assigned: true, enabled: true } },
      env: {},
      actions: ["enable", "disable", "remove"],
    },
    state: state as never,
    freshness: makeFreshness({ state: "current", current: "1.0.0", upstream: "1.0.0", checked_at: CHECKED, probe: "npm-view", cache_age_sec: 60 }),
  });
}

/** sqlite 有效定义：完整定义，env 为真实值（含合成的非 secret 字面值），含未知字段 custom_flag。 */
function fullDefinitionOf(name: string): Record<string, unknown> {
  return {
    command: "npx",
    args: ["-y", `${name}-pkg`],
    startup_timeout_sec: 120,
    agents: null,
    env: { GROK_API_KEY: "${SECRETS_GROK_API_KEY}", PLAIN_TOKEN: "synthetic-plain-value" },
    extra_args_by_agent: { "claude-code": ["--verbose"] },
    custom_flag: "keep-me",
  };
}

function routes(): Record<string, Envelope> {
  return {
    "mcp:status": makeEnvelope(
      { domain: "mcp", elements: [mcpElement("mcp:chrome-devtools", "managed-ok"), mcpElement("mcp:my-proxy", "managed-ok")] },
      { command: "mcp.status" },
    ),
    // 定义来源统一由 SQLite 投影提供；缺失时显示 builtin。
    "policy:get": makeEnvelope(
      makePolicyData({
        layers: {
          sqlite: { mcp: { definitions: { "chrome-devtools": fullDefinitionOf("chrome-devtools"), "my-proxy": fullDefinitionOf("my-proxy") } } },
        },
      }),
      { command: "policy.get" },
    ),
    "policy:set": makeEnvelope(makePolicyData({ layers: null, effective: null, attribution: null, layer: "sqlite", key: "mcp.definitions.x", value: null, path: "/native-manager.db" }), { command: "policy.set" }),
    "policy:delete": makeEnvelope(makePolicyData({ layers: null, effective: null, attribution: null, layer: "sqlite", key: "mcp.definitions.my-proxy", value: null, path: "/native-manager.db" }), { command: "policy.delete" }),
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
  it("新增服务：只保存整条定义到资源库（_replace），不写 enabled、不 plan/apply", async () => {
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
    expect(setCalls).toHaveLength(1);
    expect(setCalls[0].args[1]).toBe("local");
    expect(setCalls[0].args[3]).toBe("mcp.definitions.demo-srv");
    const definition = JSON.parse(setCalls[0].args[5]);
    expect(definition).toEqual({
      _delete: false,
      _replace: true,
      command: "npx",
      args: ["-y", "demo-srv-mcp"],
      startup_timeout_sec: 90,
      agents: ["claude-code"],
      env: { DEMO_TOKEN: "${DEMO_TOKEN}" },
    });

    // 零额外写入：无 enabled 键，无预览/同步
    expect(calls.some((call) => call.command === "policy:set" && call.args[3] === "mcp.demo-srv.enabled")).toBe(false);
    expect(calls.some((call) => call.command === "mcp:plan")).toBe(false);
    expect(calls.some((call) => call.command === "mcp:apply")).toBe(false);
    expect(screen.queryByRole("dialog", { name: /将要执行的操作/ })).toBeNull();
  });

  it("编辑 sqlite 条目：以完整定义整条替换，未展示字段与 env 真实值保留", async () => {
    const { calls } = renderWithEngine(<McpPage />, routes());
    await userEvent.click(await screen.findByText("mcp:chrome-devtools"));
    await userEvent.click(await screen.findByTestId("edit-server-mcp:chrome-devtools"));

    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).queryByTestId("builtin-overlay-note")).toBeNull();
    // 名称不可改
    expect(within(dialog).getByTestId("server-name-input")).toBeDisabled();
    // env 初始列出完整定义的全部变量名
    expect(within(dialog).getByTestId("server-env-input")).toHaveValue("GROK_API_KEY\nPLAIN_TOKEN");
    fireEvent.change(within(dialog).getByTestId("server-timeout-input"), { target: { value: "99" } });
    // 清空参数：整条 replacement 语义下保存后不应从基线复活
    fireEvent.change(within(dialog).getByTestId("server-args-input"), { target: { value: "" } });
    const callsBeforeSubmit = calls.length;
    await userEvent.click(within(dialog).getByTestId("server-form-submit"));

    const submitCalls = calls.slice(callsBeforeSubmit);
    const setCalls = submitCalls.filter((call) => call.command === "policy:set");
    // 编辑只写定义片段，不动 enabled 键；完整定义已在，无需迁移
    expect(setCalls).toHaveLength(1);
    expect(submitCalls.some((call) => call.command === "db:migrate")).toBe(false);
    expect(setCalls[0].args[3]).toBe("mcp.definitions.chrome-devtools");
    const definition = JSON.parse(setCalls[0].args[5]);
    expect(definition.startup_timeout_sec).toBe(99);
    expect(definition._replace).toBe(true);
    expect(definition.args).toEqual([]);
    // 反例断言：未展示字段保留；${REF} 引用原样；spec 中被掩码为 <literal> 的字面值
    // 用完整定义真实值重建，不允许 <literal> 落盘
    expect(definition.extra_args_by_agent).toEqual({ "claude-code": ["--verbose"] });
    expect(definition.custom_flag).toBe("keep-me");
    expect(definition.env).toEqual({ GROK_API_KEY: "${SECRETS_GROK_API_KEY}", PLAIN_TOKEN: "synthetic-plain-value" });
    expect(JSON.stringify(definition)).not.toContain("<literal>");
    // 保存不触发定向预览/同步（DriftExplanationPanel 的无参只读 plan 查询不算保存动作）
    expect(submitCalls.some((call) => call.command === "mcp:plan" && call.args.includes("--only"))).toBe(false);
    expect(submitCalls.some((call) => call.command === "mcp:apply")).toBe(false);
    expect(screen.queryByRole("dialog", { name: /将要执行的操作/ })).toBeNull();
  });

  it("编辑删除 env 变量：保存后该键不复活", async () => {
    const { calls } = renderWithEngine(<McpPage />, routes());
    await userEvent.click(await screen.findByText("mcp:my-proxy"));
    await userEvent.click(await screen.findByTestId("edit-server-mcp:my-proxy"));

    const dialog = await screen.findByRole("dialog");
    // 表单初始带出完整定义的两个变量名，清空后提交
    expect(within(dialog).getByTestId("server-env-input")).toHaveValue("GROK_API_KEY\nPLAIN_TOKEN");
    fireEvent.change(within(dialog).getByTestId("server-env-input"), { target: { value: "" } });
    await userEvent.click(within(dialog).getByTestId("server-form-submit"));

    const setCalls = calls.filter((call) => call.command === "policy:set");
    expect(setCalls).toHaveLength(1);
    const definition = JSON.parse(setCalls[0].args[5]);
    expect(definition._replace).toBe(true);
    expect(definition.env).toEqual({});
    expect(definition.extra_args_by_agent).toEqual({ "claude-code": ["--verbose"] });
    expect(definition.custom_flag).toBe("keep-me");
  });

  /** 无 DB 旧存储：policy:get 在 db:migrate 前无 sqlite 层，迁移后返回完整有效定义。 */
  function legacyRoutes(): Record<string, Envelope | ((args: string[]) => Promise<Envelope>)> {
    let migrated = false;
    const base = routes();
    return {
      ...base,
      "db:migrate": async () => {
        migrated = true;
        return makeEnvelope({ migrated: true }, { command: "db.migrate" });
      },
      "policy:get": async () =>
        makeEnvelope(
          makePolicyData({
            layers: migrated
              ? {
                  sqlite: { mcp: { definitions: { "chrome-devtools": fullDefinitionOf("chrome-devtools") } } },
                }
              : {
                  sqlite: null,
                },
          }),
          { command: "policy.get" },
        ),
    };
  }

  it("首次无 DB 编辑：先 db:migrate 再取完整定义整条替换，未知字段与 env 原值保留", async () => {
    const { calls } = renderWithEngine(<McpPage />, legacyRoutes());
    await userEvent.click(await screen.findByText("mcp:chrome-devtools"));
    await userEvent.click(await screen.findByTestId("edit-server-mcp:chrome-devtools"));

    const dialog = await screen.findByRole("dialog");
    fireEvent.change(within(dialog).getByTestId("server-timeout-input"), { target: { value: "77" } });
    await userEvent.click(within(dialog).getByTestId("server-form-submit"));

    await waitFor(() => {
      expect(calls.some((call) => call.command === "policy:set")).toBe(true);
    });
    const migrateIndex = calls.findIndex((call) => call.command === "db:migrate");
    const setCalls = calls.filter((call) => call.command === "policy:set");
    expect(migrateIndex).toBeGreaterThanOrEqual(0);
    expect(setCalls).toHaveLength(1);
    expect(calls.indexOf(setCalls[0])).toBeGreaterThan(migrateIndex);

    const definition = JSON.parse(setCalls[0].args[5]);
    expect(definition._replace).toBe(true);
    expect(definition.startup_timeout_sec).toBe(77);
    // 未知字段与表单未展示字段从迁移后的完整定义保留
    expect(definition.custom_flag).toBe("keep-me");
    expect(definition.extra_args_by_agent).toEqual({ "claude-code": ["--verbose"] });
    // env：占位引用原样、被掩码字面值重建为完整定义真实值
    expect(definition.env).toEqual({ GROK_API_KEY: "${SECRETS_GROK_API_KEY}", PLAIN_TOKEN: "synthetic-plain-value" });
    expect(JSON.stringify(definition)).not.toContain("<literal>");
    expect(calls.some((call) => call.command === "mcp:apply")).toBe(false);
  });

  it("首次无 DB 编辑删除 env 变量：迁移合并后该键不复活", async () => {
    const { calls } = renderWithEngine(<McpPage />, legacyRoutes());
    await userEvent.click(await screen.findByText("mcp:chrome-devtools"));
    await userEvent.click(await screen.findByTestId("edit-server-mcp:chrome-devtools"));

    const dialog = await screen.findByRole("dialog");
    fireEvent.change(within(dialog).getByTestId("server-env-input"), { target: { value: "" } });
    await userEvent.click(within(dialog).getByTestId("server-form-submit"));

    await waitFor(() => {
      expect(calls.some((call) => call.command === "policy:set")).toBe(true);
    });
    const setCalls = calls.filter((call) => call.command === "policy:set");
    expect(setCalls).toHaveLength(1);
    const definition = JSON.parse(setCalls[0].args[5]);
    expect(definition._replace).toBe(true);
    expect(definition.env).toEqual({});
    expect(definition.custom_flag).toBe("keep-me");
  });

  it("新增/编辑表单点取消：零写入", async () => {
    const { calls } = renderWithEngine(<McpPage />, routes());

    // 新增 → 取消
    await userEvent.click(await screen.findByTestId("add-server-button"));
    let dialog = await screen.findByRole("dialog");
    fireEvent.change(within(dialog).getByTestId("server-name-input"), { target: { value: "cancelled-srv" } });
    fireEvent.change(within(dialog).getByTestId("server-command-input"), { target: { value: "npx" } });
    await userEvent.click(within(dialog).getByRole("button", { name: "取消" }));
    expect(screen.queryByRole("dialog")).toBeNull();

    // 编辑 → 取消
    await userEvent.click(await screen.findByText("mcp:my-proxy"));
    await userEvent.click(await screen.findByTestId("edit-server-mcp:my-proxy"));
    dialog = await screen.findByRole("dialog");
    fireEvent.change(within(dialog).getByTestId("server-timeout-input"), { target: { value: "42" } });
    await userEvent.click(within(dialog).getByRole("button", { name: "取消" }));
    expect(screen.queryByRole("dialog")).toBeNull();

    expect(calls.filter((call) => call.command === "policy:set")).toHaveLength(0);
  });

  it("移除服务：单级预览确认（无前置确认弹窗、无墓碑写入），plan/apply 携带 --only mcp:<name> --action remove", async () => {
    const { calls } = renderWithEngine(<McpPage />, routes());

    // overlay 条目：my-proxy 定义在 local 层——同一流程，不再区分覆盖层/基线写墓碑
    await userEvent.click(await screen.findByTestId("remove-server-mcp:my-proxy"));
    let dialog = await screen.findByRole("dialog", { name: /将要执行的操作/ });
    expect(screen.queryByTestId("confirm-modal-input")).toBeNull();
    expect(dialog).toHaveTextContent("清理 1 条受影响的助手配置");
    let planCall = calls.find((call) => call.command === "mcp:plan");
    expect(planCall!.args).toEqual(["--only", "mcp:my-proxy", "--action", "remove"]);
    await userEvent.click(screen.getByTestId("plan-confirm-button"));
    const applyCall = calls.find((call) => call.command === "mcp:apply");
    expect(applyCall).toBeDefined();
    expect(applyCall!.args).toEqual(["--only", "mcp:my-proxy", "--action", "remove"]);

    // 基线条目：chrome-devtools 只在基线——同样只走 plan 预览；取消零写
    await userEvent.click(await screen.findByTestId("remove-server-mcp:chrome-devtools"));
    dialog = await screen.findByRole("dialog", { name: /将要执行的操作/ });
    planCall = calls.filter((call) => call.command === "mcp:plan").pop();
    expect(planCall!.args).toEqual(["--only", "mcp:chrome-devtools", "--action", "remove"]);
    await userEvent.click(screen.getByRole("button", { name: "取消" }));

    // 全程无 policy:set 墓碑/覆盖写入
    expect(calls.some((call) => call.command === "policy:set")).toBe(false);
    expect(calls.filter((call) => call.command === "mcp:apply")).toHaveLength(1);
  });
});
