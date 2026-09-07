import { QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Toaster } from "sonner";
import { afterEach, describe, expect, it } from "vitest";
import DashboardPage from "../pages/DashboardPage";
import { setEngineClient } from "../lib/engine";
import { createQueryClient } from "../lib/queries";
import { createMswEngineClient, startEngineServer, type EngineRoute } from "./engine-test-client";
import { makeAuditData, makeElement, makeEnvelope, makeFreshness } from "./fixtures";
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
      <DashboardPage />
      <Toaster richColors position="top-right" />
    </QueryClientProvider>,
  );
}

function okRoutes(): Record<string, EngineRoute> {
  return {
    "system:audit": makeEnvelope(
      makeAuditData({
        status: "warning",
        sections: {
          config: [{ name: "manager.json", status: "ok", message: "格式正确" }],
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
    "tools:status": makeEnvelope(
      {
        domain: "tools",
        elements: [
          makeElement({
            id: "tools:playwright-cli",
            domain: "tools",
            freshness: makeFreshness({ state: "unknown" }),
          }),
        ],
      },
      { command: "tools.status" },
    ),
    "mcp:status": makeEnvelope(
      {
        domain: "mcp",
        elements: [
          makeElement({
            id: "mcp:chrome-devtools",
            domain: "mcp",
            state: "drifted",
            freshness: makeFreshness({
              state: "behind",
              current: "1.2.0",
              upstream: "1.8.0",
              checked_at: "2026-09-05T09:00:00Z",
              probe: "npm-view",
              cache_age_sec: 300,
            }),
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
          makeElement({ id: "rules:claude_global", domain: "skills", freshness: makeFreshness({ state: "not-applicable" }) }),
        ],
      },
      { command: "skills.status" },
    ),
  };
}

describe("DashboardPage", () => {
  it("Git 版本信息为空时仍渲染总览并显示不可用提示", async () => {
    const routes = okRoutes();
    routes["system:audit"] = makeEnvelope(makeAuditData({ version: null }), { command: "system.audit" });
    renderDashboard(routes);

    expect(await screen.findByTestId("audit-status-card")).toBeInTheDocument();
    expect(screen.getByText(/版本 信息不可用/)).toBeInTheDocument();
  });

  it("渲染总状态卡、警告分类与 freshness 聚合卡（计数正确，not-applicable 不计入）", async () => {
    renderDashboard(okRoutes());

    const statusCard = await screen.findByTestId("audit-status-card");
    expect(within(statusCard).getByText("存在警告")).toBeInTheDocument();
    expect(within(statusCard).getByText(/错误 0 · 警告 1/)).toBeInTheDocument();

    expect(screen.getByTestId("warning-count-optional_tools")).toHaveTextContent("2");
    expect(screen.getByTestId("warning-count-unmaterialized")).toHaveTextContent("1");
    expect(screen.getByTestId("warning-count-official_gaps")).toHaveTextContent("0");

    await waitFor(() => expect(screen.getByTestId("freshness-count-agents-behind")).toHaveTextContent("1"));
    expect(screen.getByTestId("freshness-count-agents-current")).toHaveTextContent("1");
    expect(screen.getByTestId("freshness-count-agents-unknown")).toHaveTextContent("0");
    expect(screen.getByTestId("freshness-count-mcp-behind")).toHaveTextContent("1");
    expect(screen.getByTestId("freshness-count-tools-unknown")).toHaveTextContent("1");
    expect(screen.getByTestId("freshness-count-skills-current")).toHaveTextContent("1");
    // not-applicable 元素不进入任何计数桶
    expect(screen.getByTestId("freshness-count-skills-unknown")).toHaveTextContent("0");
    expect(screen.getByTestId("freshness-count-skills-behind")).toHaveTextContent("0");

    // 上次检查时间取各元素 checked_at 的最大值
    const agentsCard = screen.getByTestId("freshness-card-agents");
    expect(within(agentsCard).getByText(/上次检查/)).toHaveTextContent(/2026/);

    // 落后/未知元素表
    const staleTable = await screen.findByTestId("stale-elements");
    expect(within(staleTable).getByText("agents:claude-code")).toBeInTheDocument();
    expect(within(staleTable).getByText("mcp:chrome-devtools")).toBeInTheDocument();
    expect(within(staleTable).getByText("tools:playwright-cli")).toBeInTheDocument();

    // 审计明细按段分组渲染
    expect(screen.getByTestId("audit-section-config")).toHaveTextContent("manager.json");
    expect(screen.getByTestId("audit-section-mcp")).toHaveTextContent("托管产物尚未生成");
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
    const statusCard = await screen.findByTestId("audit-status-card");
    expect(within(statusCard).getByText("存在错误")).toBeInTheDocument();
  });

  it("桥错误（非 envelope）时展示结构化错误码", async () => {
    const routes = okRoutes();
    routes["system:audit"] = { status: 500, body: { kind: "spawn_failed", message: "找不到 manage.sh" } };
    renderDashboard(routes);

    expect(await screen.findByText("SPAWN_FAILED")).toBeInTheDocument();
    expect(screen.getByText(/找不到 manage\.sh/)).toBeInTheDocument();
  });

  it("「刷新上游状态」按钮：调用 system:refresh-upstream 并 toast 汇总计数", async () => {
    const routes = okRoutes();
    routes["system:refresh-upstream"] = makeEnvelope<RefreshUpstreamData>(
      {
        domain: "system",
        verb: "refresh-upstream",
        cache_file: "/tmp/upstream-cache.json",
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
    // 刷新完成后缓存失效重新拉取：agents:status 至少被调用过
    await waitFor(() => expect(screen.getByTestId("freshness-count-agents-behind")).toHaveTextContent("1"));
  });
});
