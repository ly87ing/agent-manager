import { describe, expect, it } from "vitest";
import { waitFor } from "@testing-library/react";
import AgentsPage from "../pages/AgentsPage";
import { makeAuditData, makeElement, makeEnvelope, makeFreshness, makeMode, makePlanAction } from "./fixtures";
import { renderWithEngine, screen, userEvent } from "./render-with-engine";
import type { Envelope } from "../lib/contract";

function routes(offline: boolean): Record<string, Envelope> {
  return {
    "system:audit": makeEnvelope(makeAuditData(), { command: "system.audit", mode: makeMode({ offline }) }),
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
            freshness: makeFreshness({ state: "behind", current: "2.0.0", upstream: "2.1.3", checked_at: "2026-09-06T00:00:00Z", probe: "npm-view", cache_age_sec: 60 }),
            actions: [
              { id: "upgrade", risk: "network", needs_lock: true, offline_ok: false },
              { id: "uninstall", risk: "install-remove", needs_lock: true, offline_ok: false },
            ],
          }),
        ],
      },
      { command: "agents.status" },
    ),
    "agents:plan": makeEnvelope(
      {
        domain: "agents",
        actions: [
          makePlanAction({ op: "run-cmd", element: "agents:claude-code", target: null, risk: "network", cmd: "npm install -g @anthropic-ai/claude-code", note: "升级到 2.1.3" }),
        ],
        summary: { read: 0, "write-local": 0, "exec-local": 0, network: 1, "install-remove": 0 },
      },
      { command: "agents.plan" },
    ),
  };
}

describe("Agents 页写动作", () => {
  it("升级按钮 → plan 预览（带 --action upgrade）→ network 知情勾选 → agents:apply 带 action 与 run_id", async () => {
    const { calls } = renderWithEngine(<AgentsPage />, routes(false));
    await userEvent.click(await screen.findByTestId("action-upgrade-agents:claude-code"));

    const dialog = await screen.findByRole("dialog");
    expect(dialog).toHaveTextContent("升级 Claude Code");
    expect(dialog).toHaveTextContent("npm install -g @anthropic-ai/claude-code");

    // 预览请求带显式动作
    const planCall = calls.find((call) => call.command === "agents:plan");
    expect(planCall!.args).toEqual(["--only", "agents:claude-code", "--action", "upgrade"]);

    // network 级：须勾选
    const confirm = screen.getByTestId("plan-confirm-button");
    expect(confirm).toBeDisabled();
    await userEvent.click(screen.getByTestId("network-ack-checkbox"));
    expect(confirm).toBeEnabled();
    await userEvent.click(confirm);

    const applyCall = calls.find((call) => call.command === "agents:apply");
    expect(applyCall).toBeDefined();
    expect(applyCall!.args).toEqual(["--only", "agents:claude-code", "--action", "upgrade"]);
    expect(applyCall!.options?.runId).toMatch(/^gui-/);
  });

  it("卸载：预览到确认全程零 policy:set，apply 带 --action uninstall 与 --allow-uninstall", async () => {
    const uninstallRoutes = routes(false);
    uninstallRoutes["agents:plan"] = makeEnvelope(
      {
        domain: "agents",
        actions: [
          makePlanAction({ op: "uninstall", element: "agents:claude-code", target: null, risk: "install-remove", cmd: "npm uninstall -g @anthropic-ai/claude-code", note: "显式卸载（provider: npm）" }),
        ],
        summary: { read: 0, "write-local": 0, "exec-local": 0, network: 0, "install-remove": 1 },
      },
      { command: "agents.plan" },
    );
    const { calls } = renderWithEngine(<AgentsPage />, uninstallRoutes);
    await userEvent.click(await screen.findByTestId("action-uninstall-agents:claude-code"));

    const dialog = await screen.findByRole("dialog");
    expect(dialog).toHaveTextContent("卸载 Claude Code");
    expect(dialog).toHaveTextContent(/不能自动撤销/);

    const planCall = calls.find((call) => call.command === "agents:plan");
    expect(planCall!.args).toEqual(["--only", "agents:claude-code", "--action", "uninstall"]);

    // install-remove 级：需输入元素 id 确认
    const confirm = screen.getByTestId("plan-confirm-button");
    expect(confirm).toBeDisabled();
    await userEvent.type(screen.getByTestId("install-remove-confirm-input"), "agents:claude-code");
    expect(confirm).toBeEnabled();
    await userEvent.click(confirm);

    const applyCall = calls.find((call) => call.command === "agents:apply");
    expect(applyCall!.args).toEqual(["--only", "agents:claude-code", "--action", "uninstall", "--allow-uninstall"]);
    // 不再有任何 policy 预写
    expect(calls.some((call) => call.command === "policy:set")).toBe(false);
  });

  it("取消预览：不写策略、不执行", async () => {
    const { calls } = renderWithEngine(<AgentsPage />, routes(false));
    await userEvent.click(await screen.findByTestId("action-uninstall-agents:claude-code"));
    await screen.findByRole("dialog");
    await userEvent.click(screen.getByRole("button", { name: "取消" }));

    expect(calls.some((call) => call.command === "policy:set")).toBe(false);
    expect(calls.some((call) => call.command === "agents:apply")).toBe(false);
  });

  it("offline：network/install-remove 按钮全部禁用", async () => {
    renderWithEngine(<AgentsPage />, routes(true));
    // offline 来自 system:audit 查询，待其就位后按钮转为禁用
    const upgrade = await screen.findByTestId("action-upgrade-agents:claude-code");
    await waitFor(() => expect(upgrade).toBeDisabled());
    expect(screen.getByTestId("action-uninstall-agents:claude-code")).toBeDisabled();
  });
});

