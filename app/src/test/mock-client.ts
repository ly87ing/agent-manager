import type { Envelope, RunEntry } from "../lib/contract";
import { EngineBridgeError, type EngineClient } from "../lib/engine";
import {
  agentManagementFixture,
  makeAuditData,
  makeElement,
  makeEnvelope,
  makeFreshness,
  makeRunEntry,
} from "./fixtures";

/**
 * dev-only mock 引擎（?mock=1 / ?mock=fresh，仅在 import.meta.env.DEV 下经
 * initEngineClient 动态加载；生产构建 treeshake 掉本模块）。
 * fixture 集覆盖：freshness 五态、收敛七态。
 */

const CHECKED_AT = "2026-09-05T10:00:00Z";

function demoEnvelopes(): Record<string, Envelope> {
  const audit = makeEnvelope(
    makeAuditData({
      status: "warning",
      sections: {
        runtime: [{ name: "engine-rs", status: "ok", message: "Rust 原生引擎" }],
        agents: [{ name: "Old Agent", status: "notice", message: "未声明策略（已忽略）" }],
        mcp: [{ name: "Kimi Code", status: "warning", message: "托管产物尚未生成", category: "unmaterialized" }],
        skills: [{ name: "Codex", status: "error", message: "skill 目标漂移 (dead-one)" }],
      },
      summary: {
        errors: 1,
        warnings: 1,
        warning_categories: { optional_tools: 0, unmaterialized: 1, official_gaps: 0 },
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
            install_source: "native",
            installed: true,
            management: agentManagementFixture("claude"),
            auth: { state: "ready", authenticated: true, mode: "oauth", storage: "file", file: ".credentials.json" },
            current_config: {
              state: "ready",
              file: "settings.json",
              target: "/Users/demo/.claude/settings.json",
              config: {
                provider: "anthropic",
                base_url: "https://old-relay.example.com",
                model: "claude-old",
                small_fast_model: null,
                model_aliases: { opus: null, sonnet: null, haiku: null },
                timeout_ms: null,
                credential: { configured: true, kind: "token", variable: null },
                runtime: {},
              },
            },
          },
          state: "managed-ok",
          freshness: makeFreshness({ state: "behind", current: "2.0.0", upstream: "2.1.3", checked_at: CHECKED_AT, probe: "native", cache_age_sec: 300 }),
          actions: [{ id: "upgrade", risk: "network", needs_lock: true, offline_ok: false }],
        }),
        makeElement({
          id: "agents:codex",
          domain: "agents",
          spec: {
            display_name: "Codex",
            bin: "codex",
            preferred_provider: "native",
            providers: { native: { managed: true } },
            settings: {
              profiles: {
                kimi: {
                  label: "Kimi Code",
                  note: "工作账号 · 月底到期",
                  base_url: "https://api.kimi.com/coding/v1",
                  token_var: "KIMI_API_KEY",
                  model: "k3-256k",
                },
              },
            },
          },
          policy: { enabled: true, auto_install: true, auto_update: true, preferred_provider: "native" },
          observed: {
            install_source: "native",
            installed: true,
            management: agentManagementFixture("codex"),
            auth: { state: "ready", authenticated: true, mode: "chatgpt", storage: "file", file: "auth.json" },
          },
          state: "managed-ok",
          freshness: makeFreshness({ state: "ahead", current: "0.43.0-beta.1", upstream: "0.42.0", checked_at: CHECKED_AT, probe: "native", cache_age_sec: 300 }),
        }),
        makeElement({
          id: "agents:kimi-code",
          domain: "agents",
          spec: { display_name: "Kimi Code", bin: "kimi", preferred_provider: "npm", providers: { npm: { managed: true }, brew: { managed: false } } },
          policy: { enabled: true, auto_install: false, auto_update: false, preferred_provider: "npm" },
          observed: { install_source: "brew", installed: true },
          state: "unmanaged-detected",
          freshness: makeFreshness({ state: "unknown", current: null, upstream: null, checked_at: null, probe: null, cache_age_sec: null }),
        }),
        makeElement({
          id: "agents:old-agent",
          domain: "agents",
          spec: { display_name: "Old Agent", bin: "old", preferred_provider: "npm", providers: { npm: { managed: true } } },
          policy: { enabled: false, auto_install: false, auto_update: false },
          observed: { install_source: null, installed: false },
          state: "disabled",
        }),
        makeElement({
          id: "agents:qoder",
          domain: "agents",
          spec: { display_name: "Qoder", bin: "qoder", preferred_provider: "npm", providers: { npm: { managed: true } } },
          policy: { enabled: true, auto_install: false, auto_update: false },
          observed: { install_source: null, installed: false },
          state: "missing",
        }),
      ],
    },
    { command: "agents.status" },
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
              "claude-code": { state: "drifted", target: "/Users/demo/.claude.json", target_exists: true, assigned: true, enabled: true, actions: ["update"] },
              codex: { state: "managed-ok", target: "/Users/demo/.codex/config.toml", target_exists: true, assigned: true, enabled: true, actions: ["disable"] },
              "kimi-code": { state: "missing", target: "/Users/demo/.kimi-code/mcp.json", target_exists: true, assigned: true, enabled: true, actions: ["add"] },
            },
            env: {},
            actions: ["disable", "remove", "upgrade"],
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
              "claude-code": { state: "managed-ok", target: "/Users/demo/.claude.json", target_exists: true, assigned: true, enabled: true, actions: ["disable"] },
              codex: { state: "not-materialized", target: "/Users/demo/.codex/config.toml", target_exists: true, assigned: true, enabled: true, actions: [], blocked_reason: "凭证变量未配置: TAVILY_API_KEY" },
              "kimi-code": { state: "managed-ok", target: "/Users/demo/.kimi-code/mcp.json", target_exists: true, assigned: true, enabled: true, actions: ["disable"] },
            },
            env: {
              GROK_API_KEY: { placeholder: "GROK_API_KEY", resolved: true },
              TAVILY_API_KEY: { placeholder: "TAVILY_API_KEY", resolved: false },
            },
            actions: ["disable", "remove"],
          },
          state: "drifted",
          freshness: makeFreshness({ state: "unknown", current: null, upstream: "8f0ae3c7f25ec419631ade7e431fa2531503433e", checked_at: CHECKED_AT, probe: "git-ls-remote", cache_age_sec: 4000 }),
        }),
        makeElement({
          id: "mcp:my-proxy",
          domain: "mcp",
          spec: { command: "npx", args: ["-y", "my-proxy-mcp"], startup_timeout_sec: 60, agents: null, env: { MY_PROXY_TOKEN: "${MY_PROXY_TOKEN}" } },
          policy: { enabled: true, agents: null },
          observed: {
            agents: {
              "claude-code": { state: "managed-ok", target: "/Users/demo/.claude.json", target_exists: true, assigned: true, enabled: true, actions: ["disable"] },
              codex: { state: "missing", target: "/Users/demo/.codex/config.toml", target_exists: true, assigned: true, enabled: true, actions: ["add"] },
              "kimi-code": { state: "missing", target: "/Users/demo/.kimi-code/mcp.json", target_exists: true, assigned: true, enabled: true, actions: ["add"] },
            },
            env: { MY_PROXY_TOKEN: { placeholder: "MY_PROXY_TOKEN", resolved: true } },
            actions: ["disable", "remove", "upgrade"],
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
              "claude-code": { state: "managed-ok", target: "/Users/demo/.claude.json", target_exists: true, assigned: true, enabled: true, actions: ["disable"] },
              codex: { state: "leftover", target: "/Users/demo/.codex/config.toml", target_exists: true, assigned: false, enabled: false, actions: ["remove"] },
              "kimi-code": { state: "not-materialized", target: "/Users/demo/.kimi-code/mcp.json", target_exists: false, assigned: true, enabled: true, actions: ["add"] },
            },
            env: {},
            actions: ["disable", "remove"],
          },
          state: "drifted",
          freshness: makeFreshness({ state: "current", current: "a1b2c3d", upstream: "a1b2c3d", checked_at: CHECKED_AT, probe: "git-ls-remote", cache_age_sec: 200 }),
        }),
        makeElement({
          id: "mcp:pending-server",
          domain: "mcp",
          spec: { command: "npx", args: ["-y", "pending-mcp"], startup_timeout_sec: 60, agents: null, env: {} },
          policy: { enabled: true, agents: null },
          observed: { install_source: null, agents: {}, env: {} },
          state: "not-materialized",
          freshness: makeFreshness({ state: "not-applicable" }),
        }),
        makeElement({
          id: "mcp:remote-api",
          domain: "mcp",
          spec: { url: "https://mcp.demo.example.com/endpoint", headers: { Authorization: "${DEMO_REMOTE_TOKEN}" }, agents: null },
          policy: { enabled: true, agents: null },
          observed: {
            agents: {
              "claude-code": { state: "managed-ok", target: "/Users/demo/.claude.json", target_exists: true, assigned: true, enabled: true, actions: ["disable"] },
              codex: { state: "missing", target: "/Users/demo/.codex/config.toml", target_exists: true, assigned: true, enabled: true, actions: ["add"] },
            },
            env: {},
            headers: { Authorization: { placeholder: "${DEMO_REMOTE_TOKEN}", resolved: true } },
            actions: ["disable", "remove"],
          },
          state: "managed-ok",
          freshness: makeFreshness({ state: "not-applicable" }),
        }),
        makeElement({
          id: "mcp:legacy-server",
          domain: "mcp",
          spec: { command: "legacy-mcp", args: [], startup_timeout_sec: 60, agents: null, env: {} },
          policy: { enabled: true, agents: null },
          observed: { install_source: null, agents: {}, env: {} },
          state: "unsupported",
          freshness: makeFreshness({ state: "not-applicable" }),
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
          observed: { install_source: null, repo_cache: "/repo/agent-skill/repositories/agent-skills",
            repo_cached: true,
            owner_marked: true,
            discovered: ["safe-merge-review", "artifact-hygiene", "dead-one", "new-hotness", "experimental-lint"],
            discovery_inbox: ["new-hotness", "experimental-lint"],
            extra_links: [],
            discovery_error: null,
            blocked_reason: null,
            actions: ["update", "disable", "remove"] },
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
              "claude-code": { state: "drifted", link_path: "/Users/demo/.claude/skills/safe-merge-review", target_dir_exists: true, assigned: true, enabled: true, actions: ["disable"] },
              codex: { state: "linked", link_path: "/Users/demo/.codex/skills/safe-merge-review", target_dir_exists: true, assigned: true, enabled: true, actions: ["disable"] },
              "kimi-code": { state: "missing-link", link_path: "/Users/demo/.kimi-code/skills/safe-merge-review", target_dir_exists: true, assigned: true, enabled: true, actions: ["update"] },
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
              "claude-code": { state: "linked", link_path: "/Users/demo/.claude/skills/artifact-hygiene", target_dir_exists: true, assigned: true, enabled: true, actions: ["disable"] },
              codex: { state: "linked", link_path: "/Users/demo/.codex/skills/artifact-hygiene", target_dir_exists: true, assigned: true, enabled: true, actions: ["disable"] },
              "kimi-code": { state: "linked", link_path: "/Users/demo/.kimi-code/skills/artifact-hygiene", target_dir_exists: true, assigned: true, enabled: true, actions: ["disable"] },
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
              "claude-code": { state: "dead-link", link_path: "/Users/demo/.claude/skills/dead-one", target_dir_exists: true, assigned: true, enabled: true, actions: ["update"] },
              codex: { state: "disabled", link_path: "/Users/demo/.codex/skills/dead-one", target_dir_exists: true, assigned: true, enabled: false, actions: ["enable"] },
              "kimi-code": { state: "linked", link_path: "/Users/demo/.kimi-code/skills/dead-one", target_dir_exists: true, assigned: true, enabled: true, actions: ["disable"] },
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
              "claude-code": { state: "not-assigned", link_path: "/Users/demo/.claude/skills/new-hotness", target_dir_exists: true, assigned: false, enabled: false, actions: ["add"] },
              codex: { state: "not-assigned", link_path: "/Users/demo/.codex/skills/new-hotness", target_dir_exists: true, assigned: false, enabled: false, actions: ["add"] },
              "kimi-code": { state: "not-assigned", link_path: "/Users/demo/.kimi-code/skills/new-hotness", target_dir_exists: true, assigned: false, enabled: false, actions: ["add"] },
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
          observed: { install_source: null, agents: {}, discovery_inbox: true, blocked_reason: "请先在集合定义中选择要分配的助手" },
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
          observed: { install_source: null, target: "/Users/demo/.codex/AGENTS.md",
            target_exists: true,
            expected_digest: "aaaa3c8ae2687d47a8e7543d3c8c43ef8dd9f744e3a3e2ce9aa9ad7a90de12a2",
            applied_digest: "aaaa3c8ae2687d47a8e7543d3c8c43ef8dd9f744e3a3e2ce9aa9ad7a90de12a2",
            shared_revision: "1111",
            adapter_revision: "2222",
            blocked_reason: null,
            registration_needed: false },
          state: "managed-ok",
          actions: [],
        }),
        makeElement({
          id: "rules:claude_global",
          domain: "rules",
          spec: { name: "Claude Code 全局规则", group: "global-rules", source_keys: ["rules.shared", "rules.claude"], target: "~/.claude/CLAUDE.md" },
          policy: { enabled: true, agent_id: "claude-code" },
          observed: { install_source: null, target: "/Users/demo/.claude/CLAUDE.md",
            target_exists: true,
            expected_digest: "bbbb4b5c130887752148845080715867e6c3e6f40e69189067dd7d92db8b40c36",
            applied_digest: "aaaa3c8ae2687d47a8e7543d3c8c43ef8dd9f744e3a3e2ce9aa9ad7a90de12a2",
            shared_revision: "1111",
            adapter_revision: "3333",
            blocked_reason: null,
            registration_needed: false },
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
          id: "secrets:mcp-env",
          domain: "secrets",
          spec: { sources: ["external file", "process env"] },
          policy: { enabled: true },
          observed: { install_source: null, variables: [
              { server: "grok-search", env_key: "GROK_API_KEY", placeholder: "GROK_API_KEY", resolved: true, kind: "env" },
              { server: "grok-search", env_key: "TAVILY_API_KEY", placeholder: "TAVILY_API_KEY", resolved: false, kind: "env" },
              { server: "remote-api", env_key: "Authorization", placeholder: "DEMO_REMOTE_TOKEN", resolved: true, kind: "header" },
            ],
            unresolved: 1,
            source: { configured: true, path: "~/agent-manager/secrets/mcp.env", ready: true, error: null },
            legacy_database_values: 0 },
          state: "missing",
        }),
      ],
    },
    { command: "secrets.status" },
  );

  const profilesStatus = makeEnvelope(
    {
      domain: "profiles",
      elements: [
        makeElement({
          id: "profiles:claude-work",
          domain: "profiles",
          spec: {
            display_name: "Claude Code · 工作账号",
            base_agent: "claude-code",
            kind: "claude",
            config_dir: "~/.claude-work",
            base_url: "https://api.example.com",
            model: "claude-sonnet-demo",
          },
          policy: { enabled: true },
          observed: {
            base_agent: "claude-code",
            kind: "claude",
            config_env: "CLAUDE_CONFIG_DIR",
            config_dir: "~/.claude-work",
            config_dir_exists: true,
            target: "~/.claude-work/settings.json",
            target_exists: true,
            env_refs: [{ name: "ANTHROPIC_AUTH_TOKEN", kind: "reference", variable: "WORK_API_KEY", resolved: true }],
            launch_value: 'CLAUDE_CONFIG_DIR="$HOME/.claude-work" claude',
            blocked_reason: null,
            auth: { state: "missing", authenticated: false, mode: null, storage: "file", file: ".credentials.json" },
          },
          state: "managed-ok",
          freshness: makeFreshness({ state: "not-applicable" }),
          actions: [{ id: "apply", risk: "write-local", needs_lock: true, offline_ok: true }],
        }),
      ],
    },
    { command: "profiles.status" },
  );

  const usageStatus = makeEnvelope(
    {
      domain: "usage",
      rows: [
        {
          hour: `${new Date().toISOString().slice(0, 10)}T02:00:00Z`,
          agent: "claude-code",
          model: "claude-sonnet-demo",
          input: 820_000,
          output: 95_000,
          cache_write: 120_000,
          cache_read: 460_000,
          requests: 14,
        },
        {
          hour: `${new Date().toISOString().slice(0, 10)}T03:00:00Z`,
          agent: "codex",
          model: "gpt-demo",
          input: 510_000,
          output: 62_000,
          cache_write: 0,
          cache_read: 180_000,
          requests: 9,
        },
      ],
      pricing: {
        "claude-sonnet-demo": { input: 3, output: 15, cache_write: 3.75, cache_read: 0.3 },
      },
      last_scan_at: new Date().toISOString(),
    },
    { command: "usage.status" },
  );

  return {
    "system:audit": audit,
    "agents:status": agentsStatus,
    "mcp:status": mcpStatus,
    "skills:status": skillsStatus,
    "rules:status": rulesStatus,
    "secrets:status": secretsStatus,
    "profiles:status": profilesStatus,
    "usage:status": usageStatus,
    "usage:scan": makeEnvelope({ domain: "usage", action: "scan", files: 3, bytes: 24_576 }, { command: "usage.scan" }),
    "hooks:status": makeEnvelope(
      {
        domain: "hooks",
        elements: [
          makeElement({
            id: "hooks:claude",
            domain: "hooks",
            spec: { name: "Claude Hooks", merge: "signature-set", source_key: "hooks.claude", target: "~/.claude/settings.json" },
            policy: { enabled: true, agent_id: "claude-code" },
            observed: { install_source: null, target: "/Users/demo/.claude/settings.json",
              target_exists: true,
              expected_digest: "cccc5d5e240998863259085b2b52df0f1e0a855f4b4b4d3d0bb0be8ba91ef23b3",
              applied_digest: "aaaa3c8ae2687d47a8e7543d3c8c43ef8dd9f744e3a3e2ce9aa9ad7a90de12a2",
              actual_digest: "aaaa3c8ae2687d47a8e7543d3c8c43ef8dd9f744e3a3e2ce9aa9ad7a90de12a2",
              source_revision: "1111",
              blocked_reason: null,
              registration_needed: false,
              managed_hook_count: 1,
              missing_signatures: 1,
              removed_signatures_present: 0 },
            state: "drifted",
            actions: [{ id: "sync", risk: "write-local", needs_lock: true, offline_ok: true }],
          }),
          makeElement({
            id: "hooks:codex",
            domain: "hooks",
            spec: { name: "Codex Hooks", merge: "whole-file", source_key: "hooks.codex", target: "~/.codex/hooks.json" },
            policy: { enabled: true, agent_id: "codex" },
            observed: { install_source: null, target: "/Users/demo/.codex/hooks.json",
              target_exists: true,
              expected_digest: "dddd6e6f351aa997436a196c3c63e01f2f1b96605c5c5e4e1cc1cf9cb02f034c4",
              applied_digest: "dddd6e6f351aa997436a196c3c63e01f2f1b96605c5c5e4e1cc1cf9cb02f034c4",
              actual_digest: "dddd6e6f351aa997436a196c3c63e01f2f1b96605c5c5e4e1cc1cf9cb02f034c4",
              source_revision: "2222",
              blocked_reason: null,
              registration_needed: false,
              managed_hook_count: 0,
              missing_signatures: 0,
              removed_signatures_present: 0 },
            state: "managed-ok",
            actions: [],
          }),
        ],
      },
      { command: "hooks.status" },
    ),
    "aliases:status": makeEnvelope({ domain: "aliases", elements: [] }, { command: "aliases.status" }),
    "system:refresh-upstream": makeEnvelope(
      {
        domain: "system",
        verb: "refresh-upstream",
        // 真实引擎：缓存落 SQLite upstream_probes 表，无缓存文件路径
        cache_file: null,
        elements: [
          { id: "agents:claude-code", probe: "npm-view", state: "behind", current: "2.0.0", upstream: "2.1.3", error: null },
          { id: "agents:codex", probe: "npm-view", state: "ahead", current: "0.43.0-beta.1", upstream: "0.42.0", error: null },
          { id: "mcp:chrome-devtools", probe: "npm-view", state: "behind", current: "1.2.0", upstream: "1.8.0", error: null },
          { id: "mcp:grok-search", probe: "git-ls-remote", state: "unknown", current: null, upstream: "8f0ae3c7f25ec419631ade7e431fa2531503433e", error: "uvx-local-unavailable" },
          { id: "mcp:my-proxy", probe: "npm-view", state: "current", current: "0.3.1", upstream: "0.3.1", error: null },
          { id: "skill:agent-skills", probe: "git-ls-remote", state: "behind", current: "a1b2c3d4e5f60718293a4b5c6d7e8f9012a3b4c5", upstream: "f0e1d2c3b4a5968778695a4b3c2d1e0f9a8b7c6d", error: null },
        ],
        summary: { probed: 6, current: 1, behind: 3, ahead: 1, unknown: 1 },
      },
      { command: "system.refresh-upstream" },
    ),
    // 接管发现：覆盖五类来源（外部目录、已定义集合的计划外链接、原生真实目录、断链、未登记助手、未纳管 MCP）
    "system:discover": makeEnvelope(
      {
        domain: "discover",
        agents: [
          {
            id: "opencode",
            display_name: "OpenCode",
            bin: "opencode",
            bin_path: "~/.local/bin/opencode",
            version: "0.6.2",
            origin: "native",
            npm_package: null,
            manage: "register",
            managed_after_takeover: false,
            suggested_definition: { display_name: "OpenCode", bin: "opencode", preferred_provider: "native", providers: { native: { managed: false, detect_paths: ["~/.local/bin/opencode"] } } },
          },
          {
            id: "gemini-cli",
            display_name: "Gemini CLI",
            bin: "gemini",
            bin_path: "~/.npm-global/bin/gemini",
            version: "0.9.0",
            origin: "npm",
            npm_package: "@google/gemini-cli",
            manage: "register",
            managed_after_takeover: true,
            suggested_definition: { display_name: "Gemini CLI", bin: "gemini", preferred_provider: "npm", providers: { npm: { managed: true, install: "npm install -g @google/gemini-cli", upgrade: "npm install -g @google/gemini-cli", uninstall: "npm uninstall -g @google/gemini-cli" } } },
          },
        ],
        skills: {
          items: [
            { agent: "claude-code", name: "ones", link_path: "~/.claude/skills/ones", target: "~/.vendor-tool/skills/ones", origin: "external-link", manage: "symlink", collection: null, group: "~/.vendor-tool/skills", suggested_collection: "vendor-tool", has_skill_md: true },
            { agent: "codex", name: "ones", link_path: "~/.codex/skills/ones", target: "~/.vendor-tool/skills/ones", origin: "external-link", manage: "symlink", collection: null, group: "~/.vendor-tool/skills", suggested_collection: "vendor-tool", has_skill_md: true },
            { agent: "claude-code", name: "gitlab", link_path: "~/.claude/skills/gitlab", target: "~/.vendor-tool/skills/gitlab", origin: "external-link", manage: "symlink", collection: null, group: "~/.vendor-tool/skills", suggested_collection: "vendor-tool", has_skill_md: true },
            { agent: "claude-code", name: "safe-merge-review", link_path: "~/.claude/skills/safe-merge-review", target: "~/legacy/agent-skills/safe-merge-review", origin: "collection-extra-link", manage: "collection", collection: "agent-skills", group: null, has_skill_md: true },
            { agent: "claude-code", name: "synced", link_path: "~/.claude/skills/synced", target: null, origin: "native-dir", manage: "native", collection: null, group: null, has_skill_md: false },
            { agent: "codex", name: "stale-one", link_path: "~/.codex/skills/stale-one", target: "~/gone/stale-one", origin: "dead-link", manage: "cleanup", collection: null, group: null, has_skill_md: false },
          ],
          groups: [
            {
              name: "vendor-tool",
              source_path: "~/.vendor-tool/skills",
              manage: "symlink",
              existing_collection: false,
              agents: ["claude-code", "codex"],
              skills: [
                { name: "gitlab", agents: ["claude-code"], has_skill_md: true },
                { name: "ones", agents: ["claude-code", "codex"], has_skill_md: true },
              ],
              suggested_definition: { source_path: "~/.vendor-tool/skills", skill_paths: ["gitlab", "ones"], agents: ["claude-code", "codex"] },
            },
          ],
        },
        mcp: [
          { name: "my-proxy", agent: "claude-code", transport: "stdio", spec: { command: "npx", args: ["-y", "my-proxy"], env: { PROXY_TOKEN: "<literal>" } }, previously_managed: false, origin: "unmanaged", manage: "register" },
          { name: "vendor-remote", agent: "codex", transport: "remote", spec: { url: "https://mcp.example.com/sse", headers: { Authorization: "<literal>" } }, previously_managed: true, origin: "previously-managed", manage: "register" },
        ],
        summary: { agents: 2, skills_symlink: 3, skills_collection: 1, skills_native: 1, skills_cleanup: 1, mcp: 2 },
      },
      { command: "system.discover" },
    ),
  };
}

