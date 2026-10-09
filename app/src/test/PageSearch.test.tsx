import { describe, expect, it } from "vitest";
import McpPage from "../pages/McpPage";
import RunsPage from "../pages/RunsPage";
import SkillsPage from "../pages/SkillsPage";
import type { Envelope } from "../lib/contract";
import {
  makeElement,
  makeEnvelope,
  makePolicyData,
  makeRunEntry,
} from "./fixtures";
import { renderWithEngine, screen, userEvent, chooseOption } from "./render-with-engine";
import { mcpStatusData } from "./mcp-fixtures";

/** 等集合出现，把收起的集合逐个展开（没有待处理的集合默认收起）。 */
async function expandAll() {
  await screen.findAllByTestId(/^collection-card-/);
  for (const toggle of screen.getAllByTestId(/^collection-toggle-/)) {
    if (toggle.getAttribute("aria-expanded") === "false") await userEvent.click(toggle);
  }
  await screen.findAllByTestId(/^skill-row-/);
}


function mcpRoutes(): Record<string, Envelope> {
  return { "mcp:status": makeEnvelope(mcpStatusData(), { command: "mcp.status" }) };
}

function runsRoutes(): Record<string, Envelope> {
  return {
    "runs:list": makeEnvelope(
      {
        runs: [
          makeRunEntry({ run_id: "20260905T235900Z-legacy1", operations: 2, last_at: "2026-09-05T23:59:00Z" }),
          makeRunEntry({ run_id: "20260906T010000Z-mock02", operations: 1, last_at: "2026-09-06T01:00:00Z" }),
        ],
      },
      { command: "runs.list" },
    ),
  };
}

function skillsRoutes(): Record<string, Envelope> {
  return {
    "skills:status": makeEnvelope(
      {
        domain: "skills",
        elements: [
          makeElement({
            id: "skill:web-tools",
            domain: "skills",
            spec: { url: "https://github.com/example/web-tools.git", skill_paths: ["lint"], agents: ["claude-code"] },
            observed: { install_source: null, repo_cache: "/repo/web-tools", repo_cached: true, owner_marked: true, discovered: ["lint"], discovery_inbox: [] },
            state: "managed-ok",
          }),
          makeElement({
            id: "skill:data-libs",
            domain: "skills",
            spec: { url: "https://github.com/example/data-libs.git", skill_paths: ["query"], agents: ["claude-code"] },
            observed: { install_source: null, repo_cache: "/repo/data-libs", repo_cached: true, owner_marked: true, discovered: ["query"], discovery_inbox: [] },
            state: "managed-ok",
          }),
          makeElement({
            id: "skill:web-tools/lint",
            domain: "skills",
            spec: { collection: "web-tools", name: "lint", source_path: "/repo/web-tools/lint" },
            policy: { enabled: true, agents: { "claude-code": true } },
            observed: {
              agents: { "claude-code": { state: "linked", link_path: "/home/.claude/skills/lint", target_dir_exists: true, assigned: true, enabled: true } },
              discovery_inbox: false,
            },
            state: "managed-ok",
          }),
          makeElement({
            id: "skill:data-libs/query",
            domain: "skills",
            spec: { collection: "data-libs", name: "query", source_path: "/repo/data-libs/query" },
            policy: { enabled: true, agents: { "claude-code": true } },
            observed: {
              agents: { "claude-code": { state: "linked", link_path: "/home/.claude/skills/query", target_dir_exists: true, assigned: true, enabled: true } },
              discovery_inbox: false,
            },
            state: "managed-ok",
          }),
        ],
      },
      { command: "skills.status" },
    ),
    "agents:status": makeEnvelope(
      { domain: "agents", elements: [makeElement({ id: "agents:claude-code", domain: "agents" })] },
      { command: "agents.status" },
    ),
    "policy:get": makeEnvelope(makePolicyData(), { command: "policy.get" }),
  };
}

