import type { McpTarget, StatusData } from "../lib/contract";
import type { McpAgentCell } from "../lib/observed";
import { makeElement, makeFreshness } from "./fixtures";

export const BASE_CLAUDE_FS = "a".repeat(64);
export const BASE_CODEX_FS = "b".repeat(64);
export const BASE_CODEX_REMOTE = "c".repeat(64);
export const BASE_CLAUDE_SNAPSHOT = "d".repeat(64);

export function mcpTargets(overrides: Partial<Record<string, Partial<McpTarget>>> = {}): McpTarget[] {
  return [
    {
      agent: "claude-code",
      path: "/tmp/demo/.claude.json",
      format: "claude-json",
      exists: true,
      agent_enabled: true,
      disable_mode: "snapshot",
      error: null,
      ...overrides["claude-code"],
    },
    {
      agent: "codex",
      path: "/tmp/demo/.codex/config.toml",
      format: "codex-toml",
      exists: true,
      agent_enabled: true,
      disable_mode: "field",
      error: null,
      ...overrides.codex,
    },
  ];
}

export function mcpCell(overrides: Partial<McpAgentCell> = {}): McpAgentCell {
  return {
    state: "enabled",
    source: "file",
    target: "/tmp/demo/.claude.json",
    target_exists: true,
    transport: "stdio",
    entry: { command: "npx", args: ["-y", "fs-mcp"], env: {} },
    extra: {},
    vars: {},
    base: BASE_CLAUDE_FS,
    legacy_marker: false,
    has_saved: false,
    command_available: true,
    notice: null,
    blocked_reason: null,
    actions: ["edit", "disable", "copy", "remove"],
    ...overrides,
  };
}

/**
 * 三个服务：
 * - fs：Claude 与 Codex 各有一条、内容不同（Codex 带 cwd 等未建模字段、密钥为字面量）；
 * - remote-api：只在 Codex，已用官方 enabled=false 停用；
 * - old-snap：Claude 停用快照 + 一份未分配的已保存定义。
 */
export function mcpStatusData(): StatusData {
  return {
    domain: "mcp",
    targets: mcpTargets(),
    elements: [
      makeElement({
        id: "mcp:fs",
        domain: "mcp",
        spec: { name: "fs", saved: null },
        observed: {
          agents: {
            "claude-code": mcpCell({
              entry: { command: "npx", args: ["-y", "fs-mcp"], env: { API_KEY: "${FS_API_KEY}" } },
              vars: { API_KEY: { placeholder: "${FS_API_KEY}", resolved: false, stored: false, kind: "env" } },
            }),
            codex: mcpCell({
              target: "/tmp/demo/.codex/config.toml",
              entry: { command: "npx", args: ["-y", "fs-mcp@2"], env: { TOKEN: "<literal>" }, startup_timeout_sec: 30 },
              extra: { cwd: "/work", enabled_tools: ["read"], tools: "<hidden>" },
              base: BASE_CODEX_FS,
              legacy_marker: true,
              notice: "条目含旧版 Agent Manager 标记，编辑或启停时会顺带清除",
            }),
          },
        },
        state: "managed-ok",
        freshness: makeFreshness({ state: "behind", current: "1.2.0", upstream: "1.8.0", checked_at: "2026-09-05T10:00:00Z", probe: "npm-view", cache_age_sec: 60 }),
      }),
      makeElement({
        id: "mcp:remote-api",
        domain: "mcp",
        spec: { name: "remote-api", saved: null },
        observed: {
          agents: {
            codex: mcpCell({
              state: "disabled",
              target: "/tmp/demo/.codex/config.toml",
              transport: "remote",
              entry: { url: "https://mcp.example.com/mcp", headers: { Authorization: "${REMOTE_TOKEN}", "X-Static": "<literal>" } },
              base: BASE_CODEX_REMOTE,
              command_available: null,
              actions: ["edit", "enable", "copy", "remove"],
            }),
          },
        },
        state: "disabled",
        freshness: makeFreshness({ state: "not-applicable" }),
      }),
      makeElement({
        id: "mcp:old-snap",
        domain: "mcp",
        spec: {
          name: "old-snap",
          saved: {
            definition: { command: "uvx", args: ["old-snap"], env: { KEY: "${OLD_KEY}" }, extra_args_by_agent: { codex: ["--codex"] } },
            valid: true,
          },
        },
        observed: {
          agents: {
            "claude-code": mcpCell({
              state: "disabled",
              source: "saved",
              entry: { command: "uvx", args: ["old-snap"], env: { KEY: "${OLD_KEY}", NODE_ENV: "<literal>" } },
              base: BASE_CLAUDE_SNAPSHOT,
              command_available: null,
              notice: "已停用：条目保存在本工具里，启用时写回配置文件",
              actions: ["enable", "copy", "remove"],
            }),
          },
        },
        state: "disabled",
        freshness: makeFreshness({ state: "not-applicable" }),
        actions: [{ id: "forget", risk: "write-local", needs_lock: true, offline_ok: true }],
      }),
    ],
  };
}
