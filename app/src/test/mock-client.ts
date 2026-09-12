import type { Envelope, RunEntry, SnapshotEntry } from "../lib/contract";
import { EngineBridgeError, type EngineClient } from "../lib/engine";
import { makeAuditData, makeElement, makeEnvelope, makeFreshness, makeMode, makeRunEntry, makeSnapshotEntry } from "./fixtures";

/**
 * dev-only mock 引擎（?mock=1 / ?mock=offline，仅在 import.meta.env.DEV 下经
 * initEngineClient 动态加载；生产构建 treeshake 掉本模块）。
 * fixture 集覆盖：freshness 五态、收敛七态、offline 模式变体。
 */

const CHECKED_AT = "2026-09-05T10:00:00Z";

function demoEnvelopes(): Record<string, Envelope> {
  const audit = makeEnvelope(
    makeAuditData({
      status: "warning",
      sections: {
        config: [{ name: "manager.json", status: "ok", message: "格式正确" }],
        runtime: [{ name: "engine-rs", status: "ok", message: "Rust 原生引擎" }],
        agents: [{ name: "Old Agent", status: "notice", message: "未声明策略（已忽略）" }],
        tools: [{ name: "demo-cli", status: "warning", message: "可选工具未就绪", category: "optional_tool" }],
        mcp: [{ name: "Kimi Code", status: "warning", message: "托管产物尚未生成", category: "unmaterialized" }],
        skills: [{ name: "Codex", status: "error", message: "skill 目标漂移 (dead-one)" }],
      },
      summary: {
        errors: 1,
        warnings: 2,
        warning_categories: { optional_tools: 1, unmaterialized: 1, official_gaps: 0 },
      },
    }),
    { command: "system.audit" },
  );

  const agentsStatus = makeEnvelope(
    {
      domain: "agents",
      elements: [
        makeElement({
          id: "agents:claude-code",
          domain: "agents",
          spec: {
            display_name: "Claude Code",
            bin: "claude",
            preferred_provider: "native",
            providers: { native: { managed: true }, brew: { managed: false } },
          },
          policy: { enabled: true, auto_install: true, auto_update: true, preferred_provider: "native" },
          observed: {
            providers: ["native"],
            classified: { managed: ["native"], deprecated: [], system_only: [], unsupported: [] },
            installed: true,
            preferred_active: true,
          },
          state: "managed-ok",
          freshness: makeFreshness({ state: "behind", current: "2.0.0", upstream: "2.1.3", checked_at: CHECKED_AT, probe: "native", cache_age_sec: 300 }),
          actions: [{ id: "upgrade", risk: "network", needs_lock: true, offline_ok: false }],
        }),
        makeElement({
          id: "agents:codex",
          domain: "agents",
          spec: { display_name: "Codex", bin: "codex", preferred_provider: "native", providers: { native: { managed: true } } },
          policy: { enabled: true, auto_install: true, auto_update: true, preferred_provider: "native" },
          observed: { providers: ["native"], classified: { managed: ["native"], deprecated: [], system_only: [], unsupported: [] }, installed: true, preferred_active: true },
          state: "managed-ok",
          freshness: makeFreshness({ state: "current", current: "0.42.0", upstream: "0.42.0", checked_at: CHECKED_AT, probe: "native", cache_age_sec: 300 }),
        }),
        makeElement({
          id: "agents:kimi-code",
          domain: "agents",
          spec: { display_name: "Kimi Code", bin: "kimi", preferred_provider: "npm", providers: { npm: { managed: true }, brew: { managed: false } } },
          policy: { enabled: true, auto_install: false, auto_update: false, preferred_provider: "npm" },
          observed: { providers: ["brew"], classified: { managed: [], deprecated: [], system_only: [], unsupported: ["brew"] }, installed: true, preferred_active: false },
          state: "unmanaged-detected",
          freshness: makeFreshness({ state: "unknown", current: null, upstream: null, checked_at: null, probe: null, cache_age_sec: null }),
        }),
        makeElement({
          id: "agents:old-agent",
          domain: "agents",
          spec: { display_name: "Old Agent", bin: "old", preferred_provider: "npm", providers: { npm: { managed: true } } },
          policy: { enabled: false, auto_install: false, auto_update: false },
          observed: { providers: [], classified: { managed: [], deprecated: [], system_only: [], unsupported: [] }, installed: false, preferred_active: false },
          state: "disabled",
        }),
        makeElement({
          id: "agents:qoder",
          domain: "agents",
          spec: { display_name: "Qoder", bin: "qoder", preferred_provider: "npm", providers: { npm: { managed: true } } },
          policy: { enabled: true, auto_install: false, auto_update: false },
          observed: { providers: [], classified: { managed: [], deprecated: [], system_only: [], unsupported: [] }, installed: false, preferred_active: false },
          state: "missing",
        }),
      ],
    },
    { command: "agents.status" },
  );

  const toolsStatus = makeEnvelope(
    {
      domain: "tools",
      elements: [
        makeElement({
          id: "tools:playwright-cli",
          domain: "tools",
          state: "managed-ok",
          freshness: makeFreshness({ state: "current", current: "0.0.66", upstream: "0.0.66", checked_at: CHECKED_AT, probe: "npm-view", cache_age_sec: 120 }),
        }),
        makeElement({
          id: "tools:demo-cli",
          domain: "tools",
          state: "managed-ok",
          freshness: makeFreshness({ state: "ahead", current: "9.9.9-beta", upstream: "9.9.8", checked_at: CHECKED_AT, probe: "npm-view", cache_age_sec: 90000 }),
        }),
        makeElement({
          id: "tools:pending-cli",
          domain: "tools",
          state: "not-materialized",
          freshness: makeFreshness({ state: "not-applicable" }),
        }),
        makeElement({
          id: "tools:legacy-cli",
          domain: "tools",
          state: "unsupported",
          freshness: makeFreshness({ state: "not-applicable" }),
        }),
      ],
    },
    { command: "tools.status" },
  );

  const mcpStatus = makeEnvelope(
    {
      domain: "mcp",
      elements: [
        makeElement({
          id: "mcp:chrome-devtools",
          domain: "mcp",
          spec: { command: "npx", args: ["-y", "chrome-devtools-mcp", "--isolated"], startup_timeout_sec: 120, agents: null, env: {} },
          policy: { enabled: true, agents: null },
          observed: {
            agents: {
              "claude-code": { state: "drifted", target: "/Users/demo/.claude.json", target_exists: true },
              codex: { state: "managed-ok", target: "/Users/demo/.codex/config.toml", target_exists: true },
              "kimi-code": { state: "missing", target: "/Users/demo/.kimi-code/mcp.json", target_exists: true },
            },
            env: {},
          },
          state: "drifted",
          freshness: makeFreshness({ state: "behind", current: "1.2.0", upstream: "1.8.0", checked_at: CHECKED_AT, probe: "npm-view", cache_age_sec: 1820 }),
        }),
        makeElement({
          id: "mcp:grok-search",
          domain: "mcp",
          spec: {
            command: "uvx",
            args: ["--from", "git+https://github.com/GuDaStudio/GrokSearch@grok-with-tavily", "grok-search"],
            startup_timeout_sec: 120,
            agents: null,
            env: { GROK_API_KEY: "${GROK_API_KEY}", TAVILY_API_KEY: "${TAVILY_API_KEY}" },
          },
          policy: { enabled: true, agents: null },
          observed: {
            agents: {
              "claude-code": { state: "managed-ok", target: "/Users/demo/.claude.json", target_exists: true },
              codex: { state: "managed-ok", target: "/Users/demo/.codex/config.toml", target_exists: true },
              "kimi-code": { state: "managed-ok", target: "/Users/demo/.kimi-code/mcp.json", target_exists: true },
            },
            env: {
              GROK_API_KEY: { placeholder: "GROK_API_KEY", resolved: true },
              TAVILY_API_KEY: { placeholder: "TAVILY_API_KEY", resolved: false },
            },
          },
          state: "managed-ok",
          freshness: makeFreshness({ state: "unknown", current: null, upstream: "8f0ae3c7f25ec419631ade7e431fa2531503433e", checked_at: CHECKED_AT, probe: "git-ls-remote", cache_age_sec: 4000 }),
        }),
        makeElement({
          id: "mcp:my-proxy",
          domain: "mcp",
          spec: { command: "npx", args: ["-y", "my-proxy-mcp"], startup_timeout_sec: 60, agents: null, env: { MY_PROXY_TOKEN: "${MY_PROXY_TOKEN}" } },
          policy: { enabled: true, agents: null },
          observed: {
            agents: {
              "claude-code": { state: "managed-ok", target: "/Users/demo/.claude.json", target_exists: true },
              codex: { state: "missing", target: "/Users/demo/.codex/config.toml", target_exists: true },
              "kimi-code": { state: "missing", target: "/Users/demo/.kimi-code/mcp.json", target_exists: true },
            },
            env: { MY_PROXY_TOKEN: { placeholder: "MY_PROXY_TOKEN", resolved: true } },
          },
          state: "managed-ok",
          freshness: makeFreshness({ state: "current", current: "0.3.1", upstream: "0.3.1", checked_at: CHECKED_AT, probe: "npm-view", cache_age_sec: 60 }),
        }),
        makeElement({
          id: "mcp:serena",
          domain: "mcp",
          spec: {
            command: "uvx",
            args: ["--from", "git+https://github.com/oraios/serena@main", "serena", "start-mcp-server"],
            extra_args_by_agent: { "claude-code": ["--context", "claude-code"], codex: ["--context", "codex"] },
            startup_timeout_sec: 120,
            agents: null,
            env: {},
          },
          policy: { enabled: true, agents: null },
          observed: {
            agents: {
              "claude-code": { state: "managed-ok", target: "/Users/demo/.claude.json", target_exists: true },
              codex: { state: "leftover", target: "/Users/demo/.codex/config.toml", target_exists: true },
              "kimi-code": { state: "not-materialized", target: "/Users/demo/.kimi-code/mcp.json", target_exists: false },
            },
            env: {},
          },
          state: "drifted",
          freshness: makeFreshness({ state: "current", current: "a1b2c3d", upstream: "a1b2c3d", checked_at: CHECKED_AT, probe: "git-ls-remote", cache_age_sec: 200 }),
        }),
      ],
    },
    { command: "mcp.status" },
  );

  const skillsStatus = makeEnvelope(
    {
      domain: "skills",
      elements: [
        makeElement({
          id: "skill:agent-skills",
          domain: "skills",
          spec: {
            url: "https://github.com/example/agent-skills.git",
            skill_paths: ["safe-merge-review", "artifact-hygiene", "dead-one"],
            agents: ["claude-code", "codex", "kimi-code"],
            matrix_mode: true,
            new_skill_default: "disabled",
          },
          policy: { enabled: true },
          observed: {
            repo_cache: "/repo/agent-skill/repositories/agent-skills",
            repo_cached: true,
            owner_marked: true,
            discovered: ["safe-merge-review", "artifact-hygiene", "dead-one", "new-hotness", "experimental-lint"],
            discovery_inbox: ["new-hotness", "experimental-lint"],
            extra_links: [],
          },
          state: "managed-ok",
          freshness: makeFreshness({ state: "behind", current: "a1b2c3d4e5f60718293a4b5c6d7e8f9012a3b4c5", upstream: "f0e1d2c3b4a5968778695a4b3c2d1e0f9a8b7c6d", checked_at: CHECKED_AT, probe: "git-ls-remote", cache_age_sec: 600 }),
        }),
        makeElement({
          id: "skill:agent-skills/safe-merge-review",
          domain: "skills",
          spec: { collection: "agent-skills", name: "safe-merge-review", source_path: "/repo/agent-skill/repositories/agent-skills/safe-merge-review" },
          policy: { enabled: true, agents: { "claude-code": true, codex: true, "kimi-code": true } },
          observed: {
            agents: {
              "claude-code": { state: "drifted", link_path: "/Users/demo/.claude/skills/safe-merge-review", target_dir_exists: true },
              codex: { state: "linked", link_path: "/Users/demo/.codex/skills/safe-merge-review", target_dir_exists: true },
              "kimi-code": { state: "missing-link", link_path: "/Users/demo/.kimi-code/skills/safe-merge-review", target_dir_exists: true },
            },
            discovery_inbox: false,
          },
          state: "drifted",
        }),
        makeElement({
          id: "skill:agent-skills/artifact-hygiene",
          domain: "skills",
          spec: { collection: "agent-skills", name: "artifact-hygiene", source_path: "/repo/agent-skill/repositories/agent-skills/artifact-hygiene" },
          policy: { enabled: true, agents: { "claude-code": true, codex: true, "kimi-code": true } },
          observed: {
            agents: {
              "claude-code": { state: "linked", link_path: "/Users/demo/.claude/skills/artifact-hygiene", target_dir_exists: true },
              codex: { state: "linked", link_path: "/Users/demo/.codex/skills/artifact-hygiene", target_dir_exists: true },
              "kimi-code": { state: "linked", link_path: "/Users/demo/.kimi-code/skills/artifact-hygiene", target_dir_exists: true },
            },
            discovery_inbox: false,
          },
          state: "managed-ok",
        }),
        makeElement({
          id: "skill:agent-skills/dead-one",
          domain: "skills",
          spec: { collection: "agent-skills", name: "dead-one", source_path: "/repo/agent-skill/repositories/agent-skills/dead-one" },
          policy: { enabled: true, agents: { "claude-code": true, codex: false, "kimi-code": true } },
          observed: {
            agents: {
              "claude-code": { state: "dead-link", link_path: "/Users/demo/.claude/skills/dead-one", target_dir_exists: true },
              codex: { state: "disabled", link_path: "/Users/demo/.codex/skills/dead-one", target_dir_exists: true },
              "kimi-code": { state: "linked", link_path: "/Users/demo/.kimi-code/skills/dead-one", target_dir_exists: true },
            },
            discovery_inbox: false,
          },
          state: "drifted",
        }),
        makeElement({
          id: "skill:agent-skills/new-hotness",
          domain: "skills",
          spec: { collection: "agent-skills", name: "new-hotness", source_path: "/repo/agent-skill/repositories/agent-skills/new-hotness" },
          policy: { enabled: false, agents: { "claude-code": false, codex: false, "kimi-code": false } },
          observed: {
            agents: {
              "claude-code": { state: "disabled", link_path: "/Users/demo/.claude/skills/new-hotness", target_dir_exists: true },
              codex: { state: "disabled", link_path: "/Users/demo/.codex/skills/new-hotness", target_dir_exists: true },
              "kimi-code": { state: "disabled", link_path: "/Users/demo/.kimi-code/skills/new-hotness", target_dir_exists: true },
            },
            discovery_inbox: true,
          },
          state: "disabled",
        }),
        makeElement({
          id: "skill:agent-skills/experimental-lint",
          domain: "skills",
          spec: { collection: "agent-skills", name: "experimental-lint", source_path: "/repo/agent-skill/repositories/agent-skills/experimental-lint" },
          policy: { enabled: false, agents: {} },
          observed: { agents: {}, discovery_inbox: true },
          state: "disabled",
        }),
      ],
    },
    { command: "skills.status" },
  );

  const rulesStatus = makeEnvelope(
    {
      domain: "rules",
      elements: [
        makeElement({
          id: "rules:codex_global",
          domain: "rules",
          spec: { name: "Codex 全局规则", group: "global-rules", source_keys: ["rules.shared", "rules.codex"], target: "~/.codex/AGENTS.md" },
          policy: { enabled: true, agent_id: "codex" },
          observed: {
            target: "/Users/demo/.codex/AGENTS.md",
            target_exists: true,
            expected_digest: "aaaa3c8ae2687d47a8e7543d3c8c43ef8dd9f744e3a3e2ce9aa9ad7a90de12a2",
            applied_digest: "aaaa3c8ae2687d47a8e7543d3c8c43ef8dd9f744e3a3e2ce9aa9ad7a90de12a2",
            shared_revision: "1111",
            adapter_revision: "2222",
            blocked_reason: null,
            registration_needed: false,
          },
          state: "managed-ok",
          actions: [],
        }),
        makeElement({
          id: "rules:claude_global",
          domain: "rules",
          spec: { name: "Claude Code 全局规则", group: "global-rules", source_keys: ["rules.shared", "rules.claude"], target: "~/.claude/CLAUDE.md" },
          policy: { enabled: true, agent_id: "claude-code" },
          observed: {
            target: "/Users/demo/.claude/CLAUDE.md",
            target_exists: true,
            expected_digest: "bbbb4b5c130887752148845080715867e6c3e6f40e69189067dd7d92db8b40c36",
            applied_digest: "aaaa3c8ae2687d47a8e7543d3c8c43ef8dd9f744e3a3e2ce9aa9ad7a90de12a2",
            shared_revision: "1111",
            adapter_revision: "3333",
            blocked_reason: null,
            registration_needed: false,
          },
          state: "drifted",
          actions: [{ id: "sync", risk: "write-local", needs_lock: true, offline_ok: true }],
        }),
      ],
    },
    { command: "rules.status" },
  );

  const secretsStatus = makeEnvelope(
    {
      domain: "secrets",
      elements: [
        makeElement({
          id: "secrets:repo",
          domain: "secrets",
          spec: { path: "secrets/", kind: "git-subrepo" },
          policy: { enabled: true },
          observed: { path: "/repo/secrets", exists: true, is_git: true, env_files: ["mcp.env", "terminal.env"] },
          state: "managed-ok",
        }),
        makeElement({
          id: "secrets:mcp-env",
          domain: "secrets",
          spec: { sources: [".env", "secrets/mcp.env", "process env"] },
          policy: { enabled: true },
          observed: {
            variables: [
              { server: "grok-search", env_key: "GROK_API_KEY", placeholder: "GROK_API_KEY", resolved: true },
              { server: "grok-search", env_key: "TAVILY_API_KEY", placeholder: "TAVILY_API_KEY", resolved: false },
              { server: "grok-search", env_key: "FIRECRAWL_API_KEY", placeholder: "FIRECRAWL_API_KEY", resolved: false },
            ],
            unresolved: 2,
          },
          state: "managed-ok",
        }),
      ],
    },
    { command: "secrets.status" },
  );

  const mcpPlan = makeEnvelope(
    {
      domain: "mcp",
      actions: [
        {
          op: "write-file",
          element: "mcp:chrome-devtools",
          target: "/Users/demo/.claude.json",
          risk: "write-local",
          before_digest: "sha256:dd9d3c8ae2687d47a8e7543d3c8c43ef8dd9f744e3a3e2ce9aa9ad7a90de12a2",
          after_digest: "sha256:89b4b5c130887752148845080715867e6c3e6f40e69189067dd7d92db8b40c36",
          backup_path: null,
          cmd: null,
          note: "注入/收敛 mcp:chrome-devtools 到 agent 'claude-code'（before=目标文件整体 digest，after=注入段规范形 digest）",
        },
        {
          op: "write-file",
          element: "mcp:chrome-devtools",
          target: "/Users/demo/.kimi-code/mcp.json",
          risk: "write-local",
          before_digest: "sha256:60c71c56a3bead4e5535e8d2a6fce94bfbde198c428679f14351078d4cf7fdc0",
          after_digest: "sha256:89b4b5c130887752148845080715867e6c3e6f40e69189067dd7d92db8b40c36",
          backup_path: null,
          cmd: null,
          note: "注入/收敛 mcp:chrome-devtools 到 agent 'kimi-code'",
        },
      ],
      summary: { read: 0, "write-local": 2, "exec-local": 0, network: 0, "install-remove": 0 },
    },
    { command: "mcp.plan" },
  );

  const skillsPlan = makeEnvelope(
    {
      domain: "skills",
      actions: [
        {
          op: "symlink",
          element: "skill:agent-skills/safe-merge-review",
          target: "/Users/demo/.claude/skills/safe-merge-review",
          risk: "exec-local",
          before_digest: null,
          after_digest: null,
          backup_path: null,
          cmd: null,
          note: "链接重定向到 /repo/agent-skill/repositories/agent-skills/safe-merge-review",
        },
        {
          op: "unlink",
          element: "skill:agent-skills/dead-one",
          target: "/Users/demo/.claude/skills/dead-one",
          risk: "exec-local",
          before_digest: null,
          after_digest: null,
          backup_path: null,
          cmd: null,
          note: "移除死链",
        },
      ],
      summary: { read: 0, "write-local": 0, "exec-local": 2, network: 0, "install-remove": 0 },
    },
    { command: "skills.plan" },
  );

  const rulesPlan = makeEnvelope(
    {
      domain: "rules",
      actions: [
        {
          op: "write-file",
          element: "rules:claude_global",
          target: "/Users/demo/.claude/CLAUDE.md",
          risk: "write-local",
          before_digest: "aaaa3c8ae2687d47a8e7543d3c8c43ef8dd9f744e3a3e2ce9aa9ad7a90de12a2",
          after_digest: "bbbb4b5c130887752148845080715867e6c3e6f40e69189067dd7d92db8b40c36",
          backup_path: null,
          cmd: null,
          note: "同步 rules:claude_global 规则文件",
        },
      ],
      summary: { read: 0, "write-local": 1, "exec-local": 0, network: 0, "install-remove": 0 },
    },
    { command: "rules.plan" },
  );

  const agentsPlan = makeEnvelope(
    {
      domain: "agents",
      actions: [
        {
          op: "run-cmd",
          element: "agents:claude-code",
          target: null,
          risk: "network",
          before_digest: null,
          after_digest: null,
          backup_path: null,
          cmd: "claude update",
          note: "升级到上游最新 2.1.3（不锁版本）",
        },
        {
          op: "install",
          element: "agents:qoder",
          target: null,
          risk: "install-remove",
          before_digest: null,
          after_digest: null,
          backup_path: null,
          cmd: "npm install -g qoder",
          note: "安装缺失 agent",
        },
      ],
      summary: { read: 0, "write-local": 0, "exec-local": 0, network: 1, "install-remove": 1 },
    },
    { command: "agents.plan" },
  );

  const emptyPlan = (domain: string) =>
    makeEnvelope(
      { domain, actions: [], summary: { read: 0, "write-local": 0, "exec-local": 0, network: 0, "install-remove": 0 } },
      { command: `${domain}.plan` },
    );

  return {
    "system:audit": audit,
    "agents:status": agentsStatus,
    "tools:status": toolsStatus,
    "mcp:status": mcpStatus,
    "skills:status": skillsStatus,
    "rules:status": rulesStatus,
    "secrets:status": secretsStatus,
    "hooks:status": makeEnvelope({ domain: "hooks", elements: [] }, { command: "hooks.status" }),
    "aliases:status": makeEnvelope({ domain: "aliases", elements: [] }, { command: "aliases.status" }),
    "mcp:plan": mcpPlan,
    "skills:plan": skillsPlan,
    "rules:plan": rulesPlan,
    "agents:plan": agentsPlan,
    "tools:plan": emptyPlan("tools"),
    "hooks:plan": emptyPlan("hooks"),
    "aliases:plan": emptyPlan("aliases"),
    "system:refresh-upstream": makeEnvelope(
      {
        domain: "system",
        verb: "refresh-upstream",
        cache_file: "/tmp/mock-upstream-cache.json",
        elements: [
          { id: "agents:claude-code", probe: "native", state: "behind", current: "2.0.0", upstream: "2.1.3", error: null },
        ],
        summary: { probed: 6, current: 2, behind: 2, ahead: 1, unknown: 1 },
      },
      { command: "system.refresh-upstream" },
    ),
  };
}