describe("Agents 页文案", () => {
  it("中文动作与安装方式措辞：无误导性 enabled 开关、无官方/推荐断言", async () => {
    renderWithEngine(<AgentsPage />, routes(false));

    await screen.findByTestId("action-upgrade-agents:claude-code");

    // auto_install/auto_update 中文化
    expect(screen.getByText("自动安装")).toBeInTheDocument();
    expect(screen.getByText("自动更新")).toBeInTheDocument();
    expect(screen.queryByText("auto_install")).toBeNull();
    expect(screen.queryByText("auto_update")).toBeNull();

    // preferred_provider 只称默认安装方式；实际来源称当前安装方式；无官方/推荐措辞
    expect(screen.getByText(/默认安装方式 npm/)).toBeInTheDocument();
    expect(screen.getByText(/当前安装方式 npm/)).toBeInTheDocument();
    expect(screen.queryByText(/推荐渠道/)).toBeNull();
    expect(screen.queryByText(/官方渠道/)).toBeNull();

    // 普通 enabled 快捷开关已移除（实际会触发卸载语义），状态仅展示
    expect(screen.queryByTestId("toggle-agents:claude-code")).toBeNull();
    expect(screen.getByText("已启用")).toBeInTheDocument();

    // 明确的安装/升级/卸载与编辑命令入口保留
    expect(screen.getByTestId("action-upgrade-agents:claude-code")).toBeInTheDocument();
    expect(screen.getByTestId("action-uninstall-agents:claude-code")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "编辑命令" })).toBeInTheDocument();
  });

  it("未知来源安装：说明管理限制，不提供误导性的更换安装方式按钮", async () => {
    const unmanagedRoutes = routes(false);
    unmanagedRoutes["agents:status"] = makeEnvelope(
      {
        domain: "agents",
        elements: [
          makeElement({
            id: "agents:codex",
            domain: "agents",
            spec: { display_name: "Codex", bin: "codex", preferred_provider: "native", providers: { native: { managed: true } } },
            policy: { enabled: true },
            observed: { providers: ["unknown-source"], classified: { managed: [], deprecated: [], system_only: [], unsupported: ["unknown-source"] }, installed: true, preferred_active: false },
            state: "unmanaged-detected",
            freshness: makeFreshness(),
            actions: [{ id: "install", risk: "install-remove", needs_lock: true, offline_ok: false }],
          }),
        ],
      },
      { command: "agents.status" },
    );

    renderWithEngine(<AgentsPage />, unmanagedRoutes);

    await screen.findByText(/不在本工具管理范围内/);
    // 不支持自动迁移：不用 install 按钮假装支持
    expect(screen.queryByTestId("migrate-agents:codex")).toBeNull();
    expect(screen.queryByText(/迁移到官方/)).toBeNull();
    expect(screen.queryByText(/更换安装方式/)).toBeNull();
    expect(screen.getByText(/请先自行卸载现有安装/)).toBeInTheDocument();
  });
});
