import { describe, expect, it } from "vitest";
import AgentsPage from "../pages/AgentsPage";
import McpPage from "../pages/McpPage";
import RunsPage from "../pages/RunsPage";
import SkillsPage from "../pages/SkillsPage";
import type { Envelope, StatusData } from "../lib/contract";
import {
  makeAuditData,
  makeElement,
  makeEnvelope,
  makeFreshness,
  makeMode,
  makePolicyData,
  makeRunEntry,
} from "./fixtures";
import { renderWithEngine, screen, userEvent } from "./render-with-engine";

function agentsRoutes(): Record<string, Envelope> {
  return {
    "system:audit": makeEnvelope(makeAuditData(), { command: "system.audit", mode: makeMode() }),
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
            freshness: makeFreshness(),
            actions: [],
          }),
          makeElement({
            id: "agents:codex",
            domain: "agents",
            spec: { display_name: "Codex", bin: "codex", preferred_provider: "native", providers: { native: { managed: true } } },
            policy: { enabled: true },
            observed: { install_source: "native", installed: true },
            state: "managed-ok",
            freshness: makeFreshness(),
            actions: [],
          }),
        ],
      },
      { command: "agents.status" },
    ),
  };
}

function mcpRoutes(): Record<string, Envelope> {
  const status: StatusData = {
    domain: "mcp",
    elements: [
      makeElement({
        id: "mcp:chrome-devtools",
        domain: "mcp",
        spec: { command: "npx", args: ["-y", "chrome-devtools-mcp"], startup_timeout_sec: 120, agents: null, env: {} },
        observed: {
          agents: {
            "claude-code": { state: "drifted", target: "/tmp/demo/.claude.json", target_exists: true, assigned: true, enabled: true, actions: ["update"] },
            codex: { state: "managed-ok", target: "/tmp/demo/.codex/config.toml", target_exists: true, assigned: true, enabled: true, actions: ["disable", "remove"] },
          },
          env: {},
          actions: ["enable", "disable", "remove"],
        },
        state: "drifted",
      }),
      makeElement({
        id: "mcp:serena",
        domain: "mcp",
        spec: { command: "uvx", args: ["serena"], startup_timeout_sec: 60, agents: null, env: {} },
        observed: {
          agents: {
            "claude-code": { state: "leftover", target: "/tmp/demo/.claude.json", target_exists: true, assigned: false, enabled: false, actions: ["remove"] },
            codex: { state: "not-materialized", target: "/tmp/demo/.codex/config.toml", target_exists: false, assigned: true, enabled: true, actions: [] },
          },
          env: {},
          actions: ["disable", "remove"],
        },
        state: "managed-ok",
      }),
    ],
  };
  return {
    "mcp:status": makeEnvelope(status, { command: "mcp.status" }),
    "agents:status": makeEnvelope(
      {
        domain: "agents",
        elements: [
          makeElement({ id: "agents:claude-code", domain: "agents" }),
          makeElement({ id: "agents:codex", domain: "agents" }),
        ],
      },
      { command: "agents.status" },
    ),
  };
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

describe("页内搜索：助手页", () => {
  it("按名称大小写不敏感过滤卡片；无匹配时给出空态与清除入口", async () => {
    renderWithEngine(<AgentsPage />, agentsRoutes());
    const input = await screen.findByTestId("agents-search");
    expect(screen.getByRole("heading", { name: /Claude Code/ })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: /Codex/ })).toBeInTheDocument();

    await userEvent.type(input, "CLAUDE");
    expect(screen.getByRole("heading", { name: /Claude Code/ })).toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: /Codex/ })).toBeNull();

    await userEvent.clear(input);
    await userEvent.type(input, "不存在的助手");
    expect(await screen.findByTestId("agents-search-empty")).toHaveTextContent("无匹配结果");

    await userEvent.click(screen.getByRole("button", { name: "清除搜索关键词" }));
    expect(screen.getByRole("heading", { name: /Claude Code/ })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: /Codex/ })).toBeInTheDocument();
  });
});

describe("页内搜索：MCP 页", () => {
  it("服务库按名称/命令过滤卡片；无匹配时给出空态与清除入口", async () => {
    renderWithEngine(<McpPage />, mcpRoutes());
    const input = await screen.findByTestId("mcp-search");
    expect(await screen.findByRole("button", { name: "mcp:chrome-devtools" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "mcp:serena" })).toBeInTheDocument();

    await userEvent.type(input, "Serena");
    expect(screen.queryByRole("button", { name: "mcp:chrome-devtools" })).toBeNull();
    expect(screen.getByRole("button", { name: "mcp:serena" })).toBeInTheDocument();

    await userEvent.clear(input);
    await userEvent.type(input, "不存在的服务");
    expect(await screen.findByTestId("mcp-search-empty")).toHaveTextContent("无匹配结果");

    await userEvent.click(screen.getByRole("button", { name: "清除搜索关键词" }));
    expect(screen.getByRole("button", { name: "mcp:chrome-devtools" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "mcp:serena" })).toBeInTheDocument();
  });

  it("助手中的服务：搜索与助手下拉筛选叠加（AND）", async () => {
    renderWithEngine(<McpPage />, mcpRoutes());
    await userEvent.click(await screen.findByRole("tab", { name: "助手中的服务" }));

    await userEvent.selectOptions(screen.getByTestId("filter-agent-select"), "claude-code");
    const input = await screen.findByTestId("mcp-search");
    await userEvent.type(input, "serena");

    // AND 生效：仅剩 serena × claude-code（leftover），chrome-devtools × claude-code 被搜索过滤
    expect(await screen.findByTitle("claude-code: leftover")).toBeInTheDocument();
    expect(screen.queryByTitle("claude-code: drifted")).toBeNull();
    expect(screen.queryByTitle("codex: managed-ok")).toBeNull();

    await userEvent.clear(input);
    expect(await screen.findByTitle("claude-code: drifted")).toBeInTheDocument();
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
    await userEvent.click(await screen.findByRole("tab", { name: "全部技能" }));

    const input = await screen.findByTestId("skills-search");
    await userEvent.type(input, "lint");
    expect(await screen.findByText("lint")).toBeInTheDocument();
    expect(screen.queryByText("query")).toBeNull();

    // 叠加集合筛选为另一集合：搜索词仍生效（AND）→ 无匹配结果
    await userEvent.selectOptions(screen.getByTestId("filter-collection-select"), "data-libs");
    expect(await screen.findByTestId("skills-empty-state")).toHaveTextContent("无匹配结果");

    await userEvent.click(screen.getByRole("button", { name: "清除搜索关键词" }));
    expect(await screen.findByText("query")).toBeInTheDocument();
  });
});
