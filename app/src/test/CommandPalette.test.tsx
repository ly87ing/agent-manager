import { beforeEach, describe, expect, it, onTestFinished, vi } from "vitest";
import { waitFor } from "@testing-library/react";
import App from "../App";
import type { Envelope, SettingsData } from "../lib/contract";
import { setSettingsClient } from "../lib/settings";
import { agentManagementFixture, makeAliasStatus, makeApplyResult, makeAuditData, makeElement, makeEnvelope, makeFreshness, makeMode, makePolicyData } from "./fixtures";
import { renderWithEngine, screen, userEvent, within } from "./render-with-engine";

const trayEvents = vi.hoisted(() => ({
  listeners: new Set<(event: { payload: unknown }) => void>(),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (_event: string, listener: (event: { payload: unknown }) => void) => {
    trayEvents.listeners.add(listener);
    return () => trayEvents.listeners.delete(listener);
  }),
}));

const dummySettings: SettingsData = {
  appVersion: "0.1.0",
  contractVersion: "v1",
  resourceDir: "/tmp/resources",
  dataDir: "/tmp/data",
  updaterConfigured: false,
  preferences: { logRetentionDays: 30 },
};

type TestRoute = Envelope | ((args: string[]) => Promise<Envelope>);

const BASE = "d".repeat(64);

/** active 是库里的上次写入记录；actual 是客户端实际配置——当前接入只看后者。 */
function agentsStatus(active = "", actual = active) {
  const actualConnection = actual === "kimi"
    ? {
        base_url: "https://api.moonshot.cn/anthropic",
        model: "kimi-k3[1m]",
        auth: "token",
      }
    : null;
  return makeEnvelope(
    {
      domain: "agents",
      elements: [
        makeElement({
          id: "agents:claude-code",
          domain: "agents",
          spec: {
            display_name: "Claude Code",
            bin: "claude",
            settings: {
              active,
              profiles: {
                kimi: {
                  label: "Kimi Code",
                  note: "月度套餐备用线路",
                  base_url: "https://api.moonshot.cn/anthropic",
                  token_var: "KIMI_API_KEY",
                  model: "kimi-k3[1m]",
                },
              },
            },
          },
          policy: { enabled: true },
          observed: {
            install_source: null,
            installed: true,
            management: agentManagementFixture("claude"),
            current_config: {
              state: "ready",
              file: "settings.json",
              config: {
                provider: "anthropic",
                base_url: actualConnection?.base_url ?? null,
                model: actualConnection?.model ?? null,
                small_fast_model: null,
                model_aliases: { opus: null, sonnet: null, haiku: null },
                timeout_ms: null,
                credential: {
                  configured: actualConnection !== null,
                  kind: actualConnection?.auth ?? null,
                  variable: null,
                },
                runtime: {},
              },
              base: BASE,
              current: {
                profile: actual === "kimi" ? "kimi" : null,
                default: actual !== "kimi",
                last_written: active || null,
                unverified: [],
                drifted_from: null,
              },
            },
          },
          state: "managed-ok",
          freshness: makeFreshness(),
          actions: [],
        }),
      ],
    },
    { command: "agents.status" },
  );
}

function routes(): Record<string, TestRoute> {
  return {
    "system:audit": makeEnvelope(makeAuditData(), { command: "system.audit", mode: makeMode() }),
    "secrets:status": makeEnvelope(
      {
        domain: "secrets",
        elements: [
          makeElement({
            id: "secrets:mcp-env",
            domain: "secrets",
            observed: { install_source: null, variables: [], unresolved: 0 },
            state: "managed-ok",
          }),
        ],
      },
      { command: "secrets.status" },
    ),
    "agents:status": agentsStatus(),
    "skills:status": makeEnvelope(
      {
        domain: "skills",
        elements: [
          makeElement({
            id: "skill:agent-skills",
            domain: "skills",
            spec: { url: "https://example.com/x.git" },
            observed: { install_source: null, repo_cached: true, discovered: [], discovery_inbox: [] },
            state: "managed-ok",
          }),
        ],
      },
      { command: "skills.status" },
    ),
    "mcp:status": makeEnvelope(
      {
        domain: "mcp",
        elements: [
          makeElement({
            id: "mcp:grok-search",
            domain: "mcp",
            spec: { command: "uvx", args: [] },
            observed: { install_source: null, agents: {}, env: {}, actions: [] },
            state: "managed-ok",
          }),
        ],
      },
      { command: "mcp.status" },
    ),
    "profiles:status": makeEnvelope({ domain: "profiles", elements: [] }, { command: "profiles.status" }),
    "aliases:status": makeEnvelope(makeAliasStatus({}), { command: "aliases.status" }),
    "rules:status": makeEnvelope({ domain: "rules", elements: [] }, { command: "rules.status" }),
    "system:discover": makeEnvelope(
      { domain: "discover", agents: [], skills: { items: [], groups: [] }, mcp: [], summary: {} },
      { command: "system.discover" },
    ),
    "runs:list": makeEnvelope({ runs: [] }, { command: "runs.list" }),
    "policy:get": makeEnvelope(makePolicyData({ layers: { sqlite: null } }), { command: "policy.get" }),
    "aliases:apply": makeEnvelope(makeApplyResult({ domain: "aliases" }), { command: "aliases.apply" }),
  };
}

