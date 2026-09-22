import { describe, expect, it, vi } from "vitest";
import { waitFor } from "@testing-library/react";
import { toast } from "sonner";
import McpPage from "../pages/McpPage";
import type { StatusData } from "../lib/contract";
import { makeApplyResult, makeElement, makeEnvelope, makeFreshness } from "./fixtures";
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
          actions: ["enable", "disable", "remove", "upgrade"],
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
            codex: {
              state: "not-materialized",
              target: "/tmp/demo/.codex/config.toml",
              target_exists: false,
              assigned: true,
              enabled: true,
              actions: [],
              blocked_reason: "凭证变量未配置: SERENA_KEY",
            },
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
    "mcp:apply": makeEnvelope(makeApplyResult({ domain: "mcp" }), { command: "mcp.apply" }),
  };
}

/** 矩阵行默认折叠，逐助手操作在展开后才出现；等行渲染出来再全部展开，幂等。 */
async function expandAllMcpRows() {
  await screen.findAllByTestId(/^expand-mcp:/);
  for (const toggle of screen.queryAllByTestId(/^expand-mcp:/)) {
    if (toggle.getAttribute("aria-expanded") !== "true") await userEvent.click(toggle);
  }
}

describe("McpPage 服务库 + 助手中的服务", () => {
  it("默认服务库：卡片渲染与 freshness；点击服务名展开详情（命令/密钥状态/附加参数/写入位置，不含密钥值）", async () => {
    const { calls } = renderWithEngine(<McpPage />, mcpRoutes());

    // 默认进入服务库
    expect(await screen.findByRole("tab", { name: "服务库" })).toHaveAttribute("aria-selected", "true");
    expect(screen.getByText(/可更新 1\.2\.0 → 1\.8\.0/)).toBeInTheDocument();
    // 凭据只显示状态统计
    expect(screen.getByText("密钥变量 0/1 已配置")).toBeInTheDocument();
    const chromeResolution = screen.getByTestId("mcp-resolution-mcp:chrome-devtools");
    expect(chromeResolution).toHaveTextContent("1 个助手待同步");
    expect(chromeResolution).toHaveTextContent("claude-code");
    expect(chromeResolution).toHaveTextContent("将按服务库定义同步到这个助手");
    expect(screen.getByTestId("sync-server-mcp:chrome-devtools")).toBeInTheDocument();
    expect(screen.queryByTestId("action-update-mcp:chrome-devtools-claude-code")).toBeNull();

    expect(screen.queryByTestId("mcp-detail-mcp:serena")).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "mcp:serena" }));
    const detail = await screen.findByTestId("mcp-detail-mcp:serena");
    expect(detail).toHaveTextContent("uvx");
    expect(detail).toHaveTextContent('"git+https://example.com/serena@main"');
    expect(detail).toHaveTextContent("启动超时 60 秒");
    expect(detail).toHaveTextContent("--context codex");
    // env 占位符：变量名 + 未解析标注，永不出现值
    expect(detail).toHaveTextContent("SERENA_KEY");
    expect(detail).toHaveTextContent("未配置");
    expect(detail).toHaveTextContent("/tmp/demo/.codex/config.toml");
    // 展开详情为纯状态展示：任何元素都不发起 plan
    expect(calls.some((call) => call.command.endsWith(":plan"))).toBe(false);
  });

  it("服务库直接闭环不一致：一键同步自动选择 update 并精确同步对应助手", async () => {
    const { calls } = renderWithEngine(<McpPage />, mcpRoutes());

    await userEvent.click(await screen.findByTestId("sync-server-mcp:chrome-devtools"));
    await waitFor(() => expect(calls.some((call) => call.command === "mcp:apply")).toBe(true));

    const apply = calls.find((call) => call.command === "mcp:apply")!;
    expect(apply.args).toEqual(["--only", "mcp:chrome-devtools", "--action", "update", "--agent", "claude-code"]);
    expect(apply.options?.runId).toMatch(/^gui-/);
    expect(calls.some((call) => call.command.endsWith(":plan"))).toBe(false);
    expect(calls.some((call) => call.command === "policy:set")).toBe(false);
  });

  it("全部纳管并同步：不受搜索结果影响，汇总全部服务的安全项与接管项后只确认一次", async () => {
    const { calls } = renderWithEngine(<McpPage />, mcpRoutes());

    await userEvent.type(await screen.findByTestId("mcp-search"), "serena");
    expect(screen.queryByTestId("mcp-card-mcp:chrome-devtools")).toBeNull();

    await userEvent.click(screen.getByTestId("sync-all-mcp"));
    const dialog = await screen.findByRole("dialog", { name: "全部纳管并同步" });
    expect(dialog).toHaveTextContent("将执行 2 项助手配置同步，涉及 2 个服务");
    expect(dialog).toHaveTextContent("1 项会用服务库定义接管同名条目");
    expect(screen.getAllByRole("dialog")).toHaveLength(1);

    await userEvent.click(screen.getByTestId("confirm-modal-confirm"));
    await waitFor(() => expect(calls.filter((call) => call.command === "mcp:apply")).toHaveLength(2));

    const applyCalls = calls.filter((call) => call.command === "mcp:apply");
    expect(applyCalls.map((call) => call.args)).toEqual([
      ["--only", "mcp:chrome-devtools", "--action", "update", "--agent", "claude-code"],
      ["--only", "mcp:serena", "--action", "add", "--agent", "claude-code", "--allow-takeover"],
    ]);
    expect(calls.some((call) => call.command.endsWith(":plan"))).toBe(false);
    expect(calls.some((call) => call.command === "policy:set")).toBe(false);
  });

  it("全部纳管并同步部分失败：Toast 只给可操作摘要，不塞入超长内部错误", async () => {
    const longError = "凭证变量未配置: NODE_REPL_INSTRUCTIONS_USE_CASE_BROWSER, NODE_REPL_INSTRUCTIONS_USE_CASE_CHROME";
    const routes: Parameters<typeof renderWithEngine>[1] = {
      ...mcpRoutes(),
      "mcp:apply": async (args: string[]) => args.includes("mcp:serena")
        ? makeEnvelope({}, {
            command: "mcp.apply",
            ok: false,
            errors: [{ code: "MCP_ERROR", element: "mcp:serena", message: longError }],
          })
        : makeEnvelope(makeApplyResult({ domain: "mcp" }), { command: "mcp.apply" }),
    };
    const toastErrorSpy = vi.spyOn(toast, "error");

    renderWithEngine(<McpPage />, routes);
    await userEvent.click(await screen.findByTestId("sync-all-mcp"));
    await userEvent.click(await screen.findByTestId("confirm-modal-confirm"));

    await waitFor(() => expect(toastErrorSpy).toHaveBeenCalled());
    const [summary, options] = toastErrorSpy.mock.calls[toastErrorSpy.mock.calls.length - 1];
    expect(summary).toBe("纳管并同步完成：成功 1，失败 1");
    expect(options?.description).toContain("serena");
    expect(options?.description).toContain("完整原因见操作记录");
    expect(options?.description).not.toContain("NODE_REPL_INSTRUCTIONS");
    toastErrorSpy.mockRestore();
  });

  it("全部纳管并同步：把发现的未导入服务先保存定义、接取变量并写回受管标记", async () => {
    const routes = mcpRoutes();
    const status = routes["mcp:status"] as ReturnType<typeof makeEnvelope<StatusData>>;
    status.data.import_candidates = [
      {
        name: "new-search",
        agent: "codex",
        transport: "stdio",
        spec: { command: "npx", args: ["-y", "new-search-mcp"], env: { SEARCH_TOKEN: "<literal>" } },
      },
    ];
    routes["secrets:adopt"] = makeEnvelope({ domain: "secrets", adopted: ["SEARCH_TOKEN"] }, { command: "secrets.adopt" });
    routes["policy:set"] = makeEnvelope({ layers: null, effective: {} }, { command: "policy.set" });
    const { calls } = renderWithEngine(<McpPage />, routes);

    await userEvent.click(await screen.findByTestId("sync-all-mcp"));
    const dialog = await screen.findByRole("dialog", { name: "全部纳管并同步" });
    expect(dialog).toHaveTextContent("将执行 3 项助手配置同步，涉及 3 个服务");
    expect(dialog).toHaveTextContent("1 个未纳管服务会先导入服务库");
    expect(dialog).toHaveTextContent("1 个服务会把原配置里的变量取值转存到“设置 → 密钥”选择的外部文件");
    expect(dialog).toHaveTextContent("2 项会用服务库定义接管同名条目");

    await userEvent.click(screen.getByTestId("confirm-modal-confirm"));
    await waitFor(() => expect(calls.filter((call) => call.command === "mcp:apply")).toHaveLength(3));

    expect(calls.find((call) => call.command === "secrets:adopt")?.args).toEqual([
      "--agent",
      "codex",
      "--server",
      "new-search",
    ]);
    const policyCall = calls.find((call) => call.command === "policy:set")!;
    expect(policyCall.args.slice(0, 4)).toEqual(["--layer", "local", "--key", "mcp.definitions.new-search"]);
    expect(JSON.parse(policyCall.args[5])).toMatchObject({
      command: "npx",
      args: ["-y", "new-search-mcp"],
      env: { SEARCH_TOKEN: "${SEARCH_TOKEN}" },
      agents: ["codex"],
      _replace: true,
    });
    expect(calls.filter((call) => call.command === "mcp:apply")[0].args).toEqual([
      "--only",
      "mcp:new-search",
      "--agent",
      "codex",
      "--action",
      "add",
      "--allow-takeover",
    ]);
  });

  it("缺少变量时在当前服务卡片直接填写，不离开 MCP 页", async () => {
    const routes = {
      ...mcpRoutes(),
      "secrets:set": makeEnvelope({}, { command: "secrets.set" }),
    };
    const { calls } = renderWithEngine(<McpPage />, routes);

    const resolution = await screen.findByTestId("mcp-resolution-mcp:serena");
    expect(resolution).toHaveTextContent("凭证变量未配置: SERENA_KEY");
    await userEvent.click(screen.getByTestId("configure-secrets-mcp:serena"));
    expect(await screen.findByRole("dialog", { name: "填写 SERENA_KEY" })).toBeInTheDocument();
    expect(screen.getByTestId("secret-value-save")).toBeDisabled();

    await userEvent.type(screen.getByTestId("secret-value-input"), "test-only-value");
    expect(screen.getByTestId("secret-value-save")).toBeEnabled();
    await userEvent.click(screen.getByTestId("secret-value-save"));
    await waitFor(() => expect(calls.some((call) => call.command === "secrets:set")).toBe(true));

    const save = calls.find((call) => call.command === "secrets:set")!;
    expect(save.args).toEqual(["--var", "SERENA_KEY", "--value", "test-only-value"]);
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "填写 SERENA_KEY" })).toBeNull());
  });

  it("助手中的服务：单元格态 已配置/不一致/以前配过/尚未生成，版本短摘要 title 存完整值", async () => {
    renderWithEngine(<McpPage />, mcpRoutes());
    await userEvent.click(await screen.findByRole("tab", { name: "助手中的服务" }));

    // 矩阵格的 title 带上版本等细节（原来「最新版本 / 当前配置版本」两列多半整列同一个值，
    // 而且「当前配置版本=未配置」与「状态=未配置」并排让人分不清在说什么，已去掉）
    const cellByPrefix = (prefix: string) =>
      screen.getAllByTitle((value) => value.startsWith(prefix))[0];
    expect(await screen.findByText("已配置")).toBeInTheDocument();
    const drifted = cellByPrefix("claude-code: drifted");
    expect(drifted).toHaveTextContent("不一致");
    expect(drifted.className).toContain("warning");
    // 「残留」对用户没意义，改成说清处境的说法
    expect(cellByPrefix("claude-code: leftover")).toHaveTextContent("以前配过");
    expect(cellByPrefix("codex: not-materialized")).toHaveTextContent("尚未生成");
    // 完整版本进 title，不单开一列
    expect(screen.getAllByTitle((v) => v.includes(LONG_VERSION)).length).toBeGreaterThan(0);

    // 助手筛选收窄列
    const agentSelect = screen.getByTestId("filter-agent-select") as HTMLSelectElement;
    await userEvent.selectOptions(agentSelect, "codex");
    expect(screen.queryAllByTitle((v) => v.startsWith("claude-code: drifted"))).toHaveLength(0);
    await userEvent.click(screen.getByRole("button", { name: /清除筛选/ }));
    expect(cellByPrefix("claude-code: drifted")).toBeInTheDocument();
  });

  it("行操作：按 cell.actions 渲染，「更新」点击直接 mcp:apply 精确 --only/--agent/--action（带 run_id），不取 plan、不开弹窗、零 policy:set", async () => {
    const { calls } = renderWithEngine(<McpPage />, mcpRoutes());
    await userEvent.click(await screen.findByRole("tab", { name: "助手中的服务" }));

    // actions 之外的按钮不出现（更新与启用不混）
    await expandAllMcpRows();
    expect(await screen.findByTestId("action-update-mcp:chrome-devtools-claude-code")).toBeInTheDocument();
    expect(screen.queryByTestId("action-enable-mcp:chrome-devtools-claude-code")).toBeNull();
    // actions 为空的 cell 不推测动作；直接显示引擎给出的可处理原因
    expect(screen.getAllByText("凭证变量未配置: SERENA_KEY").length).toBeGreaterThan(0);

    await userEvent.click(screen.getByTestId("action-update-mcp:chrome-devtools-claude-code"));
    await waitFor(() => expect(calls.some((call) => call.command === "mcp:apply")).toBe(true));

    const applyCalls = calls.filter((call) => call.command === "mcp:apply");
    expect(applyCalls).toHaveLength(1);
    expect(applyCalls[0].args).toEqual(["--only", "mcp:chrome-devtools", "--agent", "claude-code", "--action", "update"]);
    expect(applyCalls[0].args).not.toContain("--dry-run");
    expect(applyCalls[0].options?.runId).toMatch(/^gui-/);
    expect(calls.some((call) => call.command.endsWith(":plan"))).toBe(false);
    expect(calls.some((call) => call.command === "policy:set")).toBe(false);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("库级删除：点击先打开 ConfirmModal，取消时零 apply/零 policy:set", async () => {
    const { calls } = renderWithEngine(<McpPage />, mcpRoutes());

    await userEvent.click(await screen.findByTestId("remove-server-mcp:chrome-devtools"));
    const dialog = await screen.findByRole("dialog", { name: "删除服务 chrome-devtools" });
    expect(dialog).toHaveTextContent(/但不会卸载软件/);
    expect(screen.getByTestId("confirm-modal-input")).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "取消" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());

    expect(calls.some((call) => call.command === "mcp:apply")).toBe(false);
    expect(calls.some((call) => call.command === "policy:set")).toBe(false);
    expect(calls.some((call) => call.command.endsWith(":plan"))).toBe(false);
  });

  it("库级删除：输入服务名确认后 mcp:apply --action remove（带 run_id），无 plan、零 policy:set", async () => {
    const { calls } = renderWithEngine(<McpPage />, mcpRoutes());

    await userEvent.click(await screen.findByTestId("remove-server-mcp:chrome-devtools"));
    await screen.findByRole("dialog", { name: "删除服务 chrome-devtools" });
    await userEvent.type(screen.getByTestId("confirm-modal-input"), "chrome-devtools");
    await userEvent.click(screen.getByTestId("confirm-modal-confirm"));

    await waitFor(() => expect(calls.some((call) => call.command === "mcp:apply")).toBe(true));
    const applyCalls = calls.filter((call) => call.command === "mcp:apply");
    expect(applyCalls).toHaveLength(1);
    expect(applyCalls[0].args).toEqual(["--only", "mcp:chrome-devtools", "--action", "remove"]);
    expect(applyCalls[0].args).not.toContain("--dry-run");
    expect(applyCalls[0].options?.runId).toMatch(/^gui-/);
    expect(calls.some((call) => call.command.endsWith(":plan"))).toBe(false);
    expect(calls.some((call) => call.command === "policy:set")).toBe(false);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("服务 × 助手 remove：点击先确认，确认后 mcp:apply 精确 --only/--agent/--action remove，取消零 apply", async () => {
    const { calls } = renderWithEngine(<McpPage />, mcpRoutes());
    await userEvent.click(await screen.findByRole("tab", { name: "助手中的服务" }));

    await expandAllMcpRows();
    const removeBtn = await screen.findByTestId("action-remove-mcp:chrome-devtools-codex");
    await userEvent.click(removeBtn);
    const dialog = await screen.findByRole("dialog", { name: "从此助手删除 mcp:chrome-devtools @ codex" });
    expect(dialog).toHaveTextContent(/只清理该助手的配置/);

    await userEvent.click(screen.getByRole("button", { name: "取消" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(calls.some((call) => call.command === "mcp:apply")).toBe(false);

    await userEvent.click(removeBtn);
    await screen.findByRole("dialog", { name: "从此助手删除 mcp:chrome-devtools @ codex" });
    await userEvent.click(screen.getByTestId("confirm-modal-confirm"));

    await waitFor(() => expect(calls.filter((call) => call.command === "mcp:apply")).toHaveLength(1));
    expect(calls.filter((call) => call.command === "mcp:apply")[0].args).toEqual([
      "--only", "mcp:chrome-devtools", "--agent", "codex", "--action", "remove",
    ]);
    expect(calls.some((call) => call.command.endsWith(":plan"))).toBe(false);
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

describe("McpPage 空态引导", () => {
  function emptyRoutes(importCandidates: unknown[] = []): Record<string, ReturnType<typeof makeEnvelope>> {
    return {
      "mcp:status": makeEnvelope(
        { domain: "mcp", elements: [], import_candidates: importCandidates } as unknown as StatusData,
        { command: "mcp.status" },
      ),
      "agents:status": makeEnvelope({ domain: "agents", elements: [] } as unknown as StatusData, { command: "agents.status" }),
    };
  }

  it("无服务定义且无导入候选：引导点击「新增服务」，不出现导入提示", async () => {
    renderWithEngine(<McpPage />, emptyRoutes());

    expect(await screen.findByText("暂无 MCP 服务定义")).toBeInTheDocument();
    expect(screen.getByText(/点击右上角「新增服务」创建第一个服务/)).toBeInTheDocument();
    expect(screen.queryByText(/可导入的现有配置/)).toBeNull();
    // 「新增服务」按钮本身就在空态上方
    expect(screen.getByTestId("add-server-button")).toBeInTheDocument();
  });

  it("无服务定义但有导入候选：补充提示可在「发现未导入的服务器」中导入", async () => {
    renderWithEngine(
      <McpPage />,
      emptyRoutes([{ name: "grok-search", agent: "claude-code", transport: "stdio", spec: { command: "npx" } }]),
    );

    expect(await screen.findByText(/检测到 1 个可导入的现有配置/)).toHaveTextContent("发现未导入的服务器");
  });

  it("助手中的服务：筛选后无记录时引导清除筛选，全部视图下引导先到服务库新增", async () => {
    const routes = emptyRoutes();
    routes["agents:status"] = makeEnvelope(
      {
        domain: "agents",
        elements: [makeElement({ id: "agents:claude-code", domain: "agents" })],
      } as unknown as StatusData,
      { command: "agents.status" },
    );
    renderWithEngine(<McpPage />, routes);
    await userEvent.click(await screen.findByRole("tab", { name: "助手中的服务" }));

    // 全部视图：无服务定义 → 引导先到服务库新增
    expect(await screen.findByText("暂无可展示的服务配置")).toBeInTheDocument();
    expect(screen.getByText("请先在「服务库」新增服务")).toBeInTheDocument();

    // 收窄筛选后：引导清除筛选
    await userEvent.selectOptions(screen.getByTestId("filter-agent-select"), "claude-code");
    expect(screen.getByText("无符合筛选条件的服务配置")).toBeInTheDocument();
    expect(screen.getByText(/「清除筛选」后查看全部助手/)).toBeInTheDocument();
  });
});

describe("MCP 命令可用性提示", () => {
  it("命令在本机找不到时给出提示；远程服务与可用命令都不提示", async () => {
    const routes = mcpRoutes();
    routes["mcp:status"] = makeEnvelope(
      {
        domain: "mcp",
        elements: [
          makeElement({
            id: "mcp:missing-tool",
            domain: "mcp",
            spec: { command: "not-installed", args: [], agents: null, env: {} },
            observed: { agents: {}, env: {}, actions: [], command_available: false },
            state: "not-materialized",
            freshness: makeFreshness({ state: "not-applicable" }),
          }),
          makeElement({
            id: "mcp:ok-tool",
            domain: "mcp",
            spec: { command: "npx", args: [], agents: null, env: {} },
            observed: { agents: {}, env: {}, actions: [], command_available: true },
            state: "not-materialized",
            freshness: makeFreshness({ state: "not-applicable" }),
          }),
          makeElement({
            id: "mcp:remote-one",
            domain: "mcp",
            spec: { url: "https://mcp.example.com/sse", headers: {}, agents: null },
            observed: { agents: {}, headers: {}, actions: [], command_available: null },
            state: "not-materialized",
            freshness: makeFreshness({ state: "not-applicable" }),
          }),
        ],
      },
      { command: "mcp.status" },
    );
    renderWithEngine(<McpPage />, routes);

    expect(await screen.findByTestId("command-missing-mcp:missing-tool")).toHaveTextContent("找不到它要执行的命令");
    expect(screen.queryByTestId("command-missing-mcp:ok-tool")).toBeNull();
    expect(screen.queryByTestId("command-missing-mcp:remote-one")).toBeNull();
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
      // 引擎 fail-closed：无 --allow-takeover 一律拒绝，带 flag 才执行
      "mcp:apply": async (args: string[]) => {
        if (!args.includes("--allow-takeover")) {
          return makeEnvelope(
            {},
            { command: "mcp.apply", ok: false, errors: [{ code: "PREFLIGHT_FAILED", element: null, message: CONFLICT_MESSAGE }] },
          );
        }
        return makeEnvelope(makeApplyResult({ domain: "mcp" }), { command: "mcp.apply" });
      },
    };
  }

  it("add 遇「非受管同名条目」fail-closed → 失败 toast 提供「接管原条目并重试」→ 点击直接以原范围 + --allow-takeover 重试 apply（不取 plan、无二次确认）", async () => {
    const { calls } = renderWithEngine(<McpPage />, takeoverRoutes());
    await userEvent.click(await screen.findByRole("tab", { name: "助手中的服务" }));

    // codex cell 提供正常「添加」：点击直接 apply，首次因冲突 fail-closed
    await expandAllMcpRows();
    await userEvent.click(await screen.findByTestId("action-add-mcp:take-svc-codex"));
    await waitFor(() => expect(calls.some((call) => call.command === "mcp:apply")).toBe(true));
    const firstApply = calls.find((call) => call.command === "mcp:apply");
    expect(firstApply!.args).toEqual(["--only", "mcp:take-svc", "--agent", "codex", "--action", "add"]);
    expect(firstApply!.args).not.toContain("--allow-takeover");
    expect(firstApply!.options?.runId).toMatch(/^gui-/);
    expect(calls.some((call) => call.command.endsWith(":plan"))).toBe(false);

    // 失败反馈外提供接管动作；全程无弹窗、无二次确认
    const takeoverAction = await screen.findByRole("button", { name: "接管原条目并重试" });
    expect(screen.queryByRole("dialog")).toBeNull();
    await userEvent.click(takeoverAction);

    await waitFor(() => expect(calls.filter((call) => call.command === "mcp:apply")).toHaveLength(2));
    const retryApply = calls.filter((call) => call.command === "mcp:apply")[1];
    expect(retryApply.args).toEqual(["--only", "mcp:take-svc", "--agent", "codex", "--action", "add", "--allow-takeover"]);
    expect(retryApply.args).not.toContain("--dry-run");
    expect(retryApply.options?.runId).toMatch(/^gui-/);
    expect(calls.some((call) => call.command.endsWith(":plan"))).toBe(false);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("冲突 cell 行内「接管原条目」：一键直接 apply 带 --allow-takeover，不取 plan、不开弹窗，成功走既有执行 toast", async () => {
    const toastSuccessSpy = vi.spyOn(toast, "success");
    const { calls } = renderWithEngine(<McpPage />, takeoverRoutes());
    await userEvent.click(await screen.findByRole("tab", { name: "助手中的服务" }));

    // 冲突 cell：无常规动作，提供显式接管入口（assigned=false → add）
    expect(screen.queryByTestId("action-add-mcp:take-svc-claude-code")).toBeNull();
    await expandAllMcpRows();
    await userEvent.click(await screen.findByTestId("takeover-mcp:take-svc-claude-code"));

    await waitFor(() => expect(toastSuccessSpy).toHaveBeenCalled());
    const applyCalls = calls.filter((call) => call.command === "mcp:apply");
    expect(applyCalls).toHaveLength(1);
    expect(applyCalls[0].args).toEqual(["--only", "mcp:take-svc", "--agent", "claude-code", "--action", "add", "--allow-takeover"]);
    expect(applyCalls[0].args).not.toContain("--dry-run");
    expect(applyCalls[0].options?.runId).toMatch(/^gui-/);
    expect(calls.some((call) => call.command.endsWith(":plan"))).toBe(false);
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(toastSuccessSpy.mock.calls[toastSuccessSpy.mock.calls.length - 1][0]).toContain("接管原条目 mcp:take-svc @ claude-code");
  });

  it("受管条目含额外字段时提供明确覆盖动作，并以 update + --allow-takeover 收敛", async () => {
    const routes = takeoverRoutes();
    const status = routes["mcp:status"] as ReturnType<typeof makeEnvelope<StatusData>>;
    const cell = status.data.elements[0].observed.agents as Record<string, Record<string, unknown>>;
    cell["claude-code"] = {
      state: "drifted",
      target: "/tmp/demo/.claude.json",
      target_exists: true,
      assigned: true,
      enabled: true,
      actions: [],
      blocked_reason: "目标文件中的条目包含未受管外部属性（外部未识别字段 'custom_field'），拒绝覆盖",
    };
    cell.codex = {
      state: "managed-ok",
      target: "/tmp/demo/.codex/config.toml",
      target_exists: true,
      assigned: true,
      enabled: true,
      actions: ["disable", "remove"],
    };
    const { calls } = renderWithEngine(<McpPage />, routes);

    await userEvent.click(await screen.findByTestId("sync-server-mcp:take-svc"));
    const dialog = await screen.findByRole("dialog", { name: "同步 take-svc" });
    expect(dialog).toHaveTextContent("1 项会用服务库定义接管同名条目");
    await userEvent.click(screen.getByTestId("confirm-modal-confirm"));

    await waitFor(() => expect(calls.filter((call) => call.command === "mcp:apply")).toHaveLength(1));
    expect(calls.filter((call) => call.command === "mcp:apply")[0].args).toEqual([
      "--only",
      "mcp:take-svc",
      "--action",
      "update",
      "--agent",
      "claude-code",
      "--allow-takeover",
    ]);
  });

  it("apply 因非冲突原因失败：失败 toast 可观测，不提供接管重试", async () => {
    const failureRoutes = takeoverRoutes();
    failureRoutes["mcp:apply"] = async () =>
      makeEnvelope(
        {},
        { command: "mcp.apply", ok: false, errors: [{ code: "APPLY_FAILED", element: null, message: "凭证变量未配置: SOME_TOKEN" }] },
      );
    const toastErrorSpy = vi.spyOn(toast, "error");
    const { calls } = renderWithEngine(<McpPage />, failureRoutes);
    await userEvent.click(await screen.findByRole("tab", { name: "助手中的服务" }));

    await expandAllMcpRows();
    await userEvent.click(await screen.findByTestId("action-add-mcp:take-svc-codex"));

    await waitFor(() => expect(toastErrorSpy).toHaveBeenCalled());
    expect(toastErrorSpy).toHaveBeenCalledWith(expect.stringContaining("凭证变量未配置"));
    // 给潜在的接管动作 toast 一个节拍，确认不会出现
    await new Promise((resolve) => setTimeout(resolve, 50));
    const applyCalls = calls.filter((call) => call.command === "mcp:apply");
    expect(applyCalls).toHaveLength(1);
    expect(applyCalls[0].args).toEqual(["--only", "mcp:take-svc", "--agent", "codex", "--action", "add"]);
    expect(applyCalls[0].options?.runId).toMatch(/^gui-/);
    expect(calls.some((call) => call.command.endsWith(":plan"))).toBe(false);
    expect(toastErrorSpy).not.toHaveBeenCalledWith(expect.stringContaining("可显式接管"));
    expect(screen.queryByRole("button", { name: "接管原条目并重试" })).toBeNull();
  });
  it("官方有新版时给出换版本按钮：点了就把定义钉到新版，uvx 源没有这个按钮", async () => {
    const { calls } = renderWithEngine(<McpPage />, mcpRoutes());

    // uvx 跟的是分支，没有版本号可钉，不该出现这个按钮
    await screen.findByTestId("mcp-card-mcp:serena");
    expect(screen.queryByTestId("upgrade-server-mcp:serena")).toBeNull();

    const button = await screen.findByTestId("upgrade-server-mcp:chrome-devtools");
    expect(button).toHaveTextContent("换到 1.8.0");
    await userEvent.click(button);

    await waitFor(() => {
      const applyCalls = calls.filter((call) => call.command === "mcp:apply");
      expect(applyCalls).toHaveLength(1);
      expect(applyCalls[0].args).toEqual(["--only", "mcp:chrome-devtools", "--action", "upgrade"]);
    });
    // 换完顺手重查一次，卡片不该挂着旧读数
    await waitFor(() =>
      expect(
        calls.some(
          (call) =>
            call.command === "system:refresh-upstream" &&
            call.args?.[1] === "mcp:chrome-devtools",
        ),
      ).toBe(true),
    );
  });
});
