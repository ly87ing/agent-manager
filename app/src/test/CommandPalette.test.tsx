import { beforeEach, describe, expect, it, onTestFinished, vi } from "vitest";
import { waitFor } from "@testing-library/react";
import App from "../App";
import type { Envelope, SettingsData } from "../lib/contract";
import { setSettingsClient } from "../lib/settings";
import { encodeShareLink } from "../lib/shareLink";
import { agentManagementFixture, makeAliasStatus, makeAuditData, makeElement, makeEnvelope, makeMode, makePolicyData } from "./fixtures";
import { renderWithEngine, screen, userEvent, within } from "./render-with-engine";

/** 按事件名记下界面订阅的桌面壳事件（托盘动作、点开的分享链接）。 */
const trayEvents = vi.hoisted(() => ({
  byName: new Map<string, Set<(event: { payload: unknown }) => void>>(),
  get listeners() {
    return this.byName.get("tray-action") ?? new Set();
  },
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (name: string, listener: (event: { payload: unknown }) => void) => {
    const set = trayEvents.byName.get(name) ?? new Set();
    set.add(listener);
    trayEvents.byName.set(name, set);
    return () => set.delete(listener);
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

/** 主账号 claude + 有启动命令的 work + 还没有启动命令的 relay。 */
function routes(): Record<string, TestRoute> {
  return {
    "system:audit": makeEnvelope(makeAuditData(), { command: "system.audit", mode: makeMode() }),
    "agents:status": makeEnvelope({
      domain: "agents",
      elements: [makeElement({
        id: "agents:claude-code",
        domain: "agents",
        spec: { display_name: "Claude Code", bin: "claude" },
        observed: { management: agentManagementFixture("claude") },
      })],
    }, { command: "agents.status" }),
    "profiles:status": makeEnvelope({
      domain: "profiles",
      elements: ["work", "relay"].map((id) => makeElement({
        id: `profiles:${id}`,
        domain: "profiles",
        spec: { display_name: id === "work" ? "工作账号" : "中转", base_agent: "claude-code", kind: "claude", config_dir: `~/.claude-${id}`, base_url: null, model: null },
        observed: { config_dir: `~/.claude-${id}`, blocked_reason: null },
        state: "managed-ok",
      })),
    }, { command: "profiles.status" }),
    "aliases:status": makeEnvelope(makeAliasStatus({ cw: 'CLAUDE_CONFIG_DIR="$HOME/.claude-work" claude' }), { command: "aliases.status" }),
    "skills:status": makeEnvelope({
      domain: "skills",
      elements: [makeElement({ id: "skill:agent-skills", domain: "skills", spec: { url: "https://example.com/x.git" }, observed: { repo_cached: true } })],
    }, { command: "skills.status" }),
    "mcp:status": makeEnvelope({
      domain: "mcp",
      targets: [],
      elements: [makeElement({ id: "mcp:grok-search", domain: "mcp", observed: { agents: {} } })],
    }, { command: "mcp.status" }),
    "secrets:status": makeEnvelope({ domain: "secrets", elements: [] }, { command: "secrets.status" }),
    "rules:status": makeEnvelope({ domain: "rules", elements: [] }, { command: "rules.status" }),
    "runs:list": makeEnvelope({ runs: [] }, { command: "runs.list" }),
    "policy:get": makeEnvelope(makePolicyData({ layers: { sqlite: null } }), { command: "policy.get" }),
    "agents:auth-status": makeEnvelope({ state: "logged-in", logged_in: true, method: null, third_party: false, mixed: false, reason: null }, { command: "agents.auth-status" }),
  };
}

function mockClipboard() {
  const writeText = vi.fn(async () => undefined);
  const original = navigator.clipboard;
  Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText } });
  onTestFinished(() => { Object.defineProperty(navigator, "clipboard", { configurable: true, value: original }); });
  return writeText;
}

async function renderApp() {
  setSettingsClient({ get: async () => dummySettings, save: async (preferences) => ({ ...dummySettings, preferences }) });
  const view = renderWithEngine(<App />, routes());
  await screen.findByRole("heading", { name: "账号" }, { timeout: 10_000 });
  return view;
}

/** 面板本身不写任何配置。 */
function writes(calls: { command: string }[]) {
  return calls.filter((call) => /:(apply|configure|set|seed|shared-apply)$/.test(call.command));
}

describe("命令面板", () => {
  beforeEach(() => trayEvents.byName.clear());

  it("⌘K 搜到页面、账号与资源，选中页面跳过去", async () => {
    await renderApp();
    await userEvent.keyboard("{Meta>}k{/Meta}");
    const palette = await screen.findByTestId("command-palette");
    expect(within(palette).getByTestId("command-item-account:agents:claude-code")).toHaveTextContent("Claude · 主账号");
    expect(within(palette).getByTestId("command-item-account:profiles:work")).toHaveTextContent("复制 cw");
    expect(within(palette).getByTestId("command-item-skill:agent-skills")).toBeInTheDocument();
    await userEvent.type(screen.getByTestId("command-palette-input"), "MCP");
    await userEvent.click(within(palette).getByTestId("command-item-page:mcp"));
    expect(screen.queryByTestId("command-palette")).toBeNull();
    expect(within(screen.getByRole("navigation")).getByRole("button", { name: "MCP" })).toHaveAttribute("aria-current", "page");
  });

  it("⌘⇧K 只列账号；选中复制启动命令，没有启动命令的去账号页；全程不写配置", async () => {
    const writeText = mockClipboard();
    const { calls } = await renderApp();
    await userEvent.keyboard("{Meta>}{Shift>}k{/Shift}{/Meta}");
    const palette = await screen.findByRole("dialog", { name: "账号" });
    expect(within(palette).queryByTestId("command-item-page:mcp")).toBeNull();
    expect(within(palette).queryByTestId("command-item-skill:agent-skills")).toBeNull();
    expect(within(palette).getByTestId("command-item-account:profiles:relay")).toHaveTextContent("没有启动命令");

    await userEvent.type(screen.getByTestId("command-palette-input"), "cw");
    await userEvent.click(within(palette).getByTestId("command-item-account:profiles:work"));
    await waitFor(() => expect(writeText).toHaveBeenCalledWith("cw"));

    await userEvent.keyboard("{Meta>}{Shift>}k{/Shift}{/Meta}");
    await userEvent.click(await screen.findByTestId("command-item-account:profiles:relay"));
    expect(within(screen.getByRole("navigation")).getByRole("button", { name: "账号" })).toHaveAttribute("aria-current", "page");
    expect(writes(calls)).toEqual([]);
  });

  it("不再有切换主配置的入口", async () => {
    await renderApp();
    await userEvent.keyboard("{Meta>}k{/Meta}");
    await userEvent.type(screen.getByTestId("command-palette-input"), "切回");
    expect(screen.getByText("没有匹配的页面、账号或资源。")).toBeInTheDocument();
  });

  it("键盘可完成：方向键选择、回车执行、Esc 关闭", async () => {
    await renderApp();
    await userEvent.keyboard("{Meta>}k{/Meta}");
    await userEvent.type(screen.getByTestId("command-palette-input"), "设置");
    await userEvent.keyboard("{Enter}");
    expect(await screen.findByRole("heading", { name: "设置" })).toBeInTheDocument();
    await userEvent.keyboard("{Meta>}k{/Meta}");
    await screen.findByTestId("command-palette");
    await userEvent.keyboard("{Escape}");
    expect(screen.queryByTestId("command-palette")).toBeNull();
  });

  it("设置里的二级页也搜得到，选中直达", async () => {
    await renderApp();
    await userEvent.keyboard("{Meta>}k{/Meta}");
    await userEvent.type(screen.getByTestId("command-palette-input"), "密钥");
    await userEvent.click(await screen.findByTestId("command-item-page:settings:secrets"));
    expect(within(screen.getByRole("navigation")).getByRole("button", { name: "设置" })).toHaveAttribute("aria-current", "page");
  });

  it("托盘点中账号：面板只剩这个账号，Enter 复制启动命令", async () => {
    const writeText = mockClipboard();
    const { calls } = await renderApp();
    await waitFor(() => expect(trayEvents.listeners.size).toBe(1));
    for (const listener of trayEvents.listeners) {
      listener({ payload: { type: "open-account", agentName: "Claude Code", accountId: "profiles:work", accountName: "工作账号" } });
    }
    const palette = await screen.findByRole("dialog", { name: "账号" });
    expect(within(palette).getAllByRole("button")).toHaveLength(1);
    await userEvent.keyboard("{Enter}");
    await waitFor(() => expect(writeText).toHaveBeenCalledWith("cw"));
    expect(writes(calls)).toEqual([]);
  });

  it("系统里点开分享链接：打开导入对话框并填好链接，确认前不写任何东西", async () => {
    const { calls } = await renderApp();
    await waitFor(() => expect(trayEvents.byName.get("import-link")?.size).toBe(1));
    const link = encodeShareLink({ kind: "mcp", name: "shared-srv", spec: { command: "uvx", args: [], env: {} } });
    for (const listener of trayEvents.byName.get("import-link") ?? []) listener({ payload: link });
    const dialog = await screen.findByRole("dialog", { name: "从分享链接导入" });
    expect(within(dialog).getByTestId("import-link-input")).toHaveValue(link);
    expect(within(dialog).getByTestId("import-link-summary")).toHaveTextContent("shared-srv");
    expect(writes(calls)).toEqual([]);
  });
});
