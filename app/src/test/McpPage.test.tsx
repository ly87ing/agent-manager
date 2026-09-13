import { describe, expect, it } from "vitest";
import McpPage from "../pages/McpPage";
import type { StatusData } from "../lib/contract";
import { makeApplyResult, makeElement, makeEnvelope, makeFreshness, makePlanAction } from "./fixtures";
import { renderWithEngine, screen, userEvent } from "./render-with-engine";

const LONG_VERSION = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

function mcpRoutes(): Record<string, ReturnType<typeof makeEnvelope>> {
  const status: StatusData = {
    domain: "mcp",
    elements: [
      makeElement({
        id: "mcp:chrome-devtools",
        domain: "mcp",
        spec: { command: "npx", args: ["-y", "chrome-devtools-mcp"], startup_timeout_sec: 120, agents: null, env: {} },
        observed: {
          agents: {
            "claude-code": {
              state: "drifted",
              target: "/tmp/demo/.claude.json",
              target_exists: true,
              assigned: true,
              enabled: true,
              version: LONG_VERSION,
              available_version: LONG_VERSION,
              actions: ["update"],
            },
            codex: { state: "managed-ok", target: "/tmp/demo/.codex/config.toml", target_exists: true, assigned: true, enabled: true, actions: ["disable", "remove"] },
          },
          env: {},
          actions: ["enable", "disable", "remove"],
        },
        state: "drifted",
        freshness: makeFreshness({ state: "behind", current: "1.2.0", upstream: "1.8.0", checked_at: "2026-09-05T10:00:00Z", probe: "npm-view", cache_age_sec: 60 }),
      }),
      makeElement({
        id: "mcp:serena",
        domain: "mcp",
        spec: {
          command: "uvx",
          args: ["--from", "git+https://example.com/serena@main", "serena"],
          extra_args_by_agent: { codex: ["--context", "codex"] },
          startup_timeout_sec: 60,
          agents: null,
          env: { SERENA_KEY: "${SERENA_KEY}" },
        },
        observed: {
          agents: {
            "claude-code": { state: "leftover", target: "/tmp/demo/.claude.json", target_exists: true, assigned: false, enabled: false, actions: ["remove"] },
            codex: { state: "not-materialized", target: "/tmp/demo/.codex/config.toml", target_exists: false, assigned: true, enabled: true, actions: [] },
          },
          env: { SERENA_KEY: { placeholder: "SERENA_KEY", resolved: false } },
          actions: ["disable", "remove"],
        },
        state: "drifted",
        freshness: makeFreshness({ state: "current", current: "a1b2c3d", upstream: "a1b2c3d", checked_at: "2026-09-05T10:00:00Z", probe: "git-ls-remote", cache_age_sec: 30 }),
      }),
    ],
  };
  return {
    "mcp:status": makeEnvelope(status, { command: "mcp.status" }),
    "mcp:plan": makeEnvelope(
      {
        domain: "mcp",
        actions: [makePlanAction({ op: "write-file", element: "mcp:chrome-devtools", target: "/tmp/demo/.claude.json", risk: "write-local", note: "更新 claude-code 配置" })],
        summary: { read: 0, "write-local": 1, "exec-local": 0, network: 0, "install-remove": 0 },
      },
      { command: "mcp.plan" },
    ),
    "mcp:apply": makeEnvelope(makeApplyResult({ domain: "mcp" }), { command: "mcp.apply" }),
  };
}

