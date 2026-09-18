import { describe, expect, it } from "vitest";
import { waitFor } from "@testing-library/react";
import TakeoverPage from "../pages/TakeoverPage";
import type { DiscoverData } from "../lib/contract";
import { makeApplyResult, makeEnvelope } from "./fixtures";
import { renderWithEngine, screen, userEvent } from "./render-with-engine";

function discoverData(overrides: Partial<DiscoverData> = {}): DiscoverData {
  return {
    domain: "discover",
    agents: [
      {
        id: "opencode",
        display_name: "OpenCode",
        bin: "opencode",
        bin_path: "~/.local/bin/opencode",
        version: "1.2.3",
        origin: "native",
        npm_package: null,
        manage: "register",
        managed_after_takeover: false,
        suggested_definition: { display_name: "OpenCode", bin: "opencode", preferred_provider: "native", providers: { native: { managed: false } } },
      },
    ],
    skills: {
      items: [
        {
          agent: "claude-code",
          name: "ones",
          link_path: "~/.claude/skills/ones",
          target: "~/.vendor/skills/ones",
          origin: "external-link",
          manage: "symlink",
          collection: null,
          group: "~/.vendor/skills",
          suggested_collection: "vendor",
          has_skill_md: true,
        },
        {
          agent: "claude-code",
          name: "safe-merge-review",
          link_path: "~/.claude/skills/safe-merge-review",
          target: "~/elsewhere/safe-merge-review",
          origin: "collection-extra-link",
          manage: "collection",
          collection: "agent-skills",
          group: null,
          has_skill_md: true,
        },
        {
          agent: "claude-code",
          name: "synced",
          link_path: "~/.claude/skills/synced",
          target: null,
          origin: "native-dir",
          manage: "native",
          collection: null,
          group: null,
          has_skill_md: false,
        },
        {
          agent: "codex",
          name: "ghost",
          link_path: "~/.codex/skills/ghost",
          target: "~/gone/ghost",
          origin: "dead-link",
          manage: "cleanup",
          collection: null,
          group: null,
          has_skill_md: false,
        },
      ],
      groups: [
        {
          name: "vendor",
          source_path: "~/.vendor/skills",
          manage: "symlink",
          existing_collection: false,
          agents: ["claude-code"],
          skills: [{ name: "ones", agents: ["claude-code"], has_skill_md: true }],
          suggested_definition: { source_path: "~/.vendor/skills", skill_paths: ["ones"], agents: ["claude-code"] },
        },
      ],
    },
    mcp: [
      {
        name: "grok-search",
        agent: "claude-code",
        transport: "stdio",
        spec: { command: "uvx", args: ["--from", "git+https://example.com/x", "grok"], env: { GROK_API_KEY: "<literal>" } },
        previously_managed: true,
        origin: "previously-managed",
        manage: "register",
      },
      {
        name: "grok-search",
        agent: "codex",
        transport: "stdio",
        spec: { command: "uvx", args: ["--from", "git+https://example.com/x", "grok"], env: { GROK_API_KEY: "<literal>" } },
        previously_managed: true,
        origin: "previously-managed",
        manage: "register",
      },
    ],
    summary: { agents: 1, skills_symlink: 1, skills_collection: 1, skills_native: 1, skills_cleanup: 1, mcp: 2 },
    ...overrides,
  };
}

function routes(data: DiscoverData = discoverData()) {
  return {
    "system:discover": makeEnvelope(data, { command: "system.discover" }),
    "policy:set": makeEnvelope({ layers: null, effective: {} }, { command: "policy.set" }),
    "skills:apply": makeEnvelope(makeApplyResult({ domain: "skills" }), { command: "skills.apply" }),
    "mcp:apply": makeEnvelope(makeApplyResult({ domain: "mcp" }), { command: "mcp.apply" }),
  };
}

