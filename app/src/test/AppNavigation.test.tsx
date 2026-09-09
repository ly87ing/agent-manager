import { describe, expect, it } from "vitest";
import App from "../App";
import { makeAuditData, makeEnvelope, makeMode } from "./fixtures";
import { renderWithEngine, screen, userEvent, within } from "./render-with-engine";
import { setSettingsClient } from "../lib/settings";
import type { SettingsData } from "../lib/contract";

const dummySettings: SettingsData = {
  appVersion: "0.1.0",
  contractVersion: "v1",
  resourceDir: "/tmp/resources",
  dataDir: "/tmp/data",
  logDir: "/tmp/logs",
  updaterConfigured: false,
  preferences: { offline: false, defaultDryRun: false, upstreamTtlSec: 86400 },
};

function routes() {
  return {
    "system:audit": makeEnvelope(makeAuditData(), { command: "system.audit", mode: makeMode() }),
    "agents:status": makeEnvelope({ domain: "agents", elements: [] }, { command: "agents.status" }),
    "skills:status": makeEnvelope({ domain: "skills", elements: [] }, { command: "skills.status" }),
    "mcp:status": makeEnvelope({ domain: "mcp", elements: [] }, { command: "mcp.status" }),
    "rules:status": makeEnvelope({ domain: "rules", elements: [] }, { command: "rules.status" }),
    "runs:list": makeEnvelope([], { command: "runs.list" }),
    "policy:get": makeEnvelope({ layers: { baseline: null, synced: null, local: null } }, { command: "policy.get" }),
  };
}

describe("App 顶层导航收敛为日常六项", () => {
  it("侧边栏仅有六项，且精确中文，默认渲染助手页，点击导航可切换页面", async () => {
    setSettingsClient({
      get: async () => dummySettings,
      save: async (preferences) => ({ ...dummySettings, preferences }),
    });

    renderWithEngine(<App />, routes());

    const nav = await screen.findByRole("navigation");
    const navButtons = within(nav).getAllByRole("button");

    const expectedLabels = [
      "助手",
      "技能",
      "MCP",
      "规则与 Hooks",
      "操作记录",
      "设置",
    ];

    expect(navButtons.map((b) => b.textContent?.trim())).toEqual(expectedLabels);

    // 默认助手处于当前页
    const agentsNav = within(nav).getByRole("button", { name: "助手" });
    expect(agentsNav).toHaveAttribute("aria-current", "page");
    expect(await screen.findByRole("heading", { name: "助手" })).toBeInTheDocument();

    // 点击设置
    const settingsNav = within(nav).getByRole("button", { name: "设置" });
    await userEvent.click(settingsNav);
    expect(settingsNav).toHaveAttribute("aria-current", "page");
    expect(await screen.findByRole("heading", { name: "设置" })).toBeInTheDocument();

    // 点击规则与 Hooks
    const rulesHooksNav = within(nav).getByRole("button", { name: "规则与 Hooks" });
    await userEvent.click(rulesHooksNav);
    expect(rulesHooksNav).toHaveAttribute("aria-current", "page");
    expect(await screen.findByRole("tab", { name: "规则" })).toBeInTheDocument();
  });
});