describe("McpPage 服务库 + 助手中的服务", () => {
  it("服务详情优先选择 update，并在同动作中按 Agent key 升序选择", async () => {
    const priorityRoutes = mcpRoutes();
    priorityRoutes["mcp:status"] = makeEnvelope(
      {
        domain: "mcp",
        elements: [
          makeElement({
            id: "mcp:priority",
            domain: "mcp",
            spec: { command: "npx", args: ["priority-mcp"], agents: null, env: {} },
            policy: { enabled: true },
            observed: {
              agents: {
                "zeta-agent": { state: "drifted", target: "/tmp/zeta.json", target_exists: true, actions: ["update"] },
                "codex": { state: "missing", target: "/tmp/codex.json", target_exists: false, actions: ["add"] },
                "alpha-agent": { state: "drifted", target: "/tmp/alpha.json", target_exists: true, actions: ["update"] },
              },
              env: {},
              actions: [],
            },
            state: "drifted",
            freshness: makeFreshness(),
          }),
        ],
      },
      { command: "mcp.status" },
    );
    const { calls } = renderWithEngine(<McpPage />, priorityRoutes);

    await userEvent.click(await screen.findByRole("button", { name: "mcp:priority" }));
    await screen.findByTestId("mcp-detail-mcp:priority");

    expect(calls.filter((call) => call.command === "mcp:plan")).toHaveLength(1);
    expect(calls.find((call) => call.command === "mcp:plan")?.args).toEqual([
      "--only",
      "mcp:priority",
      "--agent",
      "alpha-agent",
      "--action",
      "update",
    ]);
  });

  it("默认服务库：卡片渲染与 freshness；点击服务名展开详情（命令/密钥状态/附加参数/写入位置，不含密钥值）", async () => {
    const { calls } = renderWithEngine(<McpPage />, mcpRoutes());

    // 默认进入服务库
    expect(await screen.findByRole("tab", { name: "服务库" })).toHaveAttribute("aria-selected", "true");
    expect(screen.getByText(/可更新 1\.2\.0 → 1\.8\.0/)).toBeInTheDocument();
    // 凭据只显示状态统计
    expect(screen.getByText("密钥变量 0/1 已配置")).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "mcp:serena" }));
    const detail = await screen.findByTestId("mcp-detail-mcp:serena");
    expect(detail).toHaveTextContent("uvx --from git+https://example.com/serena@main serena");
    expect(detail).toHaveTextContent("启动超时 60 秒");
    expect(detail).toHaveTextContent("--context codex");
    // env 占位符：变量名 + 未解析标注，永不出现值
    expect(detail).toHaveTextContent("SERENA_KEY");
    expect(detail).toHaveTextContent("未配置");
    expect(detail).toHaveTextContent("/tmp/demo/.codex/config.toml");
    // 只有 update/enable/add 才能自动预览；serena 只有 remove，不发起 plan。
    expect(calls.some((call) => call.command === "mcp:plan")).toBe(false);
  });

  it("服务详情按 update→enable→add 与稳定助手名选择动作，并携带精确参数", async () => {
    const { calls } = renderWithEngine(<McpPage />, mcpRoutes());
    await userEvent.click(await screen.findByRole("button", { name: "mcp:chrome-devtools" }));
    await screen.findByText("更新 claude-code 配置");

    expect(calls.find((call) => call.command === "mcp:plan")?.args).toEqual([
      "--only",
      "mcp:chrome-devtools",
      "--agent",
      "claude-code",
      "--action",
      "update",
    ]);
  });

  it("助手中的服务：单元格态 已配置/不一致/残留/尚未生成，版本短摘要 title 存完整值", async () => {
    renderWithEngine(<McpPage />, mcpRoutes());
    await userEvent.click(await screen.findByRole("tab", { name: "助手中的服务" }));

    expect(await screen.findByTitle("codex: managed-ok")).toHaveTextContent("已配置");
    const drifted = screen.getByTitle("claude-code: drifted");
    expect(drifted).toHaveTextContent("不一致");
    expect(drifted.className).toContain("amber");
    expect(screen.getByTitle("claude-code: leftover")).toHaveTextContent("残留");
    expect(screen.getByTitle("codex: not-materialized")).toHaveTextContent("尚未生成");

    // 64 位摘要只显示短形，完整值在 title（资源库版本与当前配置版本两列）
    const versionCells = screen.getAllByTitle(LONG_VERSION);
    expect(versionCells).toHaveLength(2);
    expect(versionCells[0]).toHaveTextContent("0123456");
    expect(versionCells[0]).not.toHaveTextContent(LONG_VERSION);

    // 助手筛选收窄行
    const agentSelect = screen.getByTestId("filter-agent-select") as HTMLSelectElement;
    await userEvent.selectOptions(agentSelect, "codex");
    expect(screen.queryByTitle("claude-code: drifted")).toBeNull();
    expect(screen.getByTitle("codex: managed-ok")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: /清除筛选/ }));
    expect(screen.getByTitle("claude-code: drifted")).toBeInTheDocument();
  });

  it("行操作：按 cell.actions 渲染，「更新」→ mcp:plan 精确 --only/--agent/--action → 确认 apply，全程零 policy:set", async () => {
    const { calls } = renderWithEngine(<McpPage />, mcpRoutes());
    await userEvent.click(await screen.findByRole("tab", { name: "助手中的服务" }));

    // actions 之外的按钮不出现（更新与启用不混）
    expect(await screen.findByTestId("action-update-mcp:chrome-devtools-claude-code")).toBeInTheDocument();
    expect(screen.queryByTestId("action-enable-mcp:chrome-devtools-claude-code")).toBeNull();
    // actions 为空的 cell 不推测动作
    expect(screen.getAllByText("当前引擎未提供操作").length).toBeGreaterThan(0);

    await userEvent.click(screen.getByTestId("action-update-mcp:chrome-devtools-claude-code"));
    const dialog = await screen.findByRole("dialog", { name: /将要执行的操作/ });
    expect(dialog).toHaveTextContent("更新 mcp:chrome-devtools @ claude-code");
    const planCall = calls.find((call) => call.command === "mcp:plan");
    expect(planCall!.args).toEqual(["--only", "mcp:chrome-devtools", "--agent", "claude-code", "--action", "update"]);
    expect(calls.some((call) => call.command === "policy:set")).toBe(false);

    await userEvent.click(screen.getByTestId("plan-confirm-button"));
    const applyCall = calls.find((call) => call.command === "mcp:apply");
    expect(applyCall).toBeDefined();
    expect(applyCall!.args).toEqual(["--only", "mcp:chrome-devtools", "--agent", "claude-code", "--action", "update"]);
    expect(applyCall!.options?.runId).toMatch(/^gui-/);
  });

  it("库级删除：单个预览内确认（无前置弹窗），说明显示受影响配置数与保留语义", async () => {
    const { calls } = renderWithEngine(<McpPage />, mcpRoutes());
    await userEvent.click(await screen.findByTestId("remove-server-mcp:chrome-devtools"));

    const dialog = await screen.findByRole("dialog", { name: /将要执行的操作/ });
    expect(screen.queryByTestId("confirm-modal-input")).toBeNull();
    // 仅 assigned=true 计入：chrome-devtools 两条均已分配
    expect(dialog).toHaveTextContent("清理 2 条受影响的助手配置");
    expect(dialog).toHaveTextContent(/不会卸载已安装的软件包/);
    expect(calls.find((call) => call.command === "mcp:plan")!.args).toEqual(["--only", "mcp:chrome-devtools", "--action", "remove"]);

    // 取消零写
    await userEvent.click(screen.getByRole("button", { name: "取消" }));
    expect(calls.some((call) => call.command === "mcp:apply")).toBe(false);
    expect(calls.some((call) => call.command === "policy:set")).toBe(false);
  });

  it("mcp:status 返回 ok:false 且 data:{} 时显示 DOMAIN_NOT_IMPLEMENTED 错误结构，不白屏", async () => {
    const errorRoutes = {
      "mcp:status": makeEnvelope(
        {} as unknown as StatusData,
        {
          command: "mcp.status",
          ok: false,
          errors: [{ code: "DOMAIN_NOT_IMPLEMENTED", element: null, message: "MCP 域尚未实现" }],
        },
      ),
    };
    renderWithEngine(<McpPage />, errorRoutes);

    expect(await screen.findByRole("heading", { name: "MCP" })).toBeInTheDocument();
    expect(screen.getByText("DOMAIN_NOT_IMPLEMENTED")).toBeInTheDocument();
    expect(screen.getByText("MCP 域尚未实现")).toBeInTheDocument();
    expect(screen.queryByRole("tab", { name: "服务库" })).toBeNull();
  });
});