describe("页内搜索：MCP 页", () => {
  it("服务按名称/命令/地址过滤卡片；无匹配时给出空态与清除入口", async () => {
    renderWithEngine(<McpPage />, mcpRoutes());
    const input = await screen.findByTestId("mcp-search");
    expect(await screen.findByTestId("mcp-card-mcp:fs")).toBeInTheDocument();
    expect(screen.getByTestId("mcp-card-mcp:remote-api")).toBeInTheDocument();

    await userEvent.type(input, "mcp.example.com");
    expect(screen.queryByTestId("mcp-card-mcp:fs")).toBeNull();
    expect(screen.getByTestId("mcp-card-mcp:remote-api")).toBeInTheDocument();

    await userEvent.clear(input);
    await userEvent.type(input, "不存在的服务");
    expect(await screen.findByTestId("mcp-search-empty")).toHaveTextContent("无匹配结果");

    await userEvent.click(screen.getByRole("button", { name: "清除搜索关键词" }));
    expect(screen.getByTestId("mcp-card-mcp:fs")).toBeInTheDocument();
    expect(screen.getByTestId("mcp-card-mcp:remote-api")).toBeInTheDocument();
  });

  it("账号筛选与搜索叠加（AND）：只剩该账号上的匹配条目", async () => {
    renderWithEngine(<McpPage />, mcpRoutes());
    await chooseOption(await screen.findByTestId("filter-agent-select"), "codex");
    const input = await screen.findByTestId("mcp-search");
    await userEvent.type(input, "remote");

    // 搜索中默认展开：只剩 remote-api，且只列 codex 的条目
    expect(screen.queryByTestId("mcp-card-mcp:fs")).toBeNull();
    expect(await screen.findByTestId("action-enable-mcp:remote-api-codex")).toBeInTheDocument();
    expect(screen.queryAllByTestId(/-claude-code$/)).toHaveLength(0);

    await userEvent.clear(input);
    expect(await screen.findByTestId("mcp-card-mcp:fs")).toBeInTheDocument();
  });
});

describe("页内搜索：操作记录页", () => {
  it("按运行 ID 大小写不敏感过滤历史任务；无匹配时给出空态与清除入口", async () => {
    renderWithEngine(<RunsPage />, runsRoutes());
    const input = await screen.findByTestId("runs-search");
    expect(await screen.findByTestId("run-row-20260905T235900Z-legacy1")).toBeInTheDocument();
    expect(screen.getByTestId("run-row-20260906T010000Z-mock02")).toBeInTheDocument();

    await userEvent.type(input, "LEGACY");
    expect(screen.getByTestId("run-row-20260905T235900Z-legacy1")).toBeInTheDocument();
    expect(screen.queryByTestId("run-row-20260906T010000Z-mock02")).toBeNull();

    await userEvent.clear(input);
    await userEvent.type(input, "不存在的运行");
    expect(await screen.findByTestId("runs-search-empty")).toHaveTextContent("无匹配结果");

    await userEvent.click(screen.getByRole("button", { name: "清除搜索关键词" }));
    expect(screen.getByTestId("run-row-20260905T235900Z-legacy1")).toBeInTheDocument();
    expect(screen.getByTestId("run-row-20260906T010000Z-mock02")).toBeInTheDocument();
  });
});

describe("页内搜索：技能页", () => {
  it("集合列表按名称/地址过滤卡片", async () => {
    renderWithEngine(<SkillsPage />, skillsRoutes());
    const input = await screen.findByTestId("skills-search");
    expect(await screen.findByTestId("collection-card-web-tools")).toBeInTheDocument();
    expect(screen.getByTestId("collection-card-data-libs")).toBeInTheDocument();

    await userEvent.type(input, "web");
    expect(screen.getByTestId("collection-card-web-tools")).toBeInTheDocument();
    expect(screen.queryByTestId("collection-card-data-libs")).toBeNull();
  });

  it("全部技能：搜索与集合下拉筛选叠加（AND），无匹配时给出空态与清除入口", async () => {
    renderWithEngine(<SkillsPage />, skillsRoutes());
    await expandAll();

    const input = await screen.findByTestId("skills-search");
    await userEvent.type(input, "lint");
    expect(await screen.findByText("lint")).toBeInTheDocument();
    expect(screen.queryByText("query")).toBeNull();

    // 叠加集合筛选为另一集合：搜索词仍生效（AND）→ 无匹配结果
    await chooseOption(screen.getByTestId("filter-collection-select"), "data-libs");
    expect(await screen.findByTestId("skills-empty-state")).toHaveTextContent("无匹配结果");

    await userEvent.click(screen.getByRole("button", { name: "清除搜索关键词" }));
    expect(await screen.findByText("query")).toBeInTheDocument();
  });
});
