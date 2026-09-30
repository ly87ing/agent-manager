import { describe, expect, it } from "vitest";
import App from "../App";
import { agentManagementFixture, makeAuditData, makeElement, makeEnvelope, makeFreshness, makeMode } from "./fixtures";
import { renderWithEngine, screen, userEvent, within } from "./render-with-engine";
import { setSettingsClient } from "../lib/settings";
import { encodeShareLink, SHARE_PREFIX } from "../lib/shareLink";
import { fireEvent } from "@testing-library/react";
import type { SettingsData } from "../lib/contract";

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
    "system:discover": makeEnvelope(
      { domain: "discover", agents: [], skills: { items: [], groups: [] }, mcp: [], summary: {} },
      { command: "system.discover" },
    ),
    "policy:get": makeEnvelope({ layers: { sqlite: null } }, { command: "policy.get" }),
  };
}

describe("App 顶层导航", () => {
  it("顶栏：待处理、账号、四类资源与来源，默认渲染账号页；扫描本机已有内容不再单独占设置入口", async () => {
    setSettingsClient({
      get: async () => dummySettings,
      save: async (preferences) => ({ ...dummySettings, preferences }),
    });

    renderWithEngine(<App />, routes());

    const nav = await screen.findByRole("navigation");
    const navButtons = within(nav).getAllByRole("button");

    // 用量、设置只显示图标，按无障碍名称核对
    expect(navButtons.map((b) => b.getAttribute("aria-label") ?? b.textContent?.trim())).toEqual([
      "待处理",
      "账号",
      "技能",
      "MCP",
      "规则",
      "命令",
      "来源",
      "程序",
      "搜索（⌘K）",
      "切换到深色",
      "用量",
      "设置",
    ]);

    // 顶栏一键切换浅色 / 深色，只改界面，不调用引擎
    await userEvent.click(within(nav).getByTestId("theme-toggle"));
    expect(document.documentElement).toHaveClass("dark");
    expect(within(nav).getByTestId("theme-toggle")).toHaveAccessibleName("切换到浅色");
    await userEvent.click(within(nav).getByTestId("theme-toggle"));
    expect(document.documentElement).not.toHaveClass("dark");

    const accountsNav = within(nav).getByRole("button", { name: "账号" });
    expect(accountsNav).toHaveAttribute("aria-current", "page");
    expect(await screen.findByRole("heading", { name: "账号" })).toBeInTheDocument();

    const settingsNav = within(nav).getByRole("button", { name: "设置" });
    await userEvent.click(settingsNav);
    expect(settingsNav).toHaveAttribute("aria-current", "page");
    expect(await screen.findByRole("heading", { name: "设置" })).toBeInTheDocument();

    expect(screen.queryByTestId("advanced-entry-takeover")).toBeNull();
    expect(screen.queryByTestId("advanced-entry-import-link")).toBeNull();

    // 待处理：什么都生效时是空态
    await userEvent.click(within(nav).getByRole("button", { name: "待处理" }));
    expect(await screen.findByTestId("inbox-empty")).toHaveTextContent("没有需要处理的");

    const rulesNav = within(nav).getByRole("button", { name: "规则" });
    await userEvent.click(rulesNav);
    expect(rulesNav).toHaveAttribute("aria-current", "page");
    expect(await screen.findByRole("heading", { name: "规则" })).toBeInTheDocument();
    // Hooks 已移除：规则页不再有标签切换
    expect(screen.queryByRole("tab", { name: /Hooks/ })).toBeNull();
  }, 15000);

  it("导航上标出各页待处理数；数字不进按钮名字，没有待处理的不标", async () => {
    setSettingsClient({ get: async () => dummySettings, save: async (preferences) => ({ ...dummySettings, preferences }) });
    renderWithEngine(<App />, {
      ...routes(),
      "skills:status": makeEnvelope({
        domain: "skills",
        elements: [
          makeElement({ id: "skill:c", domain: "skills", spec: { url: "https://example.com/c.git" }, observed: { repo_cached: true } }),
          makeElement({ id: "skill:c/broken", domain: "skills", spec: { collection: "c", name: "broken" }, observed: { agents: { "claude-code": { state: "dead-link", actions: ["update"] } } } }),
        ],
      }, { command: "skills.status" }),
    });
    const nav = await screen.findByRole("navigation");
    expect(await within(nav).findByTestId("nav-badge-skills")).toHaveTextContent("1");
    expect(within(nav).getByRole("button", { name: "技能" })).toBeInTheDocument();
    expect(within(nav).queryByTestId("nav-badge-mcp")).toBeNull();
  });

  it("从分享链接导入：来源页打开，带明文取值的链接拦下，合法的跳到 MCP 页并预填新增表单", async () => {
    setSettingsClient({ get: async () => dummySettings, save: async (preferences) => ({ ...dummySettings, preferences }) });
    renderWithEngine(<App />, routes());
    const nav = await screen.findByRole("navigation");
    await userEvent.click(within(nav).getByRole("button", { name: "来源" }));
    await userEvent.click(await screen.findByTestId("source-import-link"));
    const dialog = await screen.findByRole("dialog", { name: "从分享链接导入" });
    const input = within(dialog).getByTestId("import-link-input");
    fireEvent.change(input, { target: { value: SHARE_PREFIX + btoa(JSON.stringify({ v: 1, kind: "mcp", name: "x", spec: { command: "npx", env: { T: "sk-SECRET" } } })) } });
    expect(within(dialog).getByRole("alert")).toHaveTextContent("明文取值");
    expect(within(dialog).getByTestId("import-link-continue")).toBeDisabled();
    fireEvent.change(input, { target: { value: encodeShareLink({ kind: "mcp", name: "shared-srv", spec: { command: "uvx", args: ["shared-srv"], env: {} } }) } });
    expect(within(dialog).getByTestId("import-link-summary")).toHaveTextContent("shared-srv");
    await userEvent.click(within(dialog).getByTestId("import-link-continue"));
    expect(within(nav).getByRole("button", { name: "MCP" })).toHaveAttribute("aria-current", "page");
    const form = await screen.findByRole("dialog", { name: "新增服务" });
    expect(within(form).getByTestId("server-name-input")).toHaveValue("shared-srv");
  }, 15000);

  it("从账号行进入技能页并预选对应账号，点击顶栏「技能」重置为全部助手", async () => {
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
              observed: { install_source: "npm", installed: true, management: agentManagementFixture("claude") },
              state: "managed-ok",
              freshness: makeFreshness(),
              actions: [],
            }),
            makeElement({
              id: "agents:cc-work",
              domain: "agents",
              spec: {
                display_name: "Claude Code · 工作号",
                bin: "claude",
                profile: { base_agent: "claude-code", kind: "claude", config_dir: "~/.claude-cc-work" },
              },
              policy: { enabled: true },
              observed: { installed: true },
              state: "managed-ok",
              freshness: makeFreshness(),
              actions: [],
            }),
          ],
        },
        { command: "agents.status" },
      ),
      "profiles:status": makeEnvelope(
        {
          domain: "profiles",
          elements: [makeElement({
            id: "profiles:cc-work",
            domain: "profiles",
            spec: {
              display_name: "Claude Code · 工作号",
              base_agent: "claude-code",
              kind: "claude",
              config_dir: "~/.claude-cc-work",
            },
            policy: { enabled: true },
            observed: {
              base_agent: "claude-code",
              kind: "claude",
              config_env: "CLAUDE_CONFIG_DIR",
              config_dir: "~/.claude-cc-work",
              env_refs: [],
            },
            state: "not-materialized",
            freshness: makeFreshness({ state: "not-applicable" }),
            actions: [],
          })],
        },
        { command: "profiles.status" },
      ),
      "skills:status": makeEnvelope(
        {
          domain: "skills",
          elements: [
            makeElement({
              id: "skill:agent-skills",
              domain: "skills",
              spec: { url: "https://github.com/example/agent-skills.git", skill_paths: ["alpha"], agents: ["cc-work"] },
              observed: { install_source: null, repo_cache: "/repo/agent-skills", repo_cached: true, owner_marked: true, discovered: ["alpha"], discovery_inbox: [] },
              state: "managed-ok",
            }),
            makeElement({
              id: "skill:agent-skills/alpha",
              domain: "skills",
              spec: { collection: "agent-skills", name: "alpha", source_path: "/repo/.../alpha" },
              policy: { enabled: true, agents: { "cc-work": true } },
              observed: {
                agents: {
                  "cc-work": {
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

    // 账号行的「技能」直达技能页，并预选这个账号
    const row = await screen.findByTestId("account-row-profiles:cc-work");
    await userEvent.click(within(row).getByRole("button", { name: "更多操作" }));
    await userEvent.click(screen.getByRole("menuitem", { name: "技能" }));

    // 跳转到技能页，并且助手筛选预选为 cc-work
    const nav = screen.getByRole("navigation");
    const skillsNav = within(nav).getByRole("button", { name: "技能" });
    expect(skillsNav).toHaveAttribute("aria-current", "page");

    const agentSelect = (await screen.findByTestId("filter-agent-select")) as HTMLSelectElement;
    expect(agentSelect.value).toBe("cc-work");

    // 点击顶栏「技能」，重置为全部账号
    await userEvent.click(skillsNav);
    expect(agentSelect.value).toBe("all");
  }, 15000);

  it("未实现的 mcp/rules status 返回 ok:false 与 data:{} 时，页面展示错误，顶层导航仍在且可切回账号", async () => {
    setSettingsClient({
      get: async () => dummySettings,
      save: async (preferences) => ({ ...dummySettings, preferences }),
    });

    const notImplementedRoutes = {
      ...routes(),
      "mcp:status": makeEnvelope(
        {} as unknown as { domain: string; elements: [] },
        {
          command: "mcp.status",
          ok: false,
          errors: [{ code: "DOMAIN_NOT_IMPLEMENTED", element: null, message: "MCP 域尚未实现" }],
        },
      ),
      "rules:status": makeEnvelope(
        {} as unknown as { domain: string; elements: [] },
        {
          command: "rules.status",
          ok: false,
          errors: [{ code: "DOMAIN_NOT_IMPLEMENTED", element: null, message: "Rules 域尚未实现" }],
        },
      ),
    };

    renderWithEngine(<App />, notImplementedRoutes);

    const nav = await screen.findByRole("navigation");

    // 切换至 MCP 页：展示错误码与消息，无白屏
    const mcpNav = within(nav).getByRole("button", { name: "MCP" });
    await userEvent.click(mcpNav);
    expect(mcpNav).toHaveAttribute("aria-current", "page");
    expect(await screen.findByRole("heading", { name: "MCP" })).toBeInTheDocument();
    expect(screen.getByText("DOMAIN_NOT_IMPLEMENTED")).toBeInTheDocument();
    expect(screen.getByText("MCP 域尚未实现")).toBeInTheDocument();

    // 顶层导航仍在，切回账号
    const accountsNav = within(nav).getByRole("button", { name: "账号" });
    await userEvent.click(accountsNav);
    expect(accountsNav).toHaveAttribute("aria-current", "page");
    expect(await screen.findByRole("heading", { name: "账号" })).toBeInTheDocument();

    // 切换至规则页：展示 Rules 错误
    const rulesNav = within(nav).getByRole("button", { name: "规则" });
    await userEvent.click(rulesNav);
    expect(rulesNav).toHaveAttribute("aria-current", "page");
    expect(await screen.findByRole("heading", { name: "规则" })).toBeInTheDocument();
    expect(screen.getByText("DOMAIN_NOT_IMPLEMENTED")).toBeInTheDocument();
    expect(screen.getByText("Rules 域尚未实现")).toBeInTheDocument();

    // 顶层导航仍在，再次切回账号
    await userEvent.click(accountsNav);
    expect(accountsNav).toHaveAttribute("aria-current", "page");
    expect(await screen.findByRole("heading", { name: "账号" })).toBeInTheDocument();
  }, 15000);
});
