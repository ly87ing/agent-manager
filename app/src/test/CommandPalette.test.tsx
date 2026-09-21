import { beforeEach, describe, expect, it, vi } from "vitest";
import { waitFor } from "@testing-library/react";
import App from "../App";
import type { Envelope, SettingsData } from "../lib/contract";
import { setSettingsClient } from "../lib/settings";
import { agentManagementFixture, makeApplyResult, makeAuditData, makeElement, makeEnvelope, makeFreshness, makeMode, makePolicyData } from "./fixtures";
import { renderWithEngine, screen, userEvent } from "./render-with-engine";

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
    "rules:status": makeEnvelope({ domain: "rules", elements: [] }, { command: "rules.status" }),
    "system:discover": makeEnvelope(
      { domain: "discover", agents: [], skills: { items: [], groups: [] }, mcp: [], summary: {} },
      { command: "system.discover" },
    ),
    "runs:list": makeEnvelope({ runs: [] }, { command: "runs.list" }),
    "policy:get": makeEnvelope(makePolicyData({ layers: { sqlite: null } }), { command: "policy.get" }),
    "sources:get": makeEnvelope({ sources: [] }, { command: "sources.get" }),
    "rules:apply": makeEnvelope(makeApplyResult({ domain: "rules" }), { command: "rules.apply" }),
    "aliases:apply": makeEnvelope(makeApplyResult({ domain: "aliases" }), { command: "aliases.apply" }),
  };
}

