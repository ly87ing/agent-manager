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
  it("升级按钮 → plan 预览（过滤到本元素）→ network 知情勾选 → agents:apply 带 run_id", async () => {
    const { calls } = renderWithEngine(<AgentsPage />, routes(false));
    await userEvent.click(await screen.findByTestId("action-upgrade-agents:claude-code"));

    const dialog = await screen.findByRole("dialog");
    expect(dialog).toHaveTextContent("升级 Claude Code");
    expect(dialog).toHaveTextContent("npm install -g @anthropic-ai/claude-code");
    // network 级：须勾选
    const confirm = screen.getByTestId("plan-confirm-button");
    expect(confirm).toBeDisabled();
    await userEvent.click(screen.getByTestId("network-ack-checkbox"));
    expect(confirm).toBeEnabled();
    await userEvent.click(confirm);

    const applyCall = calls.find((call) => call.command === "agents:apply");
    expect(applyCall).toBeDefined();
    expect(applyCall!.args).toEqual(["--only", "agents:claude-code"]);
    expect(applyCall!.options?.runId).toMatch(/^gui-/);
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

  it("未知来源安装：提供更换安装方式，不承诺迁移到官方渠道", async () => {
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

    const migrate = await screen.findByTestId("migrate-agents:codex");
    expect(migrate).toHaveTextContent("更换安装方式");
    expect(screen.queryByText(/迁移到官方/)).toBeNull();
    // 不承诺自动迁移现有安装
    expect(screen.getByText(/不会自动卸载或迁移现有安装/)).toBeInTheDocument();
  });
});