async function openPalette() {
  await userEvent.keyboard("{Meta>}k{/Meta}");
  return screen.findByTestId("command-palette");
}

async function openAccountAndEnvironmentPalette() {
  await userEvent.keyboard("{Meta>}{Shift>}k{/Shift}{/Meta}");
  return screen.findByRole("dialog", { name: "账号与环境" });
}

describe("命令面板", () => {
  beforeEach(() => trayEvents.listeners.clear());

  it("⌘K 打开后能搜到资源，选中跳到对应页面", async () => {
    setSettingsClient({ get: async () => dummySettings, save: async (preferences) => ({ ...dummySettings, preferences }) });
    renderWithEngine(<App />, routes());
    await screen.findByRole("heading", { name: "助手" }, { timeout: 10_000 });

    const palette = await openPalette();
    await userEvent.type(screen.getByTestId("command-palette-input"), "grok");
    await waitFor(() => expect(screen.getByTestId("command-item-mcp:grok-search")).toBeInTheDocument());
    await userEvent.click(screen.getByTestId("command-item-mcp:grok-search"));

    expect(await screen.findByRole("heading", { name: "MCP" }, { timeout: 10_000 })).toBeInTheDocument();
    expect(palette).not.toBeInTheDocument();
  });

  it("⌘⇧K 只列官方账号与独立环境，不列主配置第三方接入或其他资源", async () => {
    setSettingsClient({ get: async () => dummySettings, save: async (preferences) => ({ ...dummySettings, preferences }) });
    renderWithEngine(<App />, routes());
    await screen.findByRole("heading", { name: "助手" }, { timeout: 10_000 });

    const switcher = await openAccountAndEnvironmentPalette();
    expect(within(switcher).getByTestId("command-item-connection:claude-code:default")).toHaveTextContent("Claude Code · 官方账号");
    expect(within(switcher).queryByTestId("command-item-connection:claude-code:kimi")).toBeNull();
    expect(within(switcher).queryByTestId("command-item-page:agents")).toBeNull();
    expect(within(switcher).queryByTestId("command-item-action:apply-rules")).toBeNull();

    const input = within(switcher).getByRole("textbox", { name: "搜索账号与环境" });
    await userEvent.type(input, "月度套餐");
    expect(within(switcher).queryByTestId("command-item-connection:claude-code:default")).toBeNull();

    await userEvent.keyboard("{Escape}");
    await openPalette();
    expect(screen.getByTestId("command-item-page:agents")).toBeInTheDocument();
  });

  it("⌘⇧K 按启动名搜索独立环境、复制启动名并直达卡片，全程不写客户端配置", async () => {
    setSettingsClient({ get: async () => dummySettings, save: async (preferences) => ({ ...dummySettings, preferences }) });
    const writeText = vi.fn(async () => {});
    const originalClipboard = navigator.clipboard;
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText } });
    onTestFinished(() => { Object.defineProperty(navigator, "clipboard", { configurable: true, value: originalClipboard }); });
    const testRoutes = routes();
    testRoutes["profiles:status"] = makeEnvelope({
      domain: "profiles",
      elements: [makeElement({
        id: "profiles:cc-work",
        domain: "profiles",
        spec: { display_name: "工作账号", base_agent: "claude-code", kind: "claude", config_dir: "~/.claude-work" },
        observed: { config_dir: "~/.claude-work", config_dir_exists: true, fields: [], blocked_reason: null },
        state: "managed-ok",
        actions: [],
      })],
    }, { command: "profiles.status" });
    testRoutes["aliases:status"] = makeEnvelope(makeAliasStatus({ "cc-work": 'CLAUDE_CONFIG_DIR="$HOME/.claude-work" claude' }), { command: "aliases.status" });
    const { calls } = renderWithEngine(<App />, testRoutes);
    await screen.findByRole("heading", { name: "助手" }, { timeout: 10_000 });
    await openAccountAndEnvironmentPalette();
    await userEvent.type(screen.getByTestId("command-palette-input"), "cc-work");
    await userEvent.click(await screen.findByTestId("command-item-profile-launch:cc-work:cc-work"));
    await waitFor(() => expect(writeText).toHaveBeenCalledWith("cc-work"));
    await openAccountAndEnvironmentPalette();
    await userEvent.type(screen.getByTestId("command-palette-input"), "工作账号");
    await userEvent.click(await screen.findByTestId("command-item-profile:cc-work"));
    const card = await screen.findByTestId("profile-card-cc-work");
    await waitFor(() => expect(card).toHaveFocus());
    expect(calls.some((call) => call.command === "agents:configure" || call.command.endsWith(":apply"))).toBe(false);
  });

  it("托盘独立环境项只打开对应命令或卡片，不切换当前助手配置", async () => {
    setSettingsClient({ get: async () => dummySettings, save: async (preferences) => ({ ...dummySettings, preferences }) });
    const testRoutes = routes();
    testRoutes["profiles:status"] = makeEnvelope({
      domain: "profiles",
      elements: [makeElement({
        id: "profiles:cc-work",
        domain: "profiles",
        spec: { display_name: "工作账号", base_agent: "claude-code", kind: "claude", config_dir: "~/.claude-work" },
        observed: { config_dir: "~/.claude-work", config_dir_exists: true, fields: [], blocked_reason: null },
        state: "managed-ok",
        actions: [],
      })],
    }, { command: "profiles.status" });
    testRoutes["aliases:status"] = makeEnvelope(makeAliasStatus({ "cc-work": 'CLAUDE_CONFIG_DIR="$HOME/.claude-work" claude' }), { command: "aliases.status" });
    const { calls } = renderWithEngine(<App />, testRoutes);
    await screen.findByRole("heading", { name: "助手" }, { timeout: 10_000 });
    await waitFor(() => expect(trayEvents.listeners.size).toBe(1));
    for (const listener of trayEvents.listeners) {
      listener({ payload: {
        type: "open-environment",
        agentId: "claude-code", agentName: "Claude Code",
        environmentId: "cc-work", environmentName: "工作账号", launchName: "cc-work",
      } });
    }
    const palette = await screen.findByRole("dialog", { name: "命令面板" });
    expect(within(palette).getByTestId("command-item-profile-launch:cc-work:cc-work")).toBeInTheDocument();
    expect(within(palette).queryByTestId("command-item-connection:claude-code:default")).toBeNull();
    await userEvent.keyboard("{Escape}");
    for (const listener of trayEvents.listeners) {
      listener({ payload: {
        type: "open-environment",
        agentId: "claude-code", agentName: "Claude Code",
        environmentId: "cc-work", environmentName: "工作账号", launchName: null,
      } });
    }
    expect(within(await screen.findByRole("dialog", { name: "命令面板" })).getByTestId("command-item-profile:cc-work")).toBeInTheDocument();
    expect(calls.some((call) => call.command === "agents:configure" || call.command.endsWith(":apply"))).toBe(false);
  });

  it("未写入和缺启动命令的环境只提供定位入口，不误示为可启动", async () => {
    setSettingsClient({ get: async () => dummySettings, save: async (preferences) => ({ ...dummySettings, preferences }) });
    const testRoutes = routes();
    testRoutes["profiles:status"] = makeEnvelope({
      domain: "profiles",
      elements: [
        makeElement({
          id: "profiles:pending",
          domain: "profiles",
          spec: { display_name: "待写入", base_agent: "claude-code", kind: "claude", config_dir: "~/.claude-pending" },
          observed: { config_dir: "~/.claude-pending", config_dir_exists: false },
          state: "not-materialized",
        }),
        makeElement({
          id: "profiles:no-launch",
          domain: "profiles",
          spec: { display_name: "缺命令", base_agent: "claude-code", kind: "claude", config_dir: "~/.claude-no-launch" },
          observed: { config_dir: "~/.claude-no-launch", config_dir_exists: true },
          state: "managed-ok",
        }),
      ],
    }, { command: "profiles.status" });
    testRoutes["aliases:status"] = makeEnvelope(makeAliasStatus({ pending: 'CLAUDE_CONFIG_DIR="$HOME/.claude-pending" claude' }), { command: "aliases.status" });
    renderWithEngine(<App />, testRoutes);
    await screen.findByRole("heading", { name: "助手" }, { timeout: 10_000 });
    await openAccountAndEnvironmentPalette();
    await userEvent.type(screen.getByTestId("command-palette-input"), "待写入");
    expect(await screen.findByTestId("command-item-profile:pending")).toHaveTextContent("配置尚未可用");
    expect(screen.queryByTestId("command-item-profile-launch:pending:pending")).toBeNull();
    await userEvent.clear(screen.getByTestId("command-palette-input"));
    await userEvent.type(screen.getByTestId("command-palette-input"), "缺命令");
    expect(await screen.findByTestId("command-item-profile:no-launch")).toHaveTextContent("还没有启动命令");
    expect(screen.queryByTestId("command-item-profile-launch:no-launch:no-launch")).toBeNull();
  });

  it("规则与终端快捷命令都以实际文件为准、逐条编辑，没有一键同步动作", async () => {
    setSettingsClient({ get: async () => dummySettings, save: async (preferences) => ({ ...dummySettings, preferences }) });
    const { calls } = renderWithEngine(<App />, routes());
    await screen.findByRole("heading", { name: "助手" }, { timeout: 10_000 });

    await openPalette();
    await userEvent.type(screen.getByTestId("command-palette-input"), "同步规则");
    expect(screen.queryByTestId("command-item-action:apply-rules")).toBeNull();
    await userEvent.clear(screen.getByTestId("command-palette-input"));
    // 快捷命令每次编辑都直接写回 rc 文件，没有「同步」可做（执行通道见下方官方账号用例）
    await userEvent.type(screen.getByTestId("command-palette-input"), "同步终端快捷命令");
    expect(screen.queryByTestId("command-item-action:apply-aliases")).toBeNull();
    expect(calls.some((call) => call.command === "aliases:apply")).toBe(false);
    expect(calls.some((call) => call.command === "rules:apply")).toBe(false);
  });

  it("键盘可完成：方向键选择、回车执行、Esc 关闭", async () => {
    setSettingsClient({ get: async () => dummySettings, save: async (preferences) => ({ ...dummySettings, preferences }) });
    renderWithEngine(<App />, routes());
    await screen.findByRole("heading", { name: "助手" }, { timeout: 10_000 });

    await openPalette();
    await userEvent.type(screen.getByTestId("command-palette-input"), "接管");
    expect(screen.getAllByTestId("command-item-page:takeover")).toHaveLength(1);
    expect(screen.queryByTestId("command-item-action:takeover")).toBeNull();
    await userEvent.keyboard("{Enter}");
    expect(await screen.findByRole("heading", { name: "导入现有配置" }, { timeout: 10_000 })).toBeInTheDocument();

    const palette = await openPalette();
    await userEvent.keyboard("{Escape}");
    await waitFor(() => expect(palette).not.toBeInTheDocument());
  });

  it("无匹配时给出空态，不误触发任何动作", async () => {
    setSettingsClient({ get: async () => dummySettings, save: async (preferences) => ({ ...dummySettings, preferences }) });
    const { calls } = renderWithEngine(<App />, routes());
    await screen.findByRole("heading", { name: "助手" }, { timeout: 10_000 });

    await openPalette();
    await userEvent.type(screen.getByTestId("command-palette-input"), "zzzz-not-exist");
    expect(await screen.findByText("没有匹配的资源或动作。")).toBeInTheDocument();
    await userEvent.keyboard("{Enter}");
    expect(calls.some((call) => call.command.endsWith(":apply"))).toBe(false);
  });

  it("设置里的二级页也搜得到，选中直达（密钥要填东西，不该只藏在二级菜单）", async () => {
    setSettingsClient({ get: async () => dummySettings, save: async (preferences) => ({ ...dummySettings, preferences }) });
    renderWithEngine(<App />, routes());
    await screen.findByRole("heading", { name: "助手" }, { timeout: 10_000 });

    await openPalette();
    await userEvent.type(screen.getByTestId("command-palette-input"), "密钥");
    await userEvent.click(await screen.findByTestId("command-item-page:settings:secrets"));

    expect(await screen.findByRole("heading", { name: "密钥" }, { timeout: 10_000 })).toBeInTheDocument();
  });

  it("主配置里有第三方接入时一步切回官方账号，成功必须经过状态回读且带操作记录 id", async () => {
    setSettingsClient({ get: async () => dummySettings, save: async (preferences) => ({ ...dummySettings, preferences }) });
    let actual = "kimi";
    const testRoutes = routes();
    testRoutes["agents:status"] = async () => agentsStatus("kimi", actual);
    testRoutes["agents:configure"] = async (args) => {
      if (args.includes("--use-default")) actual = "";
      return makeEnvelope(makeApplyResult({ domain: "agents" }), { command: "agents.configure" });
    };
    const { calls } = renderWithEngine(<App />, testRoutes);
    await screen.findByRole("heading", { name: "助手" }, { timeout: 10_000 });

    await openPalette();
    await userEvent.type(screen.getByTestId("command-palette-input"), "官方账号");
    const official = await screen.findByTestId("command-item-connection:claude-code:default");
    expect(official).toHaveTextContent("切回官方账号");
    await userEvent.click(official);

    await screen.findByText("已切换 Claude Code 到 官方账号");
    const configure = calls.find((call) => call.command === "agents:configure");
    expect(configure?.args).toEqual(["--only", "agents:claude-code", "--base", BASE, "--use-default"]);
    expect(configure?.options?.runId).toMatch(/^gui-/);
    expect(calls.filter((call) => call.command === "agents:status").length).toBeGreaterThanOrEqual(2);

    await openPalette();
    await userEvent.type(screen.getByTestId("command-palette-input"), "官方账号");
    const current = await screen.findByTestId("command-item-connection:claude-code:default");
    expect(current).toBeDisabled();
    expect(current).toHaveTextContent("当前使用");
  });

  it("命令面板不再提供切到第三方接入：按接入名或备注都搜不到切换项", async () => {
    setSettingsClient({ get: async () => dummySettings, save: async (preferences) => ({ ...dummySettings, preferences }) });
    renderWithEngine(<App />, routes());
    await screen.findByRole("heading", { name: "助手" }, { timeout: 10_000 });

    await openPalette();
    await userEvent.type(screen.getByTestId("command-palette-input"), "Kimi Code");
    expect(screen.queryByTestId("command-item-connection:claude-code:kimi")).toBeNull();
    await userEvent.clear(screen.getByTestId("command-palette-input"));
    await userEvent.type(screen.getByTestId("command-palette-input"), "月度套餐");
    expect(screen.queryByTestId("command-item-connection:claude-code:kimi")).toBeNull();
  });

  it("原生托盘的官方账号事件唤回精确目标，确认后走同一条切回与双回读路径", async () => {
    setSettingsClient({ get: async () => dummySettings, save: async (preferences) => ({ ...dummySettings, preferences }) });
    let actual = "kimi";
    const testRoutes = routes();
    testRoutes["agents:status"] = async () => agentsStatus("kimi", actual);
    testRoutes["agents:configure"] = async (args) => {
      if (args.includes("--use-default")) actual = "";
      return makeEnvelope(makeApplyResult({ domain: "agents" }), { command: "agents.configure" });
    };
    const { calls } = renderWithEngine(<App />, testRoutes);
    await screen.findByRole("heading", { name: "助手" }, { timeout: 10_000 });
    await waitFor(() => expect(trayEvents.listeners.size).toBe(1));

    for (const listener of trayEvents.listeners) {
      listener({
        payload: {
          type: "switch-connection",
          agentId: "claude-code",
          agentName: "Claude Code",
          profile: "",
          profileLabel: "官方账号",
        },
      });
    }

    expect(await screen.findByTestId("command-palette")).toBeInTheDocument();
    expect(await screen.findByTestId("command-item-connection:claude-code:default")).toBeInTheDocument();
    expect(calls.some((call) => call.command === "agents:configure")).toBe(false);
    await userEvent.keyboard("{Enter}");

    await screen.findByText("已切换 Claude Code 到 官方账号");
    expect(calls.find((call) => call.command === "agents:configure")?.args).toEqual([
      "--only",
      "agents:claude-code",
      "--base",
      BASE,
      "--use-default",
    ]);
    expect(calls.filter((call) => call.command === "agents:status").length).toBeGreaterThanOrEqual(2);
  });

  it("托盘切回官方遇到还没保存的实际令牌时不静默丢弃，转入可见确认后才带放弃标志重试", async () => {
    setSettingsClient({ get: async () => dummySettings, save: async (preferences) => ({ ...dummySettings, preferences }) });
    let actual = "kimi";
    const testRoutes = routes();
    testRoutes["agents:status"] = async () => agentsStatus("", actual);
    testRoutes["agents:configure"] = async (args) => {
      if (!args.includes("--discard-live-token")) {
        return makeEnvelope(
          {},
          {
            command: "agents.configure",
            ok: false,
            errors: [{ code: "AGENT_LIVE_TOKEN_UNSAVED", element: "agents:claude-code", message: "当前实际配置里的令牌还没有保存在任何变量里，切换会丢失它：先到“设置 → 密钥”选择外部密钥文件（令牌会写进该文件，库里只存变量名），或确认放弃这个令牌后带 --discard-live-token 重试" }],
          },
        );
      }
      actual = "";
      return makeEnvelope(makeApplyResult({ domain: "agents" }), { command: "agents.configure" });
    };
    const { calls } = renderWithEngine(<App />, testRoutes);
    await screen.findByRole("heading", { name: "助手" }, { timeout: 10_000 });
    await waitFor(() => expect(trayEvents.listeners.size).toBe(1));

    for (const listener of trayEvents.listeners) {
      listener({
        payload: {
          type: "switch-connection",
          agentId: "claude-code",
          agentName: "Claude Code",
          profile: "",
          profileLabel: "官方账号",
        },
      });
    }

    expect(await screen.findByTestId("command-palette")).toBeInTheDocument();
    await userEvent.keyboard("{Enter}");
    const confirm = await screen.findByRole("dialog", { name: "当前实际令牌还没有保存" });
    expect(within(confirm).getByTestId("confirm-modal-secondary")).toHaveTextContent("去选择密钥文件");
    expect(confirm).toHaveTextContent("设置 → 密钥");
    expect(confirm).not.toHaveTextContent("--discard-live-token");
    await userEvent.click(within(confirm).getByTestId("confirm-modal-confirm"));
    expect(await screen.findByText("已切换 Claude Code 到 官方账号")).toBeInTheDocument();
    expect(calls.filter((call) => call.command === "agents:configure")[1]?.args).toEqual([
      "--only",
      "agents:claude-code",
      "--base",
      BASE,
      "--use-default",
      "--discard-live-token",
    ]);
  });

  it("切回官方账号使用 use-default，不复写不存在的 profile", async () => {
    setSettingsClient({ get: async () => dummySettings, save: async (preferences) => ({ ...dummySettings, preferences }) });
    let active = "kimi";
    const testRoutes = routes();
    testRoutes["agents:status"] = async () => agentsStatus(active);
    testRoutes["agents:configure"] = async (args) => {
      if (args.includes("--use-default")) active = "";
      return makeEnvelope(makeApplyResult({ domain: "agents" }), { command: "agents.configure" });
    };
    const { calls } = renderWithEngine(<App />, testRoutes);
    await screen.findByRole("heading", { name: "助手" }, { timeout: 10_000 });

    await openPalette();
    await userEvent.type(screen.getByTestId("command-palette-input"), "官方账号");
    await userEvent.click(await screen.findByTestId("command-item-connection:claude-code:default"));

    await screen.findByText("已切换 Claude Code 到 官方账号");
    expect(calls.find((call) => call.command === "agents:configure")?.args).toEqual([
      "--only",
      "agents:claude-code",
      "--base",
      BASE,
      "--use-default",
    ]);
  });

  it("库里的记录变了但客户端实际配置没变时不误报成功", async () => {
    setSettingsClient({ get: async () => dummySettings, save: async (preferences) => ({ ...dummySettings, preferences }) });
    let active = "kimi";
    const testRoutes = routes();
    testRoutes["agents:status"] = async () => agentsStatus(active, "kimi");
    testRoutes["agents:configure"] = async () => {
      active = "";
      return makeEnvelope(makeApplyResult({ domain: "agents" }), { command: "agents.configure" });
    };
    renderWithEngine(<App />, testRoutes);
    await screen.findByRole("heading", { name: "助手" }, { timeout: 10_000 });

    await openPalette();
    await userEvent.type(screen.getByTestId("command-palette-input"), "官方账号");
    await userEvent.click(await screen.findByTestId("command-item-connection:claude-code:default"));

    expect(await screen.findByText("写入已经返回成功，但状态回读没有确认接入生效，请在助手页检查")).toBeInTheDocument();
    expect(screen.queryByText("已切换 Claude Code 到 官方账号")).not.toBeInTheDocument();
  });

  it("库里记着 kimi 但实际已是官方账号时按实际显示为当前使用", async () => {
    setSettingsClient({ get: async () => dummySettings, save: async (preferences) => ({ ...dummySettings, preferences }) });
    const testRoutes = routes();
    testRoutes["agents:status"] = agentsStatus("kimi", "");
    const { calls } = renderWithEngine(<App />, testRoutes);
    await screen.findByRole("heading", { name: "助手" }, { timeout: 10_000 });

    await openPalette();
    await userEvent.type(screen.getByTestId("command-palette-input"), "官方账号");
    const current = await screen.findByTestId("command-item-connection:claude-code:default");
    expect(current).toBeDisabled();
    expect(current).toHaveTextContent("当前使用");
    expect(calls.some((call) => call.command === "agents:configure")).toBe(false);
  });

  it("外部修改时不冒充成功：刷新后提示重新选择，不再提供用库覆盖", async () => {
    setSettingsClient({ get: async () => dummySettings, save: async (preferences) => ({ ...dummySettings, preferences }) });
    let attempts = 0;
    const testRoutes = routes();
    testRoutes["agents:status"] = agentsStatus("kimi", "kimi");
    testRoutes["agents:configure"] = async () => {
      attempts += 1;
      return makeEnvelope(
        {},
        {
          command: "agents.configure",
          ok: false,
          errors: [{ code: "AGENT_CONFIG_CHANGED", element: "agents:claude-code", message: "settings.json 已在外部修改（与读取时不一致），文件与库都没有改动；请刷新后重试" }],
        },
      );
    };
    const { calls } = renderWithEngine(<App />, testRoutes);
    await screen.findByRole("heading", { name: "助手" }, { timeout: 10_000 });

    await openPalette();
    await userEvent.type(screen.getByTestId("command-palette-input"), "官方账号");
    const statusBefore = calls.filter((call) => call.command === "agents:status").length;
    await userEvent.click(await screen.findByTestId("command-item-connection:claude-code:default"));

    expect(await screen.findByText("配置文件已在外部修改，接入没有切换")).toBeInTheDocument();
    expect(screen.queryByText("已切换 Claude Code 到 官方账号")).not.toBeInTheDocument();
    expect(screen.queryByRole("dialog", { name: "覆盖被改过的接入配置" })).not.toBeInTheDocument();
    expect(attempts).toBe(1);
    expect(calls.filter((call) => call.command === "agents:status").length).toBeGreaterThan(statusBefore);
  });

  it("配置被拒绝时保留真实状态并说明失败，不弹放弃确认", async () => {
    setSettingsClient({ get: async () => dummySettings, save: async (preferences) => ({ ...dummySettings, preferences }) });
    const testRoutes = routes();
    testRoutes["agents:status"] = agentsStatus("kimi", "kimi");
    testRoutes["agents:configure"] = async () =>
      makeEnvelope(
        {},
        {
          command: "agents.configure",
          ok: false,
          errors: [{ code: "MISSING_SECRET", element: "agents:claude-code", message: "凭据变量 KIMI_API_KEY 未配置" }],
        },
      );
    renderWithEngine(<App />, testRoutes);
    await screen.findByRole("heading", { name: "助手" }, { timeout: 10_000 });

    await openPalette();
    await userEvent.type(screen.getByTestId("command-palette-input"), "官方账号");
    await userEvent.click(await screen.findByTestId("command-item-connection:claude-code:default"));

    expect(await screen.findByText("接入没有切换：凭据变量 KIMI_API_KEY 未配置")).toBeInTheDocument();
    expect(screen.queryByRole("dialog", { name: "当前实际令牌还没有保存" })).not.toBeInTheDocument();
  });
});