async function openPalette() {
  await userEvent.keyboard("{Meta>}k{/Meta}");
  return screen.findByTestId("command-palette");
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

  it("可直接执行常用动作，走与页面相同的执行通道", async () => {
    setSettingsClient({ get: async () => dummySettings, save: async (preferences) => ({ ...dummySettings, preferences }) });
    const { calls } = renderWithEngine(<App />, routes());
    await screen.findByRole("heading", { name: "助手" }, { timeout: 10_000 });

    await openPalette();
    await userEvent.type(screen.getByTestId("command-palette-input"), "同步规则");
    await userEvent.click(await screen.findByTestId("command-item-action:apply-rules"));

    await waitFor(() => expect(calls.some((call) => call.command === "rules:apply")).toBe(true));
    expect(calls.find((call) => call.command === "rules:apply")?.options?.runId).toMatch(/^gui-/);
  });

  it("键盘可完成：方向键选择、回车执行、Esc 关闭", async () => {
    setSettingsClient({ get: async () => dummySettings, save: async (preferences) => ({ ...dummySettings, preferences }) });
    renderWithEngine(<App />, routes());
    await screen.findByRole("heading", { name: "助手" }, { timeout: 10_000 });

    await openPalette();
    await userEvent.type(screen.getByTestId("command-palette-input"), "接管");
    await userEvent.keyboard("{Enter}");
    expect(await screen.findByRole("heading", { name: "接管中心" }, { timeout: 10_000 })).toBeInTheDocument();

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

  it("能搜索接入并一步切换，成功必须经过状态回读且带操作记录 id", async () => {
    setSettingsClient({ get: async () => dummySettings, save: async (preferences) => ({ ...dummySettings, preferences }) });
    let active = "";
    const testRoutes = routes();
    testRoutes["agents:status"] = async () => agentsStatus(active);
    testRoutes["agents:configure"] = async (args) => {
      active = args.includes("--use") ? args[args.indexOf("--use") + 1] : "";
      return makeEnvelope(makeApplyResult({ domain: "agents" }), { command: "agents.configure" });
    };
    const { calls } = renderWithEngine(<App />, testRoutes);
    await screen.findByRole("heading", { name: "助手" }, { timeout: 10_000 });

    await openPalette();
    await userEvent.type(screen.getByTestId("command-palette-input"), "Kimi Code");
    await userEvent.click(await screen.findByTestId("command-item-connection:claude-code:kimi"));

    await screen.findByText("已切换 Claude Code 到 Kimi Code");
    const configure = calls.find((call) => call.command === "agents:configure");
    expect(configure?.args).toEqual(["--only", "agents:claude-code", "--use", "kimi"]);
    expect(configure?.options?.runId).toMatch(/^gui-/);
    expect(calls.filter((call) => call.command === "agents:status").length).toBeGreaterThanOrEqual(2);

    await openPalette();
    await userEvent.type(screen.getByTestId("command-palette-input"), "Kimi Code");
    expect(await screen.findByTestId("command-item-connection:claude-code:kimi")).toBeDisabled();
  });

  it("原生托盘接入事件唤回精确目标且确认后走同一条切换与双回读路径", async () => {
    setSettingsClient({ get: async () => dummySettings, save: async (preferences) => ({ ...dummySettings, preferences }) });
    let active = "";
    const testRoutes = routes();
    testRoutes["agents:status"] = async () => agentsStatus(active);
    testRoutes["agents:configure"] = async (args) => {
      active = args.includes("--use") ? args[args.indexOf("--use") + 1] : "";
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
          profile: "kimi",
          profileLabel: "Kimi Code",
        },
      });
    }

    expect(await screen.findByTestId("command-palette")).toBeInTheDocument();
    expect(await screen.findByTestId("command-item-connection:claude-code:kimi")).toBeInTheDocument();
    expect(calls.some((call) => call.command === "agents:configure")).toBe(false);
    await userEvent.keyboard("{Enter}");

    await screen.findByText("已切换 Claude Code 到 Kimi Code");
    expect(calls.find((call) => call.command === "agents:configure")?.args).toEqual([
      "--only",
      "agents:claude-code",
      "--use",
      "kimi",
    ]);
    expect(calls.filter((call) => call.command === "agents:status").length).toBeGreaterThanOrEqual(2);
  });

  it("托盘目标遇到外部修改时不静默覆盖而是转入可见确认", async () => {
    setSettingsClient({ get: async () => dummySettings, save: async (preferences) => ({ ...dummySettings, preferences }) });
    const testRoutes = routes();
    testRoutes["agents:configure"] = async () => makeEnvelope(
      {},
      {
        command: "agents.configure",
        ok: false,
        errors: [{ code: "EXTERNAL_MODIFICATION", element: "agents:claude-code", message: "配置已被外部修改，拒绝覆盖" }],
      },
    );
    renderWithEngine(<App />, testRoutes);
    await screen.findByRole("heading", { name: "助手" }, { timeout: 10_000 });
    await waitFor(() => expect(trayEvents.listeners.size).toBe(1));

    for (const listener of trayEvents.listeners) {
      listener({
        payload: {
          type: "switch-connection",
          agentId: "claude-code",
          agentName: "Claude Code",
          profile: "kimi",
          profileLabel: "Kimi Code",
        },
      });
    }

    expect(await screen.findByTestId("command-palette")).toBeInTheDocument();
    await userEvent.keyboard("{Enter}");
    expect(await screen.findByRole("dialog", { name: "覆盖被改过的接入配置" })).toBeInTheDocument();
  });

  it("切回助手默认账号使用 use-default，不复写不存在的 profile", async () => {
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
    await userEvent.type(screen.getByTestId("command-palette-input"), "默认账号");
    await userEvent.click(await screen.findByTestId("command-item-connection:claude-code:default"));

    await screen.findByText("已切换 Claude Code 到 助手默认账号");
    expect(calls.find((call) => call.command === "agents:configure")?.args).toEqual([
      "--only",
      "agents:claude-code",
      "--use-default",
    ]);
  });

  it("纳管选择已变化但客户端实际配置未生效时不误报成功", async () => {
    setSettingsClient({ get: async () => dummySettings, save: async (preferences) => ({ ...dummySettings, preferences }) });
    let active = "";
    const testRoutes = routes();
    testRoutes["agents:status"] = async () => agentsStatus(active, "");
    testRoutes["agents:configure"] = async () => {
      active = "kimi";
      return makeEnvelope(makeApplyResult({ domain: "agents" }), { command: "agents.configure" });
    };
    renderWithEngine(<App />, testRoutes);
    await screen.findByRole("heading", { name: "助手" }, { timeout: 10_000 });

    await openPalette();
    await userEvent.type(screen.getByTestId("command-palette-input"), "Kimi Code");
    await userEvent.click(await screen.findByTestId("command-item-connection:claude-code:kimi"));

    expect(await screen.findByText("写入已经返回成功，但状态回读没有确认接入生效，请在助手页检查")).toBeInTheDocument();
    expect(screen.queryByText("已切换 Claude Code 到 Kimi Code")).not.toBeInTheDocument();
  });

  it("当前接入被外部改动时可在命令面板原位重新应用并恢复可信状态", async () => {
    setSettingsClient({ get: async () => dummySettings, save: async (preferences) => ({ ...dummySettings, preferences }) });
    let actual = "";
    const testRoutes = routes();
    testRoutes["agents:status"] = async () => agentsStatus("kimi", actual);
    testRoutes["agents:configure"] = async () => {
      actual = "kimi";
      return makeEnvelope(makeApplyResult({ domain: "agents" }), { command: "agents.configure" });
    };
    const { calls } = renderWithEngine(<App />, testRoutes);
    await screen.findByRole("heading", { name: "助手" }, { timeout: 10_000 });

    await openPalette();
    await userEvent.type(screen.getByTestId("command-palette-input"), "Kimi Code");
    const repair = await screen.findByTestId("command-item-connection:claude-code:kimi");
    expect(repair).toBeEnabled();
    expect(repair).toHaveTextContent("配置不一致 · 重新应用");
    await userEvent.click(repair);

    expect(await screen.findByText("已切换 Claude Code 到 Kimi Code")).toBeInTheDocument();
    expect(calls.find((call) => call.command === "agents:configure")?.args).toEqual([
      "--only",
      "agents:claude-code",
      "--use",
      "kimi",
    ]);

    await openPalette();
    await userEvent.type(screen.getByTestId("command-palette-input"), "Kimi Code");
    const current = await screen.findByTestId("command-item-connection:claude-code:kimi");
    expect(current).toBeDisabled();
    expect(current).toHaveTextContent("当前接入");
  });

  it("外部修改时不冒充成功，确认后才携带 allow-takeover 重试", async () => {
    setSettingsClient({ get: async () => dummySettings, save: async (preferences) => ({ ...dummySettings, preferences }) });
    let active = "";
    let attempts = 0;
    const testRoutes = routes();
    testRoutes["agents:status"] = async () => agentsStatus(active);
    testRoutes["agents:configure"] = async (args) => {
      attempts += 1;
      if (!args.includes("--allow-takeover")) {
        return makeEnvelope(
          {},
          {
            command: "agents.configure",
            ok: false,
            errors: [{ code: "EXTERNAL_MODIFICATION", element: "agents:claude-code", message: "配置已被外部修改，拒绝覆盖" }],
          },
        );
      }
      active = "kimi";
      return makeEnvelope(makeApplyResult({ domain: "agents" }), { command: "agents.configure" });
    };
    const { calls } = renderWithEngine(<App />, testRoutes);
    await screen.findByRole("heading", { name: "助手" }, { timeout: 10_000 });

    await openPalette();
    await userEvent.type(screen.getByTestId("command-palette-input"), "Kimi Code");
    await userEvent.click(await screen.findByTestId("command-item-connection:claude-code:kimi"));

    const confirm = await screen.findByRole("dialog", { name: "覆盖被改过的接入配置" });
    expect(screen.queryByText("已切换 Claude Code 到 Kimi Code")).not.toBeInTheDocument();
    await userEvent.click(screen.getByTestId("confirm-modal-confirm"));
    await waitFor(() => expect(confirm).not.toBeInTheDocument());
    expect(attempts).toBe(2);
    expect(calls.filter((call) => call.command === "agents:configure")[1]?.args).toEqual([
      "--only",
      "agents:claude-code",
      "--use",
      "kimi",
      "--allow-takeover",
    ]);
  });

  it("配置被拒绝时保留真实状态并说明失败，不弹接管确认", async () => {
    setSettingsClient({ get: async () => dummySettings, save: async (preferences) => ({ ...dummySettings, preferences }) });
    const testRoutes = routes();
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
    await userEvent.type(screen.getByTestId("command-palette-input"), "Kimi Code");
    await userEvent.click(await screen.findByTestId("command-item-connection:claude-code:kimi"));

    expect(await screen.findByText("接入没有切换：凭据变量 KIMI_API_KEY 未配置")).toBeInTheDocument();
    expect(screen.queryByRole("dialog", { name: "覆盖被改过的接入配置" })).not.toBeInTheDocument();
  });
});
