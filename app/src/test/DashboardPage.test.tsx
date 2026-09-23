import { QueryClientProvider } from "@tanstack/react-query";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Toaster } from "sonner";
import { afterEach, describe, expect, it } from "vitest";
import DashboardPage from "../pages/DashboardPage";
import { setEngineClient } from "../lib/engine";
import { ExecutionProvider } from "../lib/execution";
import { createQueryClient } from "../lib/queries";
import { createMswEngineClient, startEngineServer, type EngineRoute } from "./engine-test-client";
import { makeAction, makeAliasStatus, makeApplyResult, makeAuditData, makeElement, makeEnvelope, makeFreshness } from "./fixtures";
import type { RefreshUpstreamData } from "../lib/contract";

const servers: Array<ReturnType<typeof startEngineServer>> = [];

afterEach(() => {
  while (servers.length > 0) {
    servers.pop()?.close();
  }
});

function renderDashboard(routes: Record<string, EngineRoute>) {
  servers.push(startEngineServer(routes));
  setEngineClient(createMswEngineClient());
  const queryClient = createQueryClient({ retry: false });
  return render(
    <QueryClientProvider client={queryClient}>
      {/* 总览清单能一键同步，会执行 apply，所以需要执行通道 */}
      <ExecutionProvider>
        <DashboardPage onNavigate={() => {}} />
      </ExecutionProvider>
      <Toaster richColors position="bottom-right" />
    </QueryClientProvider>,
  );
}

function okRoutes(): Record<string, EngineRoute> {
  return {
    "system:audit": makeEnvelope(
      makeAuditData({
        status: "warning",
        sections: {
          runtime: [{ name: "engine-rs", status: "ok", message: "Rust 原生引擎" }],
          agents: [{ name: "Kimi Code", status: "notice", message: "未声明策略（已忽略）" }],
          mcp: [{ name: "Codex", status: "warning", message: "托管产物尚未生成", category: "unmaterialized" }],
        },
        summary: {
          errors: 0,
          warnings: 1,
          warning_categories: { optional_tools: 2, unmaterialized: 1, official_gaps: 0 },
        },
      }),
      { command: "system.audit" },
    ),
    "agents:status": makeEnvelope(
      {
        domain: "agents",
        elements: [
          makeElement({
            id: "agents:claude-code",
            domain: "agents",
            state: "managed-ok",
            freshness: makeFreshness({
              state: "behind",
              current: "2.0.0",
              upstream: "2.1.3",
              checked_at: "2026-09-05T10:00:00Z",
              probe: "npm-view",
              cache_age_sec: 120,
            }),
          }),
          makeElement({
            id: "agents:codex",
            domain: "agents",
            observed: {
              usage: {
                source: ".codex/sessions",
                checked_at: "2026-09-05T11:00:00Z",
                windows: [
                  { window_minutes: 300, used_percent: 12, resets_at: null },
                  { window_minutes: 10080, used_percent: 65, resets_at: null },
                ],
              },
            },
            freshness: makeFreshness({
              state: "current",
              current: "0.42.0",
              upstream: "0.42.0",
              checked_at: "2026-09-05T11:00:00Z",
              probe: "npm-view",
              cache_age_sec: 60,
            }),
          }),
        ],
      },
      { command: "agents.status" },
    ),
    "mcp:status": makeEnvelope(
      {
        domain: "mcp",
        elements: [
          makeElement({
            id: "mcp:chrome-devtools",
            domain: "mcp",
            state: "managed-ok",
            observed: { agents: { "claude-code": { state: "enabled", source: "file", base: "a".repeat(64), actions: [] } } },
            freshness: makeFreshness({
              state: "behind",
              current: "1.2.0",
              upstream: "1.8.0",
              checked_at: "2026-09-05T09:00:00Z",
              probe: "npm-view",
              cache_age_sec: 300,
            }),
          }),
          makeElement({
            id: "mcp:grok-search",
            domain: "mcp",
            state: "disabled",
            observed: { agents: { codex: { state: "disabled", source: "file", base: "b".repeat(64), actions: [] } } },
            freshness: makeFreshness({ state: "unknown" }),
          }),
        ],
      },
      { command: "mcp.status" },
    ),
    "skills:status": makeEnvelope(
      {
        domain: "skills",
        elements: [
          makeElement({
            id: "skill:agent-skills/find-skills",
            domain: "skills",
            freshness: makeFreshness({
              state: "current",
              current: "a1b2c3d",
              upstream: "a1b2c3d",
              checked_at: "2026-09-05T10:30:00Z",
              probe: "git-ls-remote",
              cache_age_sec: 90,
            }),
          }),
          makeElement({ id: "skill:agent-skills", domain: "skills", freshness: makeFreshness({ state: "not-applicable" }) }),
        ],
      },
      { command: "skills.status" },
    ),
    "profiles:status": makeEnvelope({ domain: "profiles", elements: [] }, { command: "profiles.status" }),
    "rules:status": makeEnvelope({ domain: "rules", elements: [] }, { command: "rules.status" }),
    "hooks:status": makeEnvelope({ domain: "hooks", elements: [] }, { command: "hooks.status" }),
    "aliases:status": makeEnvelope(
      makeAliasStatus(
        { cc: "claude", yolo: "claude --dangerously-skip-permissions" },
        { recommendations: [{ name: "grok", value: "grok", line: "alias grok='grok'", source: "agent-skills" }] },
      ),
      { command: "aliases.status" },
    ),
  };
}

