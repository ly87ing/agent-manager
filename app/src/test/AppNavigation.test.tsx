import { describe, expect, it } from "vitest";
import App from "../App";
import { makeAuditData, makeElement, makeEnvelope, makeFreshness, makeMode } from "./fixtures";
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
    "policy:get": makeEnvelope({ layers: { sqlite: null } }, { command: "policy.get" }),
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
  }, 15000);

  it("从助手卡片点击「管理技能」跳转到技能页并预选对应助手，点击侧栏「技能」重置为全部助手", async () => {
    setSettingsClient({
      get: async () => dummySettings,
      save: async (preferences) => ({ ...dummySettings, preferences }),
    });

    const customRoutes = {
      ...routes(),
      "agents:status": makeEnvelope(
        {
          domain: "agents",
          elements: [
            makeElement({
              id: "agents:claude-code",
              domain: "agents",
              spec: { display_name: "Claude Code", bin: "claude", preferred_provider: "npm", providers: { npm: { managed: true } } },
              policy: { enabled: true },
              observed: { providers: ["npm"], installed: true, preferred_active: true },
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
              spec: { url: "https://github.com/example/agent-skills.git", skill_paths: ["alpha"], agents: ["claude-code"] },
              observed: { repo_cache: "/repo/agent-skills", repo_cached: true, owner_marked: true, discovered: ["alpha"], discovery_inbox: [] },
              state: "managed-ok",
            }),
            makeElement({
              id: "skill:agent-skills/alpha",
              domain: "skills",
              spec: { collection: "agent-skills", name: "alpha", source_path: "/repo/.../alpha" },
              policy: { enabled: true, agents: { "claude-code": true } },
              observed: {
                agents: {
                  "claude-code": {
                    state: "linked",
                    link_path: "/home/.claude/skills/alpha",
                    target_dir_exists: true,
                    actions: ["disable"],
                  },
                },
                discovery_inbox: false,
              },
              state: "managed-ok",
            }),
          ],
        },
        { command: "skills.status" },
      ),
    };

    renderWithEngine(<App />, customRoutes);

    // 默认助手页，点击 Claude Code 的「管理技能」
    const manageBtn = await screen.findByTestId("manage-skills-agents:claude-code");
    await userEvent.click(manageBtn);

    // 跳转到技能页，并且助手筛选预选为 claude-code
    const nav = screen.getByRole("navigation");
    const skillsNav = within(nav).getByRole("button", { name: "技能" });
    expect(skillsNav).toHaveAttribute("aria-current", "page");

    const tabSkills = await screen.findByRole("tab", { name: "全部技能" });
    expect(tabSkills).toHaveAttribute("aria-selected", "true");

    const agentSelect = screen.getByTestId("filter-agent-select") as HTMLSelectElement;
    expect(agentSelect.value).toBe("claude-code");

    // 点击侧栏「技能」项，重置为全部助手
    await userEvent.click(skillsNav);
    expect(agentSelect.value).toBe("all");
  }, 15000);
});