describe("McpPage 显式接管通道（--allow-takeover）", () => {
  const CONFLICT_MESSAGE = "目标文件 /tmp/demo/.claude.json 中已存在非受管同名条目 'take-svc'，拒绝覆盖以防止冲突";

  function takeoverRoutes(): Record<string, ReturnType<typeof makeEnvelope> | ((args: string[]) => Promise<ReturnType<typeof makeEnvelope>>)> {
    const status: StatusData = {
      domain: "mcp",
      elements: [
        makeElement({
          id: "mcp:take-svc",
          domain: "mcp",
          spec: { command: "npx", args: ["-y", "take-mcp"], agents: null, env: {} },
          observed: {
            agents: {
              // 冲突 cell：drifted + 非受管同名条目 blocked_reason + 无动作（导入后落地断点场景）
              "claude-code": { state: "drifted", target: "/tmp/demo/.claude.json", target_exists: true, assigned: false, enabled: false, actions: [], blocked_reason: "存在外部非受管同名条目，拒绝操作以防止冲突" },
              codex: { state: "missing", target: "/tmp/demo/.codex/config.toml", target_exists: true, assigned: false, enabled: false, actions: ["add"] },
            },
            env: {},
            actions: ["disable", "remove"],
          },
          state: "drifted",
        }),
      ],
    };
    return {
      "mcp:status": makeEnvelope(status, { command: "mcp.status" }),
      "mcp:plan": async (args: string[]) => {
        if (!args.includes("--allow-takeover")) {
          return makeEnvelope(
            {},
            { command: "mcp.plan", ok: false, errors: [{ code: "PREFLIGHT_FAILED", element: null, message: CONFLICT_MESSAGE }] },
          );
        }
        return makeEnvelope(
          {
            domain: "mcp",
            actions: [
              makePlanAction({
                op: "write-file",
                element: "mcp:take-svc",
                target: "/tmp/demo/.claude.json",
                risk: "write-local",
                note: "add mcp:take-svc 到 claude-code（接管：将替换目标文件中的未受管同名条目）",
                takeover: true,
              }),
            ],
            summary: { read: 0, "write-local": 1, "exec-local": 0, network: 0, "install-remove": 0 },
          },
          { command: "mcp.plan" },
        );
      },
      "mcp:apply": makeEnvelope(makeApplyResult({ domain: "mcp" }), { command: "mcp.apply" }),
    };
  }

  it("plan 遇「非受管同名条目」→ 错误区「接管原条目并重试」→ 重发 plan/apply 带 --allow-takeover 且预览可见接管标注", async () => {
    const { calls } = renderWithEngine(<McpPage />, takeoverRoutes());
    await userEvent.click(await screen.findByRole("tab", { name: "助手中的服务" }));

    // codex cell 提供正常「添加」；plan 首次因冲突 fail-closed
    await userEvent.click(await screen.findByTestId("action-add-mcp:take-svc-codex"));
    const dialog = await screen.findByRole("dialog", { name: /将要执行的操作/ });
    expect(dialog).toHaveTextContent(CONFLICT_MESSAGE);
    const firstPlan = calls.find((call) => call.command === "mcp:plan");
    expect(firstPlan!.args).toEqual(["--only", "mcp:take-svc", "--agent", "codex", "--action", "add"]);
    expect(firstPlan!.args).not.toContain("--allow-takeover");

    // 错误区提供接管重试
    await userEvent.click(await screen.findByTestId("takeover-retry-button"));

    // 以相同 --only/--action 追加 --allow-takeover 重跑 plan
    const planCalls = calls.filter((call) => call.command === "mcp:plan");
    const retryPlan = planCalls[planCalls.length - 1];
    expect(retryPlan.args).toEqual(["--only", "mcp:take-svc", "--agent", "codex", "--action", "add", "--allow-takeover"]);

    // 预览可见 takeover 标注（徽标 + 说明文案）
    expect(await screen.findByTestId("takeover-badge")).toHaveTextContent("接管");
    expect(dialog).toHaveTextContent(/接管：将替换目标文件中的未受管同名条目/);

    // 确认后 apply 同样带 flag
    await userEvent.click(screen.getByTestId("plan-confirm-button"));
    const applyCall = calls.find((call) => call.command === "mcp:apply");
    expect(applyCall!.args).toEqual(["--only", "mcp:take-svc", "--agent", "codex", "--action", "add", "--allow-takeover"]);
  });

  it("冲突 cell 行内「接管原条目」直接以 --allow-takeover 打开预览，确认后 apply 带 flag", async () => {
    const { calls } = renderWithEngine(<McpPage />, takeoverRoutes());
    await userEvent.click(await screen.findByRole("tab", { name: "助手中的服务" }));

    // 冲突 cell：无常规动作，提供显式接管入口（assigned=false → add）
    expect(screen.queryByTestId("action-add-mcp:take-svc-claude-code")).toBeNull();
    await userEvent.click(await screen.findByTestId("takeover-mcp:take-svc-claude-code"));

    const dialog = await screen.findByRole("dialog", { name: /将要执行的操作/ });
    expect(dialog).toHaveTextContent("接管原条目 mcp:take-svc @ claude-code");
    const planCall = calls.find((call) => call.command === "mcp:plan");
    expect(planCall!.args).toEqual(["--only", "mcp:take-svc", "--agent", "claude-code", "--action", "add", "--allow-takeover"]);
    expect(await screen.findByTestId("takeover-badge")).toBeInTheDocument();

    await userEvent.click(screen.getByTestId("plan-confirm-button"));
    const applyCall = calls.find((call) => call.command === "mcp:apply");
    expect(applyCall!.args).toEqual(["--only", "mcp:take-svc", "--agent", "claude-code", "--action", "add", "--allow-takeover"]);
  });

  it("非冲突错误不展示接管重试按钮", async () => {
    const routes = takeoverRoutes();
    routes["mcp:plan"] = async () =>
      makeEnvelope(
        {},
        { command: "mcp.plan", ok: false, errors: [{ code: "PREFLIGHT_FAILED", element: null, message: "凭证变量未配置: SOME_TOKEN" }] },
      );
    renderWithEngine(<McpPage />, routes);
    await userEvent.click(await screen.findByRole("tab", { name: "助手中的服务" }));
    await userEvent.click(await screen.findByTestId("action-add-mcp:take-svc-codex"));
    const dialog = await screen.findByRole("dialog", { name: /将要执行的操作/ });
    expect(dialog).toHaveTextContent("凭证变量未配置");
    expect(screen.queryByTestId("takeover-retry-button")).toBeNull();
  });
});