describe("DashboardPage", () => {
  it("清单按域列出每一项，并把 AI CLI 版本维护收敛到只读诊断页", async () => {
    renderDashboard(okRoutes());

    // 助手：总览只显示配置/额度状态，不再暴露版本与更新入口。
    const agents = await screen.findByTestId("overview-agents");
    expect(within(agents).getByTestId("overview-item-agents:claude-code")).not.toHaveTextContent("2.0.0");
    expect(within(agents).getByTestId("overview-item-agents:claude-code")).not.toHaveTextContent("2.1.3");
    expect(within(agents).queryByText("可更新")).not.toBeInTheDocument();
    expect(within(agents).queryByText("已是最新")).not.toBeInTheDocument();
    // 额度只放最吃紧的那个窗口（7 天 65% 而不是 5 小时 12%）；这份夹具的记账时刻很旧，
    // 数字后面带问号提示别当真
    expect(within(agents).getByTestId("overview-item-agents:codex")).toHaveTextContent("额度 65%?");
    expect(within(agents).getByTestId("overview-item-agents:claude-code")).not.toHaveTextContent("额度");

    // MCP：实际条目就是当前配置，没有「待同步 / 要你决定」（原断言来自已退役的服务库模型）；
    // 只看启用与否，更新轴照常显示
    const mcp = await screen.findByTestId("overview-mcp");
    expect(within(mcp).getByTestId("overview-item-mcp:chrome-devtools")).not.toHaveTextContent("要你决定");
    expect(within(mcp).getByTestId("overview-item-mcp:chrome-devtools")).toHaveTextContent("可更新");
    expect(within(mcp).getByTestId("overview-item-mcp:grok-search")).toHaveTextContent("已停用");
    expect(within(mcp).getByTestId("overview-item-mcp:grok-search")).toHaveTextContent("未检查");

    // 技能：来源已经展开成单个技能后，来源本身不再占一行
    const skills = await screen.findByTestId("overview-skills");
    expect(within(skills).getByTestId("overview-item-skill:agent-skills/find-skills")).toBeInTheDocument();
    expect(within(skills).queryByTestId("overview-item-skill:agent-skills")).not.toBeInTheDocument();

    // 快捷命令：以终端配置文件里的实际行为准，按名字列出，永远不算待同步；推荐不进清单
    const aliases = await screen.findByTestId("overview-aliases");
    expect(within(aliases).getByTestId("overview-item-alias:cc")).toHaveTextContent("已同步");
    expect(within(aliases).getByTestId("overview-item-alias:yolo")).toHaveTextContent("已同步");
    expect(within(aliases).queryByTestId("overview-item-alias:grok")).toBeNull();
    expect(screen.queryByTestId("sync-all-button")).toBeNull();

    // 空的域不占位置
    expect(screen.queryByTestId("overview-profiles")).not.toBeInTheDocument();

    // 引擎自检明细已撤下：清单本身就回答「要不要动手」
    expect(screen.queryByTestId("audit-details")).toBeNull();
  });

  it("一键同步：只对待同步的执行 apply，要你决定的不碰", async () => {
    const routes = okRoutes();
    // 技能来源有可用更新 → 待同步（原夹具用快捷命令当待同步项：快捷命令已改为以实际区块为准，
    // 不再有待同步）。只注册 skills:apply；MCP 与快捷命令不进批量，若误算进去未注册的 apply 会记为失败
    const skills = routes["skills:status"] as ReturnType<typeof makeEnvelope<{ domain: string; elements: ReturnType<typeof makeElement>[] }>>;
    skills.data.elements = skills.data.elements.map((element) =>
      element.id === "skill:agent-skills"
        ? { ...element, freshness: makeFreshness({ state: "behind", current: "a1b2c3d", upstream: "f0e1d2c" }), actions: [makeAction("update")] }
        : element,
    );
    routes["skills:apply"] = makeEnvelope(makeApplyResult({ domain: "skills" }), { command: "skills.apply" });
    renderDashboard(routes);

    const button = await screen.findByTestId("sync-all-button");
    // 待同步只有这一个技能来源；MCP 与快捷命令没有待同步
    expect(button).toHaveTextContent("全部同步（1）");
    await userEvent.click(button);

    const confirm = await screen.findByRole("dialog", { name: "同步 1 个待处理项目" });
    expect(within(confirm).getByTestId("sync-all-preview")).toHaveTextContent("agent-skills");
    expect(within(confirm).getByText(/“要你决定”的项目不会执行/)).toBeInTheDocument();
    await userEvent.click(within(confirm).getByRole("button", { name: "确认同步" }));

    expect(await screen.findByText("同步完成：成功 1，失败 0")).toBeInTheDocument();
  });

  it("envelope errors 时 ErrorPanel 展示 code/message，其余内容照常渲染", async () => {
    const routes = okRoutes();
    routes["system:audit"] = makeEnvelope(
      makeAuditData({ status: "error", summary: { errors: 6, warnings: 0, warning_categories: { optional_tools: 0, unmaterialized: 0, official_gaps: 0 } } }),
      { command: "system.audit", ok: false, errors: [{ code: "AUDIT_ERRORS", element: null, message: "doctor 报告 6 个错误" }] },
    );
    renderDashboard(routes);

    expect(await screen.findByText("AUDIT_ERRORS")).toBeInTheDocument();
    expect(screen.getByText("doctor 报告 6 个错误")).toBeInTheDocument();
  });

  it("桥错误（非 envelope）时展示结构化错误码", async () => {
    const routes = okRoutes();
    routes["system:audit"] = { status: 500, body: { kind: "spawn_failed", message: "Rust 原生引擎不可用" } };
    renderDashboard(routes);

    expect(await screen.findByText("SPAWN_FAILED")).toBeInTheDocument();
    expect(screen.getByText(/Rust 原生引擎不可用/)).toBeInTheDocument();
  });

  it("「刷新上游状态」按钮：调用 system:refresh-upstream 并 toast 汇总计数", async () => {
    const routes = okRoutes();
    routes["system:refresh-upstream"] = makeEnvelope<RefreshUpstreamData>(
      {
        domain: "system",
        verb: "refresh-upstream",
        cache_file: null,
        elements: [],
        summary: { probed: 5, current: 3, behind: 1, ahead: 0, unknown: 1 },
      },
      { command: "system.refresh-upstream" },
    );
    renderDashboard(routes);

    const button = await screen.findByTestId("refresh-upstream-button");
    expect(button).toBeEnabled();
    await userEvent.click(button);

    expect(await screen.findByText(/检查更新完成/)).toBeInTheDocument();
    // 刷新完成后缓存失效重新拉取，清单照常在
    await screen.findByTestId("overview-agents");
  });
});
