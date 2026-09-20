import { describe, expect, it } from "vitest";
import { waitFor } from "@testing-library/react";
import App from "../App";
import type { Envelope, SettingsData } from "../lib/contract";
import { setSettingsClient } from "../lib/settings";
import { makeApplyResult, makeAuditData, makeElement, makeEnvelope, makeFreshness, makeMode, makePolicyData } from "./fixtures";
import { renderWithEngine, screen, userEvent } from "./render-with-engine";

const dummySettings: SettingsData = {
  appVersion: "0.1.0",
  contractVersion: "v1",
  resourceDir: "/tmp/resources",
  dataDir: "/tmp/data",
  updaterConfigured: false,
  preferences: { logRetentionDays: 30 },
};

function routes(): Record<string, Envelope> {
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
    "agents:status": makeEnvelope(
      {
        domain: "agents",
        elements: [
          makeElement({
            id: "agents:claude-code",
            domain: "agents",
            spec: { display_name: "Claude Code", bin: "claude" },
            policy: { enabled: true },
            observed: { install_source: null, installed: true },
            state: "managed-ok",
            freshness: makeFreshness(),
            actions: [],
          }),
        ],
      },
      { command: "agents.status" },
    ),
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
});