export function createMockEngineClient(options: { fresh?: boolean } = {}): EngineClient {
  const routes = demoEnvelopes();
  const initialSecretVariables = ((routes["secrets:status"].data as { elements: Array<{ observed: { variables?: Array<{ placeholder?: string; env_key?: string; resolved?: boolean; stored?: boolean }> } }> })
    .elements[0]?.observed.variables ?? []);
  const externallyResolvedSecrets = new Set(
    initialSecretVariables
      .filter((item) => item.resolved === true && item.stored !== true)
      .map((item) => {
        const placeholder = item.placeholder ?? item.env_key ?? "";
        return /^\$\{([^}]+)\}$/.exec(placeholder)?.[1] ?? placeholder;
      }),
  );

  // ---- 有状态的写路径演示数据（内存态，仅 DEV mock） ----
  const builtinPolicy: Record<string, unknown> = {
      agents: { "claude-code": { enabled: true, auto_update: true } },
      mcp: { "grok-search": { enabled: true } },
      skills: { "agent-skills": { enabled: true, new_skill_default: "disabled" } },
  };
  // fresh：全新机器，本地层空着——首次使用引导据此出现
  const sqlitePolicy: Record<string, unknown> = options.fresh
    ? {}
    : {
      browser_us_sync: { enabled: false },
      mcp: {
        definitions: { "my-proxy": { command: "npx", args: ["-y", "my-proxy-mcp"], startup_timeout_sec: 60, agents: null, env: { MY_PROXY_TOKEN: "${MY_PROXY_TOKEN}" } } },
        "my-proxy": { enabled: true },
      },
  };
  const runs: RunEntry[] = [
    makeRunEntry({ run_id: "20260905T235900Z-legacy1", last_at: "2026-09-05T23:59:00Z", operations: 2 }),
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
    aliases: "shell-aliases",
  };

  const recordRun = (runId: string, _command: string): void => {
    runs.unshift(makeRunEntry({ run_id: runId, last_at: new Date().toISOString(), operations: 2 }));
  };

  const applyEnvelope = (domain: string, _args: string[], runId: string): Envelope => {
    recordRun(runId, `${domain}.apply`);
    return makeEnvelope(
      {
        domain,
        steps: [{ name: applyStepNames[domain] ?? `${domain}-sync`, ok: true, exit_code: 0 }],
        skipped: [],
      },
      { command: `${domain}.apply`, run_id: runId, lock: { held: true, pid: 4242 } },
    );
  };

  const setSecret = (args: string[], runId: string): Envelope => {
    const variable = args[args.indexOf("--var") + 1] ?? "";
    const value = args[args.indexOf("--value") + 1] ?? "";
    const normalize = (placeholder: string): string => /^\$\{([^}]+)\}$/.exec(placeholder)?.[1] ?? placeholder;
    const secretsData = routes["secrets:status"].data as {
      elements: Array<{ observed: { variables?: Array<{ placeholder?: string; env_key?: string; resolved?: boolean; stored?: boolean }>; unresolved?: number } }>;
    };
    const variables = secretsData.elements[0]?.observed.variables ?? [];
    for (const item of variables) {
      if (normalize(item.placeholder ?? item.env_key ?? "") !== variable) continue;
      item.stored = value.length > 0;
      item.resolved = value.length > 0 || externallyResolvedSecrets.has(variable);
    }
    if (secretsData.elements[0]) {
      secretsData.elements[0].observed.unresolved = variables.filter((item) => item.resolved === false).length;
    }

    const mcpData = routes["mcp:status"].data as {
      elements: Array<{
        observed: {
          env?: Record<string, { placeholder?: string; resolved?: boolean }>;
          headers?: Record<string, { placeholder?: string; resolved?: boolean }>;
          agents?: Record<string, { state: string; actions?: string[]; blocked_reason?: string | null }>;
        };
      }>;
    };
    for (const element of mcpData.elements) {
      for (const resolution of [...Object.values(element.observed.env ?? {}), ...Object.values(element.observed.headers ?? {})]) {
        if (normalize(resolution.placeholder ?? "") === variable) {
          resolution.resolved = value.length > 0 || externallyResolvedSecrets.has(variable);
        }
      }
      if (variable === "TAVILY_API_KEY" && element.observed.agents?.codex) {
        const cell = element.observed.agents.codex;
        cell.state = value.length > 0 ? "missing" : "not-materialized";
        cell.actions = value.length > 0 ? ["add"] : [];
        cell.blocked_reason = value.length > 0 ? null : "凭证变量未配置: TAVILY_API_KEY";
      }
    }
    recordRun(runId, "secrets.set");
    return makeEnvelope({ variable, stored: value.length > 0 }, { command: "secrets.set", run_id: runId });
  };

  const configureAgent = (args: string[], runId: string): Envelope => {
    const agentId = args[args.indexOf("--only") + 1];
    const data = routes["agents:status"].data as {
      elements: Array<{ id: string; spec: Record<string, unknown>; observed: Record<string, unknown> }>;
    };
    const element = data.elements.find((item) => item.id === agentId);
    if (!element) {
      return makeEnvelope({}, {
        command: "agents.configure",
        ok: false,
        errors: [{ code: "UNKNOWN_ELEMENT", element: agentId ?? null, message: `找不到助手:${agentId ?? ""}` }],
      });
    }

    const current =
      element.spec.settings && typeof element.spec.settings === "object" && !Array.isArray(element.spec.settings)
        ? (element.spec.settings as Record<string, unknown>)
        : {};
    const next = structuredClone(current);
    const runtime = element.spec.bin === "codex"
      ? {
          automation_mode: next.automation_mode ?? "manual",
          codex_reasoning_effort: next.codex_reasoning_effort ?? null,
          codex_web_search: next.codex_web_search ?? null,
          codex_context_window: next.codex_context_window ?? null,
          codex_auto_compact_token_limit: next.codex_auto_compact_token_limit ?? null,
        }
      : {
          automation_mode: next.automation_mode ?? "manual",
          statusline: next.statusline === "builtin" ? "builtin" : null,
          claude_effort_level: next.claude_effort_level ?? null,
          claude_tool_search: next.claude_tool_search === true,
          claude_agent_teams: next.claude_agent_teams === true,
          claude_disable_artifact: next.claude_disable_artifact === true,
          claude_hide_attribution: next.claude_hide_attribution === true,
        };
    const useIndex = args.indexOf("--use");
    if (useIndex >= 0) {
      const profile = args[useIndex + 1];
      const profiles = next.profiles;
      if (!profile || !profiles || typeof profiles !== "object" || Array.isArray(profiles) || !(profile in profiles)) {
        return makeEnvelope({}, {
          command: "agents.configure",
          ok: false,
          errors: [{ code: "UNKNOWN_PROFILE", element: agentId, message: `找不到接入:${profile ?? ""}` }],
        });
      }
      next.active = profile;
      const selected = (profiles as Record<string, Record<string, unknown>>)[profile];
      const codex = element.spec.bin === "codex";
      element.observed.current_config = {
        state: "ready",
        file: codex ? "config.toml" : "settings.json",
        target: codex ? "/Users/demo/.codex/config.toml" : "/Users/demo/.claude/settings.json",
        config: {
          provider: codex ? profile : "anthropic",
          base_url: selected.base_url ?? null,
          model: selected.model ?? null,
          small_fast_model: selected.small_fast_model ?? null,
          model_aliases: selected.model_aliases ?? { opus: null, sonnet: null, haiku: null },
          timeout_ms: selected.timeout_ms ?? null,
          credential: {
            configured: true,
            kind: codex ? "env" : selected.auth ?? "token",
            variable: codex ? selected.token_var ?? null : null,
          },
          runtime,
        },
      };
    } else if (args.includes("--use-default")) {
      delete next.active;
      const codex = element.spec.bin === "codex";
      element.observed.current_config = {
        state: "ready",
        file: codex ? "config.toml" : "settings.json",
        target: codex ? "/Users/demo/.codex/config.toml" : "/Users/demo/.claude/settings.json",
        config: {
          provider: codex ? "openai" : "anthropic",
          base_url: null,
          model: null,
          small_fast_model: null,
          model_aliases: { opus: null, sonnet: null, haiku: null },
          timeout_ms: null,
          credential: { configured: false, kind: null, variable: null },
          runtime,
        },
      };
    }
    const observed = element.observed.current_config as {
      state?: string;
      config?: { runtime?: Record<string, unknown> } | null;
    } | undefined;
    if (observed?.state === "ready" && observed.config) observed.config.runtime = runtime;
    element.spec.settings = next;
    recordRun(runId, "agents.configure");
    return makeEnvelope(
      { domain: "agents", steps: [{ name: "agent-settings", ok: true, exit_code: 0 }], skipped: [] },
      { command: "agents.configure", run_id: runId, lock: { held: true, pid: 4242 } },
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
        if (domain === "hooks") {
          return makeEnvelope({ sources: [
            { key: "hooks.claude", label: "Claude Hooks", value: { hooks: { SessionStart: [{ hooks: [{ type: "command", command: "echo demo-hook" }] }] }, removed_signatures: [] }, revision: "demo-revision-claude", format: "json" },
            { key: "hooks.codex", label: "Codex Hooks", value: { hooks: {} }, revision: "demo-revision-codex", format: "json" },
          ] }, { command: "sources.get" });
        }
        if (domain === "aliases") {
          return makeEnvelope(
            {
              sources: [
                {
                  key: "aliases.shell",
                  label: "Shell aliases",
                  format: "json",
                  revision: "demo-revision-aliases",
                  value: {
                    timezone: "America/Los_Angeles",
                    lang: "en_US.UTF-8",
                    aliases: {
                      claude: 'TZ="{timezone}" LANG="{lang}" LC_ALL="{lang}" claude --dangerously-skip-permissions',
                      codex: 'TZ="{timezone}" LANG="{lang}" LC_ALL="{lang}" codex',
                      chrome: 'open --env TZ="{timezone}" -n -a "Google Chrome" --args --lang=en-US',
                    },
                  },
                },
              ],
            },
            { command: "sources.get" },
          );
        }
        const suffix = domain === "rules" ? "shared" : "shell";
        return makeEnvelope({ sources: [{ key: `${domain}.${suffix}`, label: domain === "rules" ? "共享规则" : "Shell aliases", value: domain === "rules" ? "使用简体中文回答。\n" : {}, revision: "demo-revision", format: domain === "rules" ? "text" : "json" }] }, { command: "sources.get" });
      }
      if (command === "sources:set") return makeEnvelope({}, { command: "sources.set" });
      if (command === "policy:get") return policyGet();
      if (command === "policy:set") return policySet(args);
      if (command === "policy:delete") return policyDelete(args);
      if (command === "secrets:set") return setSecret(args, runId);
      if (command === "secrets:source") {
        const path = args[args.indexOf("--path") + 1] ?? "";
        const data = routes["secrets:status"].data as {
          elements: Array<{ observed: { source?: { configured: boolean; path: string | null; ready: boolean; error: string | null } } }>;
        };
        if (data.elements[0]) {
          data.elements[0].observed.source = {
            configured: path.length > 0,
            path: path || null,
            ready: path.length > 0,
            error: null,
          };
        }
        recordRun(runId, "secrets.source");
        return makeEnvelope({ domain: "secrets", configured: path.length > 0 }, { command: "secrets.source", run_id: runId });
      }
      if (command === "secrets:migrate-legacy") {
        recordRun(runId, "secrets.migrate-legacy");
        return makeEnvelope({ domain: "secrets", migrated: [] }, { command: "secrets.migrate-legacy", run_id: runId });
      }
      if (command === "secrets:adopt") {
        recordRun(runId, "secrets.adopt");
        return makeEnvelope({ domain: "secrets", adopted: [] }, { command: "secrets.adopt", run_id: runId });
      }
      if (command === "config:export") {
        return makeEnvelope(
          {
            format: "agent-manager/config-backup/v1",
            policy: structuredClone(sqlitePolicy),
            sources: {},
          },
          { command: "config.export" },
        );
      }
      if (command === "config:import") {
        try {
          JSON.parse(args[args.indexOf("--value") + 1] ?? "");
        } catch {
          return makeEnvelope({}, { command: "config.import", ok: false, errors: [{ code: "USAGE_ERROR", element: null, message: "备份内容不是合法 JSON" }] });
        }
        recordRun(runId, "config.import");
        return makeEnvelope({ imported: true }, { command: "config.import", run_id: runId });
      }
      if (command === "runs:list") {
        return makeEnvelope({ runs }, { command: "runs.list" });
      }
      if (command === "runs:get") {
        const id = args[args.indexOf("--run") + 1];
        return makeEnvelope(
          { run_id: id, operations: [{ id: "op-mock", operation_type: "mcp:apply", target: "mcp:grok-search", detail: { result: "ok" }, created_at: "2026-09-05T23:59:00Z" }] },
          { command: "runs.get" },
        );
      }
      if (command === "runs:clear") {
        const deleted = runs.length;
        runs.splice(0, runs.length);
        return makeEnvelope({ deleted }, { command: "runs.clear" });
      }
      if (command === "agents:configure") return configureAgent(args, runId);
      if (command === "agents:check") {
        return makeEnvelope(
          { ok: true, detail: "模拟连接正常", latency_ms: 128 },
          { command: "agents.check" },
        );
      }
      if (command.endsWith(":apply")) {
        return applyEnvelope(command.split(":")[0], args, runId);
      }

      // ---- 只读命令 ----
      const envelope = routes[command];
      if (!envelope) {
        throw new EngineBridgeError({ kind: "unknown_command", message: `mock 未覆盖命令: ${command}` });
      }
      const cloned = structuredClone(envelope);
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
