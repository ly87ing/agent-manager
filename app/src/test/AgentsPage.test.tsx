import { describe, expect, it } from "vitest";
import { waitFor } from "@testing-library/react";
import AgentsPage from "../pages/AgentsPage";
import { makeApplyResult, makeAuditData, makeElement, makeEnvelope, makeFreshness, makeMode, makePolicyData } from "./fixtures";
import { renderWithEngine, screen, userEvent } from "./render-with-engine";
import type { Envelope } from "../lib/contract";

/** 低频动作（停用、卸载、移除管理）收在卡片的「更多」里：点开才可见。 */
async function openCardMenu(agentId = "agents:claude-code") {
  const trigger = await screen.findByTestId(`agent-menu-${agentId}`);
  // 幂等：已经展开时再点会收起
  if (trigger.getAttribute("aria-expanded") !== "true") await userEvent.click(trigger);
}

function routes(offline: boolean): Record<string, Envelope> {
  return {
    "system:audit": makeEnvelope(makeAuditData(), { command: "system.audit", mode: makeMode({ offline }) }),
    // policy:get 已加载且 sqlite 层无 agents.definitions 覆盖 → claude-code 为内置定义，安装/升级一键直达
    "policy:get": makeEnvelope(makePolicyData(), { command: "policy.get" }),
    "agents:status": makeEnvelope(
      {
        domain: "agents",
        elements: [
          makeElement({
            id: "agents:claude-code",
            domain: "agents",
            spec: { display_name: "Claude Code", bin: "claude", preferred_provider: "npm", providers: { npm: { managed: true } } },
            policy: { enabled: true },
            observed: { install_source: "npm", installed: true },
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
    "sources:get": makeEnvelope(
      {
        sources: [
          {
            key: "aliases.shell",
            label: "Shell Aliases",
            format: "json",
            revision: "rev-1",
            value: { timezone: "America/Los_Angeles", lang: "en_US.UTF-8", aliases: {} },
          },
        ],
      },
      { command: "sources.get" },
    ),
    "sources:set": makeEnvelope({ key: "aliases.shell", revision: "rev-2" }, { command: "sources.set" }),
    "aliases:apply": makeEnvelope(makeApplyResult({ domain: "aliases" }), { command: "aliases.apply" }),
  };
}

/** 已设启动命令的来源变体（与助手可执行名同名的一条）。 */
function routesWithAlias(value: string): Record<string, Envelope> {
  const base = routes(false);
  base["sources:get"] = makeEnvelope(
    {
      sources: [
        {
          key: "aliases.shell",
          label: "Shell Aliases",
          format: "json",
          revision: "rev-1",
          value: { timezone: "America/Los_Angeles", lang: "en_US.UTF-8", aliases: { claude: value } },
        },
      ],
    },
    { command: "sources.get" },
  );
  return base;
}

describe("助手卡片的启动命令", () => {
  it("已设置时展示同步后实际生效的那一行", async () => {
    renderWithEngine(<AgentsPage />, routesWithAlias('TZ="{timezone}" LANG="{lang}" LC_ALL="{lang}" claude --dangerously-skip-permissions'));
    const preview = await screen.findByTestId("alias-preview-agents:claude-code");
    expect(preview).toHaveTextContent(
      `alias claude='TZ="America/Los_Angeles" LANG="en_US.UTF-8" LC_ALL="en_US.UTF-8" claude --dangerously-skip-permissions'`,
    );
  });

  it("未设置时给出提示，且不显示同步按钮", async () => {
    renderWithEngine(<AgentsPage />, routes(false));
    expect(await screen.findByTestId("alias-preview-agents:claude-code")).toHaveTextContent("还没设置");
    expect(screen.queryByTestId("sync-alias-agents:claude-code")).toBeNull();
  });

  it("设置启动命令：预填助手的可执行名，保存只写配置不碰终端文件", async () => {
    const { calls } = renderWithEngine(<AgentsPage />, routes(false));
    await userEvent.click(await screen.findByTestId("edit-alias-agents:claude-code"));
    const dialog = await screen.findByRole("dialog", { name: "新增启动命令" });
    expect(screen.getByTestId("alias-name-input")).toHaveValue("claude");
    expect(screen.getByTestId("alias-command-input")).toHaveValue("claude");
    await userEvent.type(screen.getByTestId("alias-extra-args-input"), "--dangerously-skip-permissions");
    await userEvent.selectOptions(screen.getByTestId("alias-timezone-select"), "Asia/Shanghai");
    await userEvent.click(screen.getByTestId("alias-form-submit"));

    await waitFor(() => expect(calls.some((call) => call.command === "sources:set")).toBe(true));
    const write = calls.find((call) => call.command === "sources:set")!;
    const payload = JSON.parse(write.args[write.args.indexOf("--value") + 1]);
    expect(payload.revision).toBe("rev-1");
    expect(payload.value.aliases.claude).toBe('TZ="Asia/Shanghai" LANG="{lang}" LC_ALL="{lang}" claude --dangerously-skip-permissions');
    expect(calls.some((call) => call.command === "aliases:apply")).toBe(false);
    expect(dialog).toBeDefined();
  });

  it("同步到终端触发 aliases:apply，不影响助手自身动作", async () => {
    const { calls } = renderWithEngine(<AgentsPage />, routesWithAlias("claude --foo"));
    await userEvent.click(await screen.findByTestId("sync-alias-agents:claude-code"));
    await waitFor(() => expect(calls.some((call) => call.command === "aliases:apply")).toBe(true));
    expect(calls.some((call) => call.command === "agents:apply")).toBe(false);
  });
});

describe("Agents 页写动作", () => {
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
    await openCardMenu();
    await userEvent.click(screen.getByTestId("menu-uninstall-agents:claude-code"));

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
    await openCardMenu();
    await userEvent.click(screen.getByTestId("menu-uninstall-agents:claude-code"));

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

});

describe("Agents 页动作执行反馈", () => {
  function pendingApplyRoutes(): { routes: Record<string, Envelope | (() => Promise<Envelope>)>; release: () => void } {
    let release: (() => void) | null = null;
    const pending: Record<string, Envelope | (() => Promise<Envelope>)> = {
      ...routes(false),
      "agents:apply": () =>
        new Promise<Envelope>((resolve) => {
          release = () => resolve(makeEnvelope(makeApplyResult({ domain: "agents" }), { command: "agents.apply" }));
        }),
    };
    return { routes: pending, release: () => release?.() };
  }

  it("升级 label 与按钮 title 携带版本对照（当前 → 最新）", async () => {
    renderWithEngine(<AgentsPage />, routes(false));
    const upgrade = await screen.findByTestId("action-upgrade-agents:claude-code");
    expect(upgrade).toHaveAttribute("title", expect.stringContaining("2.0.0 → 2.1.3"));

    await userEvent.click(upgrade);
    expect(await screen.findByText("升级 Claude Code（2.0.0 → 2.1.3）完成")).toBeInTheDocument();
  });

  it("升级 label 缺版本时用「未知」占位", async () => {
    const unknownRoutes = routes(false);
    unknownRoutes["agents:status"] = makeEnvelope(
      {
        domain: "agents",
        elements: [
          makeElement({
            id: "agents:claude-code",
            domain: "agents",
            spec: { display_name: "Claude Code", bin: "claude", preferred_provider: "npm", providers: { npm: { managed: true } } },
            policy: { enabled: true },
            observed: { install_source: "npm", installed: true },
            state: "managed-ok",
            freshness: makeFreshness({ state: "behind", current: "1.2.3", upstream: null }),
            actions: [{ id: "upgrade", risk: "network", needs_lock: true, offline_ok: false }],
          }),
        ],
      },
      { command: "agents.status" },
    );
    renderWithEngine(<AgentsPage />, unknownRoutes);

    await userEvent.click(await screen.findByTestId("action-upgrade-agents:claude-code"));
    expect(await screen.findByText("升级 Claude Code（1.2.3 → 未知）完成")).toBeInTheDocument();
  });

  it("升级点击后立即进入执行态：按钮转 spinner、同卡其它动作禁用，结束后恢复", async () => {
    const pending = pendingApplyRoutes();
    const { calls } = renderWithEngine(<AgentsPage />, pending.routes);
    const upgrade = await screen.findByTestId("action-upgrade-agents:claude-code");

    await userEvent.click(upgrade);
    await waitFor(() => expect(upgrade).toHaveTextContent("正在执行…"));
    expect(upgrade.querySelector("svg.animate-spin")).not.toBeNull();
    expect(upgrade).toBeDisabled();
    await openCardMenu();
    expect(screen.getByTestId("menu-uninstall-agents:claude-code")).toBeDisabled();

    pending.release();
    await waitFor(() => expect(screen.getByTestId("action-upgrade-agents:claude-code")).toHaveTextContent("升级"));
    await openCardMenu();
    expect(screen.getByTestId("menu-uninstall-agents:claude-code")).toBeEnabled();
    expect(calls.filter((call) => call.command === "agents:apply")).toHaveLength(1);
  });

  it("卸载确认后：弹窗关闭，卡片卸载按钮保持执行态直到执行结束", async () => {
    const pending = pendingApplyRoutes();
    renderWithEngine(<AgentsPage />, pending.routes);
    await openCardMenu();
    await userEvent.click(screen.getByTestId("menu-uninstall-agents:claude-code"));

    await screen.findByRole("dialog", { name: "卸载 Claude Code" });
    await userEvent.type(screen.getByTestId("confirm-modal-input"), "agents:claude-code");
    await userEvent.click(screen.getByTestId("confirm-modal-confirm"));

    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    const uninstall = screen.getByTestId("action-uninstall-agents:claude-code");
    await waitFor(() => expect(uninstall).toHaveTextContent("正在执行…"));
    expect(uninstall.querySelector("svg.animate-spin")).not.toBeNull();
    expect(screen.getByTestId("action-upgrade-agents:claude-code")).toBeDisabled();

    pending.release();
    // 执行结束后卸载按钮退回「更多」里，主行只剩常用动作
    await waitFor(() => expect(screen.queryByTestId("action-uninstall-agents:claude-code")).toBeNull());
    expect(screen.getByTestId("action-upgrade-agents:claude-code")).toBeEnabled();
  });
});

describe("Agents 页文案", () => {
  it("卡片只留能改变下一步动作的信息：不重复安装渠道、不显示恒真的偏好与状态", async () => {
    renderWithEngine(<AgentsPage />, routes(false));

    await screen.findByTestId("action-upgrade-agents:claude-code");

    // 安装渠道三个徽标说的是同一件事（且没有切换入口），全部撤下；异常来源仍由警告框说明
    expect(screen.queryByText(/默认安装方式/)).toBeNull();
    expect(screen.queryByText(/当前安装方式/)).toBeNull();
    expect(screen.queryByText(/由此工具管理/)).toBeNull();

    // 引擎从不读 auto_install/auto_update，界面不再假装它们是开关
    expect(screen.queryByText("自动安装")).toBeNull();
    expect(screen.queryByText("自动更新")).toBeNull();

    // 启用是常态，不占位；「正常」同理，只有不正常时才出徽标
    expect(screen.queryByText("已启用")).toBeNull();
    expect(screen.queryByText("正常")).toBeNull();

    // 常用动作留在主行，破坏性动作收进「更多」
    expect(screen.getByTestId("action-upgrade-agents:claude-code")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "编辑命令" })).toBeInTheDocument();
    expect(screen.queryByTestId("action-uninstall-agents:claude-code")).toBeNull();
    await openCardMenu();
    expect(screen.getByTestId("menu-uninstall-agents:claude-code")).toBeInTheDocument();
    expect(screen.getByTestId("policy-enabled-agents:claude-code")).toHaveTextContent("停用");
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
            observed: { install_source: "unknown-source", installed: true },
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
  /** my-agent 的定义写在本地数据库（agents.definitions 覆盖）→ 自定义定义，安装/升级需先确认。 */
  function customRoutes(): Record<string, Envelope> {
    const base = routes(false);
    base["policy:get"] = makeEnvelope(
      makePolicyData({
        layers: {
          sqlite: {
            agents: {
              definitions: {
                "my-agent": { display_name: "My Agent", bin: "my-agent" },
              },
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
            observed: { install_source: null, installed: false },
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

  it("自定义助手升级：确认后 apply label 携带版本对照", async () => {
    const custom = customRoutes();
    custom["agents:status"] = makeEnvelope(
      {
        domain: "agents",
        elements: [
          makeElement({
            id: "agents:my-agent",
            domain: "agents",
            spec: { display_name: "My Agent", bin: "my-agent", providers: { custom: { managed: true } } },
            policy: { enabled: true },
            observed: { install_source: "custom", installed: true },
            state: "managed-ok",
            freshness: makeFreshness({ state: "behind", current: "0.9.0", upstream: "1.0.0" }),
            actions: [{ id: "upgrade", risk: "network", needs_lock: true, offline_ok: false }],
          }),
        ],
      },
      { command: "agents.status" },
    );
    renderWithEngine(<AgentsPage />, custom);

    await userEvent.click(await screen.findByTestId("action-upgrade-agents:my-agent"));
    await screen.findByRole("dialog", { name: "升级 My Agent" });
    await userEvent.click(screen.getByTestId("confirm-modal-confirm"));

    expect(await screen.findByText("升级 My Agent（0.9.0 → 1.0.0）完成")).toBeInTheDocument();
  });
});
