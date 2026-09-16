import { describe, expect, it } from "vitest";
import { waitFor } from "@testing-library/react";
import AgentsPage from "../pages/AgentsPage";
import { makeApplyResult, makeAuditData, makeElement, makeEnvelope, makeFreshness, makeMode, makePolicyData } from "./fixtures";
import { renderWithEngine, screen, userEvent } from "./render-with-engine";
import type { Envelope } from "../lib/contract";

function routes(offline: boolean): Record<string, Envelope> {
  return {
    "system:audit": makeEnvelope(makeAuditData(), { command: "system.audit", mode: makeMode({ offline }) }),
    // policy:get 已加载且 sqlite 层无 agent_definitions 覆盖 → claude-code 为内置定义，安装/升级一键直达
    "policy:get": makeEnvelope(makePolicyData(), { command: "policy.get" }),
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
    "agents:apply": makeEnvelope(makeApplyResult({ domain: "agents" }), { command: "agents.apply" }),
  };
}

describe("Agents 页写动作", () => {
  it("详情展开为纯状态展示（drift 面板），任何动作组合都不发起 plan", async () => {
    const { calls } = renderWithEngine(<AgentsPage />, routes(false));
    await screen.findByTestId("action-upgrade-agents:claude-code");

    await userEvent.click(screen.getByRole("button", { name: "展开同步详情" }));
    expect(await screen.findByTestId("drift-panel")).toBeInTheDocument();
    expect(calls.some((call) => call.command.endsWith(":plan"))).toBe(false);
  });

  it("升级按钮：点击直接 agents:apply 带 --action upgrade 与 run_id，不取 plan、不开弹窗、零 policy:set", async () => {
    const { calls } = renderWithEngine(<AgentsPage />, routes(false));
    await userEvent.click(await screen.findByTestId("action-upgrade-agents:claude-code"));

    await waitFor(() => expect(calls.some((call) => call.command === "agents:apply")).toBe(true));
    const applyCalls = calls.filter((call) => call.command === "agents:apply");
    expect(applyCalls).toHaveLength(1);
    expect(applyCalls[0].args).toEqual(["--only", "agents:claude-code", "--action", "upgrade"]);
    expect(applyCalls[0].args).not.toContain("--dry-run");
    expect(applyCalls[0].options?.runId).toMatch(/^gui-/);
    expect(calls.some((call) => call.command.endsWith(":plan"))).toBe(false);
    expect(calls.some((call) => call.command === "policy:set")).toBe(false);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("卸载：点击先打开 ConfirmModal，取消时零 apply/零 policy:set", async () => {
    const { calls } = renderWithEngine(<AgentsPage />, routes(false));
    await userEvent.click(await screen.findByTestId("action-uninstall-agents:claude-code"));

    await screen.findByRole("dialog", { name: "卸载 Claude Code" });
    expect(screen.getByRole("dialog", { name: "卸载 Claude Code" })).toHaveTextContent(/命令以你的权限运行/);
    expect(screen.getByTestId("confirm-modal-input")).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "取消" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());

    expect(calls.some((call) => call.command === "agents:apply")).toBe(false);
    expect(calls.some((call) => call.command === "policy:set")).toBe(false);
    expect(calls.some((call) => call.command.endsWith(":plan"))).toBe(false);
  });

  it("卸载：输入 element.id 确认后 agents:apply 带 --action uninstall --allow-uninstall，无 plan、零 policy:set", async () => {
    const { calls } = renderWithEngine(<AgentsPage />, routes(false));
    await userEvent.click(await screen.findByTestId("action-uninstall-agents:claude-code"));

    await screen.findByRole("dialog", { name: "卸载 Claude Code" });
    await userEvent.type(screen.getByTestId("confirm-modal-input"), "agents:claude-code");
    await userEvent.click(screen.getByTestId("confirm-modal-confirm"));

    await waitFor(() => expect(calls.some((call) => call.command === "agents:apply")).toBe(true));
    const applyCalls = calls.filter((call) => call.command === "agents:apply");
    expect(applyCalls).toHaveLength(1);
    expect(applyCalls[0].args).toEqual(["--only", "agents:claude-code", "--action", "uninstall", "--allow-uninstall"]);
    expect(applyCalls[0].args).not.toContain("--dry-run");
    expect(applyCalls[0].options?.runId).toMatch(/^gui-/);
    expect(calls.some((call) => call.command.endsWith(":plan"))).toBe(false);
    expect(calls.some((call) => call.command === "policy:set")).toBe(false);
    expect(screen.queryByRole("dialog")).toBeNull();
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

  it("点击管理技能按钮调用 onManageSkills 回调并传递 agentId", async () => {
    let selectedAgent = "";
    renderWithEngine(<AgentsPage onManageSkills={(id) => { selectedAgent = id; }} />, routes(false));

    const manageBtn = await screen.findByTestId("manage-skills-agents:claude-code");
    expect(manageBtn).toBeInTheDocument();
    await userEvent.click(manageBtn);

    expect(selectedAgent).toBe("claude-code");
  });
});

describe("自定义助手的安装/升级确认", () => {
  /** my-agent 的定义写在本地数据库（agent_definitions 覆盖）→ 自定义定义，安装/升级需先确认。 */
  function customRoutes(): Record<string, Envelope> {
    const base = routes(false);
    base["policy:get"] = makeEnvelope(
      makePolicyData({
        layers: {
          sqlite: {
            agent_definitions: {
              "my-agent": { display_name: "My Agent", bin: "my-agent" },
            },
          },
        },
      }),
      { command: "policy.get" },
    );
    base["agents:status"] = makeEnvelope(
      {
        domain: "agents",
        elements: [
          makeElement({
            id: "agents:my-agent",
            domain: "agents",
            spec: { display_name: "My Agent", bin: "my-agent", providers: { custom: { managed: true } } },
            policy: { enabled: true },
            observed: { providers: [], classified: { managed: [], deprecated: [], system_only: [], unsupported: [] }, installed: false },
            state: "missing",
            actions: [
              { id: "install", risk: "install-remove", needs_lock: true, offline_ok: false },
              { id: "upgrade", risk: "network", needs_lock: true, offline_ok: false },
            ],
          }),
        ],
      },
      { command: "agents.status" },
    );
    return base;
  }

  it("内置助手升级维持一键直达：不开弹窗，直接 agents:apply", async () => {
    const { calls } = renderWithEngine(<AgentsPage />, routes(false));
    await userEvent.click(await screen.findByTestId("action-upgrade-agents:claude-code"));

    await waitFor(() => expect(calls.some((call) => call.command === "agents:apply")).toBe(true));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(calls.filter((call) => call.command === "agents:apply")[0].args).toEqual([
      "--only", "agents:claude-code", "--action", "upgrade",
    ]);
  });

  it("自定义助手安装：点击先弹 ConfirmModal（含风险说明），不立即执行", async () => {
    const { calls } = renderWithEngine(<AgentsPage />, customRoutes());
    await userEvent.click(await screen.findByTestId("action-install-agents:my-agent"));

    const dialog = await screen.findByRole("dialog", { name: "安装 My Agent" });
    expect(dialog).toHaveTextContent(/将执行定义中的安装命令，可能写入系统目录/);
    expect(dialog).toHaveTextContent(/命令以你的权限运行，结果不能自动撤销/);
    expect(calls.some((call) => call.command === "agents:apply")).toBe(false);
  });

  it("自定义助手升级：确认后 agents:apply 带 --action upgrade（带 run_id），无 plan、零 policy:set", async () => {
    const { calls } = renderWithEngine(<AgentsPage />, customRoutes());
    await userEvent.click(await screen.findByTestId("action-upgrade-agents:my-agent"));

    await screen.findByRole("dialog", { name: "升级 My Agent" });
    await userEvent.click(screen.getByTestId("confirm-modal-confirm"));

    await waitFor(() => expect(calls.some((call) => call.command === "agents:apply")).toBe(true));
    const applyCalls = calls.filter((call) => call.command === "agents:apply");
    expect(applyCalls).toHaveLength(1);
    expect(applyCalls[0].args).toEqual(["--only", "agents:my-agent", "--action", "upgrade"]);
    expect(applyCalls[0].args).not.toContain("--dry-run");
    expect(applyCalls[0].options?.runId).toMatch(/^gui-/);
    expect(calls.some((call) => call.command.endsWith(":plan"))).toBe(false);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("自定义助手安装：取消弹窗则零 apply、零 policy:set", async () => {
    const { calls } = renderWithEngine(<AgentsPage />, customRoutes());
    await userEvent.click(await screen.findByTestId("action-install-agents:my-agent"));

    await screen.findByRole("dialog", { name: "安装 My Agent" });
    await userEvent.click(screen.getByRole("button", { name: "取消" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());

    expect(calls.some((call) => call.command === "agents:apply")).toBe(false);
    expect(calls.some((call) => call.command === "policy:set")).toBe(false);
  });
});
