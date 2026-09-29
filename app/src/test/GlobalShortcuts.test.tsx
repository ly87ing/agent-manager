import { describe, expect, it } from "vitest";
import App from "../App";
import type { SettingsData } from "../lib/contract";
import { setSettingsClient } from "../lib/settings";
import { makeAuditData, makeEnvelope, makeMode } from "./fixtures";
import { renderWithEngine, screen, userEvent, within } from "./render-with-engine";

const dummySettings: SettingsData = {
  appVersion: "0.1.0",
  contractVersion: "v1",
  resourceDir: "/tmp/resources",
  dataDir: "/tmp/data",
  updaterConfigured: false,
  preferences: { logRetentionDays: 30 },
};

function routes() {
  return {
    "system:audit": makeEnvelope(makeAuditData(), { command: "system.audit", mode: makeMode() }),
    "agents:status": makeEnvelope({ domain: "agents", elements: [] }, { command: "agents.status" }),
    "skills:status": makeEnvelope({ domain: "skills", elements: [] }, { command: "skills.status" }),
    "mcp:status": makeEnvelope({ domain: "mcp", elements: [] }, { command: "mcp.status" }),
    "rules:status": makeEnvelope({ domain: "rules", elements: [] }, { command: "rules.status" }),
    "runs:list": makeEnvelope({ runs: [] }, { command: "runs.list" }),
    "policy:get": makeEnvelope({ layers: { sqlite: null } }, { command: "policy.get" }),
  };
}

describe("全局快捷键", () => {
  it("Cmd/Ctrl+, 跳到设置页", async () => {
    setSettingsClient({ get: async () => dummySettings, save: async (preferences) => ({ ...dummySettings, preferences }) });
    renderWithEngine(<App />, routes());

    const nav = await screen.findByRole("navigation");
    expect(await screen.findByRole("heading", { name: "账号" })).toBeInTheDocument();

    await userEvent.keyboard("{Meta>},{/Meta}");
    expect(within(nav).getByRole("button", { name: "设置" })).toHaveAttribute("aria-current", "page");
    expect(await screen.findByRole("heading", { name: "设置" })).toBeInTheDocument();

    // Ctrl 变体（非 macOS 约定）：先回账号页再用 Ctrl+, 进设置
    await userEvent.click(within(nav).getByRole("button", { name: "账号" }));
    expect(within(nav).getByRole("button", { name: "账号" })).toHaveAttribute("aria-current", "page");
    await userEvent.keyboard("{Control>},{/Control}");
    expect(within(nav).getByRole("button", { name: "设置" })).toHaveAttribute("aria-current", "page");
  }, 15000);

  it("⌘F 聚焦当前页搜索框；无搜索框的页面不响应", async () => {
    setSettingsClient({ get: async () => dummySettings, save: async (preferences) => ({ ...dummySettings, preferences }) });
    renderWithEngine(<App />, routes());

    const nav = await screen.findByRole("navigation");
    await userEvent.click(within(nav).getByRole("button", { name: "MCP" }));
    const search = await screen.findByTestId("mcp-search");
    expect(search).not.toHaveFocus();

    await userEvent.keyboard("{Meta>}f{/Meta}");
    expect(search).toHaveFocus();

    // 设置页没有搜索框：⌘F 不抢占、不报错（先移出焦点，避免守卫拦截 ⌘,）
    search.blur();
    await userEvent.keyboard("{Meta>},{/Meta}");
    expect(await screen.findByRole("heading", { name: "设置" })).toBeInTheDocument();
    expect(screen.queryByTestId("mcp-search")).toBeNull();
    await userEvent.keyboard("{Meta>}f{/Meta}");
    expect(document.activeElement?.tagName).toBe("BODY");
  }, 15000);

  it("焦点在输入框内时快捷键不抢占编辑", async () => {
    setSettingsClient({ get: async () => dummySettings, save: async (preferences) => ({ ...dummySettings, preferences }) });
    renderWithEngine(<App />, routes());

    const nav = await screen.findByRole("navigation");
    await userEvent.click(within(nav).getByRole("button", { name: "MCP" }));
    const search = await screen.findByTestId("mcp-search");
    await userEvent.click(search);
    expect(search).toHaveFocus();

    await userEvent.keyboard("{Meta>},{/Meta}");
    expect(within(nav).getByRole("button", { name: "MCP" })).toHaveAttribute("aria-current", "page");
    expect(screen.queryByRole("heading", { name: "设置" })).toBeNull();
    await userEvent.keyboard("{Meta>}{Shift>}k{/Shift}{/Meta}");
    expect(screen.queryByRole("dialog", { name: "账号与环境" })).toBeNull();
  }, 15000);
});