describe("接管中心", () => {
  it("按来源分类展示，原生安装项只标识不可勾选", async () => {
    renderWithEngine(<TakeoverPage />, routes());
    await waitFor(() => expect(screen.getByTestId("takeover-agents")).toBeInTheDocument());

    // 可接管项带勾选框
    expect(screen.getByTestId("takeover-agent-opencode").querySelector("input[type=checkbox]")).toBeTruthy();
    expect(screen.getByTestId("takeover-skill-group-vendor").querySelector("input[type=checkbox]")).toBeTruthy();
    expect(screen.getByTestId("takeover-mcp-grok-search").querySelector("input[type=checkbox]")).toBeTruthy();

    // 原生管理项只展示，不提供勾选（它归各自的安装方式管）
    expect(screen.getByTestId("takeover-native-synced-claude-code").querySelector("input[type=checkbox]")).toBeNull();
    expect(screen.getByTestId("takeover-native-synced-claude-code")).toHaveTextContent("原生管理");
    // 残留链接可勾选，但走的是清理而不是纳管
    expect(screen.getByTestId("takeover-dead-ghost-codex").querySelector("input[type=checkbox]")).toBeTruthy();
    // 非 npm 来源的助手登记后保持原生管理方式
    expect(screen.getByTestId("takeover-agent-opencode")).toHaveTextContent("仅登记标识，保持原生管理");
  });

  it("外部技能组接管：先登记本地来源集合定义，再逐助手 add（不带接管旗标）", async () => {
    const { calls } = renderWithEngine(<TakeoverPage />, routes());
    await waitFor(() => expect(screen.getByTestId("takeover-skill-group-vendor")).toBeInTheDocument());

    await userEvent.click(screen.getByTestId("takeover-skill-group-vendor").querySelector("input[type=checkbox]")!);
    await userEvent.click(screen.getByTestId("takeover-run"));
    await userEvent.click(await screen.findByRole("button", { name: "开始接管" }));

    await waitFor(() => expect(calls.some((call) => call.command === "skills:apply")).toBe(true));
    const policyCall = calls.find((call) => call.command === "policy:set");
    expect(policyCall?.args).toEqual([
      "--layer",
      "local",
      "--key",
      "skills.repositories.vendor",
      "--value",
      JSON.stringify({ source_path: "~/.vendor/skills", skill_paths: ["ones"], agents: ["claude-code"], _delete: false, _replace: true }),
    ]);
    const applyCall = calls.find((call) => call.command === "skills:apply");
    expect(applyCall?.args).toEqual(["--only", "vendor/ones", "--agent", "claude-code", "--action", "add"]);
    // 登记源根后链接已在受管基准内，无需显式接管旗标
    expect(applyCall?.args).not.toContain("--allow-takeover");
  });

  it("已定义集合的计划外链接走既有接管通道（带 --allow-takeover）", async () => {
    const { calls } = renderWithEngine(<TakeoverPage />, routes());
    await waitFor(() => expect(screen.getByTestId("takeover-skill-extra-safe-merge-review-claude-code")).toBeInTheDocument());

    await userEvent.click(
      screen.getByTestId("takeover-skill-extra-safe-merge-review-claude-code").querySelector("input[type=checkbox]")!,
    );
    await userEvent.click(screen.getByTestId("takeover-run"));
    expect(await screen.findByText(/会替换助手里的同名外部条目/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "开始接管" }));

    await waitFor(() => expect(calls.some((call) => call.command === "skills:apply")).toBe(true));
    expect(calls.find((call) => call.command === "skills:apply")?.args).toEqual([
      "--only",
      "agent-skills/safe-merge-review",
      "--agent",
      "claude-code",
      "--action",
      "add",
      "--allow-takeover",
    ]);
  });

  it("MCP 接管：同名条目合并为一条定义（agents 含全部来源），再逐助手写回", async () => {
    const { calls } = renderWithEngine(<TakeoverPage />, routes());
    await waitFor(() => expect(screen.getByTestId("takeover-mcp-grok-search")).toBeInTheDocument());

    await userEvent.click(screen.getByTestId("takeover-mcp-grok-search").querySelector("input[type=checkbox]")!);
    await userEvent.click(screen.getByTestId("takeover-run"));
    await userEvent.click(await screen.findByRole("button", { name: "开始接管" }));

    await waitFor(() => expect(calls.some((call) => call.command === "mcp:apply")).toBe(true));
    const policyCall = calls.find((call) => call.command === "policy:set");
    const saved = JSON.parse(policyCall!.args[5]) as Record<string, unknown>;
    expect(policyCall!.args[3]).toBe("mcp.definitions.grok-search");
    expect(saved.agents).toEqual(["claude-code", "codex"]);
    // 密钥只以变量引用入库，明文绝不出现
    expect(JSON.stringify(saved)).toContain("${GROK_API_KEY}");
    expect(JSON.stringify(saved)).not.toContain("<literal>");
    const mcpCalls = calls.filter((call) => call.command === "mcp:apply");
    expect(mcpCalls).toHaveLength(2);
    expect(mcpCalls[0].args).toEqual(["--only", "mcp:grok-search", "--agent", "claude-code", "--action", "add", "--allow-takeover"]);
    expect(mcpCalls[1].args).toEqual(["--only", "mcp:grok-search", "--agent", "codex", "--action", "add", "--allow-takeover"]);
  });

  it("助手登记只写定义，不执行任何 agents:apply", async () => {
    const { calls } = renderWithEngine(<TakeoverPage />, routes());
    await waitFor(() => expect(screen.getByTestId("takeover-agent-opencode")).toBeInTheDocument());

    await userEvent.click(screen.getByTestId("takeover-agent-opencode").querySelector("input[type=checkbox]")!);
    await userEvent.click(screen.getByTestId("takeover-run"));
    await userEvent.click(await screen.findByRole("button", { name: "开始接管" }));

    await waitFor(() => expect(calls.some((call) => call.command === "policy:set")).toBe(true));
    expect(calls.find((call) => call.command === "policy:set")?.args[3]).toBe("agents.definitions.opencode");
    expect(calls.some((call) => call.command === "agents:apply")).toBe(false);
  });

  it("残留链接可勾选清理：只删链接本身，确认文案分开说明", async () => {
    const { calls } = renderWithEngine(<TakeoverPage />, {
      ...routes(),
      "skills:apply": makeEnvelope(makeApplyResult({ domain: "skills" }), { command: "skills.apply" }),
    });
    await waitFor(() => expect(screen.getByTestId("takeover-dead-ghost-codex")).toBeInTheDocument());

    await userEvent.click(screen.getByTestId("takeover-dead-ghost-codex").querySelector("input[type=checkbox]")!);
    await userEvent.click(screen.getByTestId("takeover-run"));
    expect(await screen.findByText(/条残留链接会被删除（只删链接本身，不动源目录）/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "开始接管" }));

    await waitFor(() => expect(calls.some((call) => call.command === "skills:apply")).toBe(true));
    expect(calls.find((call) => call.command === "skills:apply")?.args).toEqual([
      "--action",
      "remove-link",
      "--agent",
      "codex",
      "--link",
      "ghost",
    ]);
  });

  it("批量执行时显示进度：第几项、正在做哪一项", async () => {
    // 卡住第一条 policy:set，让进度停在第 1 项上可断言
    let release: (() => void) | null = null;
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    renderWithEngine(<TakeoverPage />, {
      ...routes(),
      "policy:set": async () => {
        await gate;
        return makeEnvelope({ layers: null, effective: {} }, { command: "policy.set" });
      },
    });
    await waitFor(() => expect(screen.getByTestId("takeover-agent-opencode")).toBeInTheDocument());

    // 两项：助手登记 + MCP，总数因此是 2
    await userEvent.click(screen.getByTestId("takeover-agent-opencode").querySelector("input[type=checkbox]")!);
    await userEvent.click(screen.getByTestId("takeover-mcp-grok-search").querySelector("input[type=checkbox]")!);
    await userEvent.click(screen.getByTestId("takeover-run"));
    await userEvent.click(await screen.findByRole("button", { name: "开始接管" }));

    const progress = await screen.findByTestId("takeover-progress");
    expect(progress).toHaveTextContent("正在处理第 1 / 2 项：助手 OpenCode");
    expect(screen.getByTestId("takeover-run")).toHaveTextContent("正在接管 1/2…");

    release!();
    await waitFor(() => expect(screen.queryByTestId("takeover-progress")).toBeNull());
  });

  it("没有可接管项时按钮禁用且空态可见", async () => {
    const empty: DiscoverData = {
      domain: "discover",
      agents: [],
      skills: { items: [], groups: [] },
      mcp: [],
      summary: {},
    };
    renderWithEngine(<TakeoverPage />, routes(empty));
    await waitFor(() => expect(screen.getByTestId("takeover-agents-empty")).toBeInTheDocument());
    expect(screen.getByTestId("takeover-skill-groups-empty")).toBeInTheDocument();
    expect(screen.getByTestId("takeover-mcp-empty")).toBeInTheDocument();
    expect(screen.getByTestId("takeover-run")).toBeDisabled();
  });
});
