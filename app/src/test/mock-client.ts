import type { Envelope } from "../lib/contract";
import { EngineBridgeError, type EngineClient } from "../lib/engine";
import { makeAuditData, makeElement, makeEnvelope, makeFreshness, makeMode } from "./fixtures";

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
        runtime: [{ name: "python3", status: "ok", message: "Python 3.14.7" }],
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
            preferred_provider: "npm",
            providers: { npm: { managed: true }, native: { managed: true }, brew: { managed: false } },
          },
          policy: { enabled: true, auto_install: true, auto_update: true, preferred_provider: "npm" },
          observed: {
            providers: ["npm"],
            classified: { managed: ["npm"], deprecated: [], system_only: [], unsupported: [] },
            installed: true,
            preferred_active: true,
          },
          state: "managed-ok",
          freshness: makeFreshness({ state: "behind", current: "2.0.0", upstream: "2.1.3", checked_at: CHECKED_AT, probe: "npm-view", cache_age_sec: 300 }),
          actions: [{ id: "upgrade", risk: "network", needs_lock: true, offline_ok: false }],
        }),
        makeElement({
          id: "agents:codex",
          domain: "agents",
          spec: { display_name: "Codex", bin: "codex", preferred_provider: "npm", providers: { npm: { managed: true } } },
          policy: { enabled: true, auto_install: true, auto_update: true, preferred_provider: "npm" },
          observed: { providers: ["npm"], classified: { managed: ["npm"], deprecated: [], system_only: [], unsupported: [] }, installed: true, preferred_active: true },
          state: "managed-ok",
          freshness: makeFreshness({ state: "current", current: "0.42.0", upstream: "0.42.0", checked_at: CHECKED_AT, probe: "npm-view", cache_age_sec: 300 }),
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
          spec: { name: "codex_global", group: "global", source: "rules/dist/codex/AGENTS.md", target: "~/.codex/AGENTS.md" },
          policy: { enabled: true },
          observed: { source: "/repo/agent-rule/rules/dist/codex/AGENTS.md", target: "/Users/demo/.codex/AGENTS.md", source_exists: true, target_exists: true, build_stale: false },
          state: "managed-ok",
        }),
        makeElement({
          id: "rules:claude_global",
          domain: "rules",
          spec: { name: "claude_global", group: "global", source: "rules/dist/claude/CLAUDE.md", target: "~/.claude/CLAUDE.md" },
          policy: { enabled: true },
          observed: { source: "/repo/agent-rule/rules/dist/claude/CLAUDE.md", target: "/Users/demo/.claude/CLAUDE.md", source_exists: true, target_exists: true, build_stale: false },
          state: "drifted",
        }),
        makeElement({
          id: "rules:kimi_global",
          domain: "rules",
          spec: { name: "kimi_global", group: "global", source: "rules/dist/kimi/AGENTS.md", target: "~/.kimi-code/AGENTS.md" },
          policy: { enabled: true },
          observed: { source: "/repo/agent-rule/rules/dist/kimi/AGENTS.md", target: "/Users/demo/.kimi-code/AGENTS.md", source_exists: false, target_exists: false, build_stale: true },
          state: "not-materialized",
        }),
        makeElement({
          id: "rules:windows_only",
          domain: "rules",
          spec: { name: "windows_only", group: "platform", source: "rules/dist/win/AGENTS.md", target: "%USERPROFILE%\\AGENTS.md" },
          policy: { enabled: true },
          observed: { source_exists: false, target_exists: false },
          state: "unsupported",
        }),
        makeElement({
          id: "rules:dist",
          domain: "rules",
          spec: { source_dir: "agent-rule/rules/src", dist_dir: "agent-rule/rules/dist" },
          policy: { enabled: true },
          observed: {
            src_latest_mtime: 1757000000,
            products: ["rules/dist/codex/AGENTS.md", "rules/dist/claude/CLAUDE.md"],
            missing_products: ["rules/dist/kimi/AGENTS.md"],
            stale_products: ["rules/dist/claude/CLAUDE.md"],
          },
          state: "drifted",
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
          before_digest: "sha256:aaaa3c8ae2687d47a8e7543d3c8c43ef8dd9f744e3a3e2ce9aa9ad7a90de12a2",
          after_digest: "sha256:bbbb4b5c130887752148845080715867e6c3e6f40e69189067dd7d92db8b40c36",
          backup_path: "/repo/snapshots/demo/CLAUDE.md.bak",
          cmd: null,
          note: "分发 rules/dist/claude/CLAUDE.md 到目标",
        },
        {
          op: "render",
          element: "rules:dist",
          target: "rules/dist/kimi/AGENTS.md",
          risk: "write-local",
          before_digest: null,
          after_digest: null,
          backup_path: null,
          cmd: null,
          note: "重建缺失产物",
        },
      ],
      summary: { read: 0, "write-local": 2, "exec-local": 0, network: 0, "install-remove": 0 },
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
          cmd: "npm install -g @anthropic-ai/claude-code",
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
          { id: "agents:claude-code", probe: "npm-view", state: "behind", current: "2.0.0", upstream: "2.1.3", error: null },
        ],
        summary: { probed: 6, current: 2, behind: 2, ahead: 1, unknown: 1 },
      },
      { command: "system.refresh-upstream" },
    ),
  };
}

export function createMockEngineClient(options: { offline?: boolean } = {}): EngineClient {
  const routes = demoEnvelopes();
  return {
    async run(command, args = []) {
      // 模拟进程耗时，让加载/进行中的 UI 状态可见
      await new Promise((resolve) => setTimeout(resolve, 120));
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