export function createMockEngineClient(options: { offline?: boolean } = {}): EngineClient {
  const routes = demoEnvelopes();

  // ---- 有状态的写路径演示数据（内存态，仅 DEV mock） ----
  const builtinPolicy: Record<string, unknown> = {
      agents: { "claude-code": { enabled: true, auto_update: true } },
      mcp: { "grok-search": { enabled: true } },
      skills: { "agent-skills": { enabled: true, new_skill_default: "disabled" } },
  };
  const sqlitePolicy: Record<string, unknown> = {
      browser_us_sync: { enabled: false },
      mcp: {
        definitions: { "my-proxy": { command: "npx", args: ["-y", "my-proxy-mcp"], startup_timeout_sec: 60, agents: null, env: { MY_PROXY_TOKEN: "${MY_PROXY_TOKEN}" } } },
        "my-proxy": { enabled: true },
      },
  };
  const runs: RunEntry[] = [
    makeRunEntry({ run_id: "20260905T235900Z-legacy1", last_at: "2026-09-05T23:59:00Z", operations: 2 }),
  ];
  const snapshots: SnapshotEntry[] = [
    makeSnapshotEntry({ run_id: "20260905T230000Z-mock00", command: "mcp.apply", file_count: 3, link_count: 0 }),
    makeSnapshotEntry({ run_id: "20260905T235000Z-mock01", command: "system.converge", file_count: 8, link_count: 2, created_at: "2026-09-05T23:50:00Z" }),
  ];

  const deepMerge = (base: Record<string, unknown>, overlay: Record<string, unknown>): Record<string, unknown> => {
    const out: Record<string, unknown> = { ...base };
    for (const [key, value] of Object.entries(overlay)) {
      const existing = out[key];
      out[key] =
        value !== null && typeof value === "object" && !Array.isArray(value) && existing !== null && typeof existing === "object" && !Array.isArray(existing)
          ? deepMerge(existing as Record<string, unknown>, value as Record<string, unknown>)
          : value;
    }
    return out;
  };

  const policyGet = (): Envelope => {
    const effective = deepMerge(builtinPolicy, sqlitePolicy);
    const attribution: Record<string, string> = {
      ...Object.fromEntries(Object.keys(builtinPolicy).map((key) => [key, "builtin"])),
      ...Object.fromEntries(Object.keys(sqlitePolicy).map((key) => [key, "sqlite"])),
    };
    return makeEnvelope(
      { layers: { sqlite: sqlitePolicy }, effective, attribution, layer: null, key: null, value: null, path: null },
      { command: "policy.get" },
    );
  };

  const policySet = (args: string[]): Envelope => {
    const layer = args[args.indexOf("--layer") + 1];
    const key = args[args.indexOf("--key") + 1];
    const rawValue = args[args.indexOf("--value") + 1];
    if (layer !== "local") {
      return makeEnvelope({}, { command: "policy.set", ok: false, errors: [{ code: "USAGE_ERROR", element: null, message: "--layer 只能是 local" }] });
    }
    let value: unknown;
    try {
      value = JSON.parse(rawValue ?? "");
    } catch {
      return makeEnvelope({}, { command: "policy.set", ok: false, errors: [{ code: "USAGE_ERROR", element: null, message: "--value 不是合法 JSON" }] });
    }
    // 门禁演示：值含 demo-secret 字样即视为疑似明文
    if (typeof value === "string" && value.includes("demo-secret")) {
      return makeEnvelope({}, { command: "policy.set", ok: false, errors: [{ code: "POLICY_SECRET_GATE", element: null, message: "疑似明文敏感信息（demo_secret），拒绝写入本地数据库" }] });
    }
    const target = sqlitePolicy;
    const segments = key.split(".");
    let cursor = target;
    for (const segment of segments.slice(0, -1)) {
      if (typeof cursor[segment] !== "object" || cursor[segment] === null) cursor[segment] = {};
      cursor = cursor[segment] as Record<string, unknown>;
    }
    cursor[segments[segments.length - 1]] = value;
    return makeEnvelope(
      { layers: null, effective: null, attribution: null, layer: "sqlite", key, value, path: "/native-manager.db" },
      { command: "policy.set" },
    );
  };

  const policyDelete = (args: string[]): Envelope => {
    const layer = args[args.indexOf("--layer") + 1];
    const key = args[args.indexOf("--key") + 1];
    if (layer !== "local") {
      return makeEnvelope({}, { command: "policy.delete", ok: false, errors: [{ code: "USAGE_ERROR", element: null, message: "--layer 只能是 local" }] });
    }
    const target = sqlitePolicy;
    const segments = key.split(".");
    let cursor: Record<string, unknown> | undefined = target ?? undefined;
    for (const segment of segments.slice(0, -1)) {
      const next = cursor?.[segment];
      if (typeof next !== "object" || next === null) { cursor = undefined; break; }
      cursor = next as Record<string, unknown>;
    }
    const leaf = segments[segments.length - 1];
    if (!cursor || !(leaf in cursor)) {
      return makeEnvelope({}, { command: "policy.delete", ok: false, errors: [{ code: "POLICY_KEY_NOT_FOUND", element: null, message: `键不存在:${key}` }] });
    }
    delete cursor[leaf];
    return makeEnvelope(
      { layers: null, effective: null, attribution: null, layer: "sqlite", key, value: null, path: "/native-manager.db" },
      { command: "policy.delete" },
    );
  };

  const applyStepNames: Record<string, string> = {
    agents: "agent-lifecycle",
    mcp: "mcp-sync",
    skills: "skills-link",
    rules: "rules-sync",
    hooks: "hooks-sync",
    tools: "tools-sync",
    aliases: "shell-aliases",
  };

  const recordRun = (runId: string, _command: string, dryRun: boolean): void => {
    runs.unshift(makeRunEntry({ run_id: runId, last_at: new Date().toISOString(), operations: dryRun ? 1 : 2 }));
  };

  const applyEnvelope = (domain: string, args: string[], runId: string): Envelope => {
    const dryRun = args.includes("--dry-run");
    recordRun(runId, `${domain}.apply`, dryRun);
    const snapshot = dryRun
      ? null
      : { run_id: runId, path: `/tmp/mock-snapshots/${runId}`, file_count: 2, link_count: 1 };
    if (snapshot) {
      snapshots.unshift(makeSnapshotEntry({ run_id: runId, command: `${domain}.apply`, created_at: new Date().toISOString(), file_count: 2, link_count: 1 }));
    }
    return makeEnvelope(
      {
        domain,
        dry_run: dryRun,
        steps: [{ name: applyStepNames[domain] ?? `${domain}-sync`, ok: true, exit_code: 0 }],
        snapshot,
        skipped: [],
      },
      { command: `${domain}.apply`, run_id: runId, lock: { held: !dryRun, pid: dryRun ? null : 4242 } },
    );
  };

  const convergeEnvelope = (args: string[], runId: string): Envelope => {
    const dryRun = args.includes("--dry-run");
    recordRun(runId, "system.converge", dryRun);
    const snapshot = dryRun ? null : { run_id: runId, path: `/tmp/mock-snapshots/${runId}`, file_count: 9, link_count: 2 };
    if (snapshot) {
      snapshots.unshift(makeSnapshotEntry({ run_id: runId, command: "system.converge", created_at: new Date().toISOString(), file_count: 9, link_count: 2 }));
    }
    return makeEnvelope(
      {
        pipeline: "update",
        dry_run: dryRun,
        steps: ["shell-aliases", "tools-sync", "rules-sync", "skills-link", "mcp-sync", "hooks-sync"].map((name) => ({ name, ok: true, exit_code: 0 })),
        snapshot,
        skipped: [],
      },
      { command: "system.converge", run_id: runId, lock: { held: !dryRun, pid: dryRun ? null : 4242 } },
    );
  };

  return {
    async run(command, args = [], runOptions = {}) {
      // 模拟进程耗时，让加载/进行中的 UI 状态可见
      await new Promise((resolve) => setTimeout(resolve, 120));
      const runId = runOptions.runId ?? "20260906T020000Z-mockrun";

      // ---- 写路径与有状态命令 ----
      if (command === "sources:get") {
        const domain = args[args.indexOf("--key") + 1];
        const suffix = domain === "rules" ? "shared" : domain === "hooks" ? "claude" : "shell";
        return makeEnvelope({ sources: [{ key: `${domain}.${suffix}`, label: domain === "rules" ? "共享规则" : domain === "hooks" ? "Claude Hooks" : "Shell aliases", value: domain === "rules" ? "使用简体中文回答。\n" : {}, revision: "demo-revision", format: domain === "rules" ? "text" : "json" }] }, { command: "sources.get" });
      }
      if (command === "sources:set") return makeEnvelope({}, { command: "sources.set" });
      if (command === "policy:get") return policyGet();
      if (command === "policy:set") return policySet(args);
      if (command === "policy:delete") return policyDelete(args);
      if (command === "runs:list") {
        return makeEnvelope({ runs }, { command: "runs.list" });
      }
      if (command === "runs:get") {
        const id = args[args.indexOf("--run") + 1];
        return makeEnvelope(
          { run_id: id, operations: [{ id: "op-mock", operation_type: "mcp:apply", target: "mcp:grok-search", detail: { dry_run: false }, created_at: "2026-09-05T23:59:00Z" }] },
          { command: "runs.get" },
        );
      }
      if (command === "snapshots:list") {
        return makeEnvelope({ snapshots, restore: null }, { command: "snapshots.list" });
      }
      if (command === "snapshots:restore") {
        const id = args[args.indexOf("--run") + 1];
        const resnapshot = `resnap-${id}`;
        snapshots.unshift(makeSnapshotEntry({ run_id: resnapshot, command: "snapshots.restore", created_at: new Date().toISOString() }));
        return makeEnvelope(
          { snapshots: null, restore: { restored_run: id, files_restored: 3, files_deleted: 1, links_restored: 1, skipped: [], resnapshot_run_id: resnapshot } },
          { command: "snapshots.restore", run_id: runId },
        );
      }
      if (command === "system:converge") return convergeEnvelope(args, runId);
      if (command.endsWith(":apply")) {
        return applyEnvelope(command.split(":")[0], args, runId);
      }

      // ---- 只读命令 ----
      const envelope = routes[command];
      if (!envelope) {
        throw new EngineBridgeError({ kind: "unknown_command", message: `mock 未覆盖命令: ${command}` });
      }
      const cloned = structuredClone(envelope);
      if (options.offline) {
        cloned.mode = makeMode({ offline: true });
        if (command === "system:refresh-upstream") {
          cloned.ok = false;
          cloned.data = {};
          cloned.errors = [
            { code: "OFFLINE_NETWORK_DENIED", element: null, message: "system:refresh-upstream 需要网络探测，offline 模式下拒绝执行" },
          ];
        }
      }
      if (command.endsWith(":plan")) {
        const onlyList: string[] = [];
        for (let i = 0; i < args.length; i++) {
          if (args[i] === "--only" && i + 1 < args.length) {
            onlyList.push(args[i + 1]);
          }
        }
        const agentIdx = args.indexOf("--agent");
        const agent = agentIdx !== -1 ? args[agentIdx + 1] : null;

        if (onlyList.length > 0 || agent) {
          const data = cloned.data as { actions?: Array<Record<string, unknown>>; summary?: Record<string, number> };
          if (data && Array.isArray(data.actions)) {
            if (onlyList.length > 0) {
              data.actions = data.actions.filter((act) =>
                onlyList.some((o) => {
                  const el = act.element as string;
                  if (el === o) return true;
                  if (typeof el === "string") {
                    if (el.endsWith(`:${o}`) || el.startsWith(`${o}/`) || el.replace(/^[^:]+:/, "") === o) return true;
                  }
                  return false;
                })
              );
            }
            if (agent && command === "skills:plan") {
              data.actions = data.actions.filter((act) =>
                typeof act.target === "string" ? act.target.includes(agent) : (typeof act.note === "string" && act.note.includes(agent))
              );
            }
            if (data.summary) {
              const summary: Record<string, number> = { read: 0, "write-local": 0, "exec-local": 0, network: 0, "install-remove": 0 };
              for (const act of data.actions) {
                const risk = act.risk as string;
                if (risk in summary) summary[risk]++;
              }
              data.summary = summary;
            }
          }
        }
      }
      if (command === "system:refresh-upstream" && args[0] === "--only" && typeof args[1] === "string") {
        const only = args[1];
        const data = cloned.data as { elements?: Array<Record<string, unknown>> };
        data.elements = (data.elements ?? []).filter((entry) => entry["id"] === only);
        if (data.elements.length === 0) {
          data.elements = [{ id: only, probe: "npm-view", state: "current", current: "1.0.0", upstream: "1.0.0", error: null }];
        }
      }
      return cloned;
    },
  };
}
