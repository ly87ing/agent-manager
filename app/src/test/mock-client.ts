import type { Envelope, RunEntry } from "../lib/contract";
import { EngineBridgeError, type EngineClient } from "../lib/engine";
import {
  agentManagementFixture,
  makeAliasStatus,
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

/** mock 的写入基准：每次写入换一个，模拟 status 给出的摘要。 */
function mockBase(seed: number): string {
  return seed.toString(16).padStart(64, "0");
}

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
                // 演示：本工具不提供的档位（表单显示「保持当前」）、已有取值的预填、旧格式状态栏的修复入口
                runtime: {
                  automation_mode: "custom",
                  automation_raw: { "permissions.defaultMode": "acceptEdits" },
                  claude_effort_level: "high",
                  statusline: "builtin-legacy",
                },
              },
              // 实际配置对不上任何已保存接入：演示「当前实际配置（未保存）」
              base: mockBase(1),
              current: { default: false, official: false },
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
            // 官方账号，只在 config.toml 里指定了模型：仍是官方账号，不是一套未保存的接入
            current_config: {
              state: "ready",
              file: "config.toml",
              target: "/Users/demo/.codex/config.toml",
              config: {
                provider: "openai",
                base_url: null,
                model: "gpt-6-luna",
                credential: { configured: false, kind: null, variable: null },
                runtime: { automation_mode: "full", automation_raw: null, codex_reasoning_effort: "high" },
                fatal_issues: [],
                default_permissions: null,
              },
              base: mockBase(2),
              current: { default: true, official: true },
            },
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

  const mcpCellBase = { target_exists: true, extra: {}, vars: {}, legacy_marker: false, has_saved: false, notice: null, blocked_reason: null };
  const mcpStatus = makeEnvelope(
    {
      domain: "mcp",
      targets: [
        { agent: "claude-code", path: "/Users/demo/.claude.json", format: "claude-json", exists: true, agent_enabled: true, disable_mode: "snapshot", error: null },
        { agent: "codex", path: "/Users/demo/.codex/config.toml", format: "codex-toml", exists: true, agent_enabled: true, disable_mode: "field", error: null },
        { agent: "kimi-code", path: "/Users/demo/.kimi-code/mcp.json", format: "unsupported", exists: true, agent_enabled: true, disable_mode: "field", error: "不支持的 MCP 配置格式: kimi" },
      ],
      elements: [
        makeElement({
          id: "mcp:chrome-devtools",
          domain: "mcp",
          spec: { name: "chrome-devtools", saved: null },
          observed: {
            agents: {
              "claude-code": { ...mcpCellBase, state: "enabled", source: "file", target: "/Users/demo/.claude.json", transport: "stdio", entry: { command: "npx", args: ["-y", "chrome-devtools-mcp", "--isolated"], env: {} }, base: "1".repeat(64), command_available: true, actions: ["edit", "disable", "copy", "remove"] },
              codex: { ...mcpCellBase, state: "enabled", source: "file", target: "/Users/demo/.codex/config.toml", transport: "stdio", entry: { command: "npx", args: ["-y", "chrome-devtools-mcp@1.2.0"], env: {}, startup_timeout_sec: 120 }, extra: { cwd: "/Users/demo/work", enabled_tools: ["take_screenshot"] }, base: "2".repeat(64), legacy_marker: true, notice: "含旧版 Agent Manager 标记，可能触发助手配置警告；可预览后单独清理", command_available: true, actions: ["edit", "disable", "copy", "remove", "clean-markers"] },
            },
          },
          state: "managed-ok",
          freshness: makeFreshness({ state: "behind", current: "1.2.0", upstream: "1.8.0", checked_at: CHECKED_AT, probe: "npm-view", cache_age_sec: 1820 }),
        }),
        makeElement({
          id: "mcp:example-search",
          domain: "mcp",
          spec: { name: "example-search", saved: null },
          observed: {
            agents: {
              "claude-code": {
                ...mcpCellBase,
                state: "enabled",
                source: "file",
                target: "/Users/demo/.claude.json",
                transport: "stdio",
                entry: { command: "uvx", args: ["--from", "git+https://example.com/mcp/search@main", "example-search"], env: { SEARCH_API_KEY: "<literal>", CRAWL_API_KEY: "${CRAWL_API_KEY}" } },
                vars: { CRAWL_API_KEY: { placeholder: "${CRAWL_API_KEY}", resolved: false, stored: false, kind: "env" } },
                base: "3".repeat(64),
                command_available: true,
                actions: ["edit", "disable", "copy", "remove"],
              },
              codex: { ...mcpCellBase, state: "disabled", source: "file", target: "/Users/demo/.codex/config.toml", transport: "stdio", entry: { command: "uvx", args: ["example-search"], env: { SEARCH_API_KEY: "<literal>" } }, base: "4".repeat(64), command_available: true, actions: ["edit", "enable", "copy", "remove"] },
            },
          },
          state: "managed-ok",
          freshness: makeFreshness({ state: "unknown", current: null, upstream: "8f0ae3c7f25ec419631ade7e431fa2531503433e", checked_at: CHECKED_AT, probe: "git-ls-remote", cache_age_sec: 4000 }),
        }),
        makeElement({
          id: "mcp:my-proxy",
          domain: "mcp",
          spec: { name: "my-proxy", saved: null },
          observed: {
            agents: {
              "claude-code": { ...mcpCellBase, state: "disabled", source: "saved", target: "/Users/demo/.claude.json", transport: "stdio", entry: { command: "npx", args: ["-y", "my-proxy-mcp"], env: { MY_PROXY_TOKEN: "${MY_PROXY_TOKEN}", NODE_ENV: "<literal>" } }, vars: { MY_PROXY_TOKEN: { placeholder: "${MY_PROXY_TOKEN}", resolved: true, stored: true, kind: "env" } }, base: "5".repeat(64), command_available: null, notice: "已停用：条目保存在本工具里，启用时写回配置文件", actions: ["enable", "copy", "remove"] },
            },
          },
          state: "disabled",
          freshness: makeFreshness({ state: "current", current: "0.3.1", upstream: "0.3.1", checked_at: CHECKED_AT, probe: "npm-view", cache_age_sec: 60 }),
        }),
        makeElement({
          id: "mcp:remote-api",
          domain: "mcp",
          spec: { name: "remote-api", saved: null },
          observed: {
            agents: {
              codex: { ...mcpCellBase, state: "enabled", source: "file", target: "/Users/demo/.codex/config.toml", transport: "remote", entry: { url: "https://mcp.demo.example.com/endpoint", headers: { Authorization: "${DEMO_REMOTE_TOKEN}" } }, vars: { Authorization: { placeholder: "${DEMO_REMOTE_TOKEN}", resolved: true, stored: false, kind: "header" } }, base: "6".repeat(64), command_available: null, actions: ["edit", "disable", "copy", "remove"] },
            },
          },
          state: "managed-ok",
          freshness: makeFreshness({ state: "not-applicable" }),
        }),
        makeElement({
          id: "mcp:example-code",
          domain: "mcp",
          spec: {
            name: "example-code",
            saved: {
              definition: { command: "uvx", args: ["--from", "git+https://example.com/mcp/code@main", "example-code", "serve"], env: {}, extra_args_by_agent: { "claude-code": ["--client", "claude-code"], codex: ["--client", "codex"] } },
              valid: true,
            },
          },
          observed: {
            agents: {
              codex: { ...mcpCellBase, state: "invalid", source: "file", target: "/Users/demo/.codex/config.toml", reason: "command 必须是字符串", base: "7".repeat(64), blocked_reason: "条目结构无法识别（command 必须是字符串），只能删除或直接在配置文件里修正", actions: ["remove"] },
            },
          },
          state: "unsupported",
          freshness: makeFreshness({ state: "current", current: "a1b2c3d", upstream: "a1b2c3d", checked_at: CHECKED_AT, probe: "git-ls-remote", cache_age_sec: 200 }),
          actions: [{ id: "forget", risk: "write-local", needs_lock: true, offline_ok: true }],
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
        // 用户添加、还没拉取的集合：收敛七态里的 not-materialized 由这里演示
        //（原先由 MCP「已分配未写入」提供，该状态随服务库模型退役）
        makeElement({
          id: "skill:team-skills",
          domain: "skills",
          spec: {
            url: "https://github.com/example/team-skills.git",
            skill_paths: ["team-review"],
            agents: [],
            matrix_mode: true,
            new_skill_default: "disabled",
          },
          policy: { enabled: true },
          observed: { install_source: null, repo_cache: "/repo/agent-skill/repositories/team-skills",
            repo_cached: false,
            owner_marked: false,
            discovered: [],
            discovery_inbox: [],
            extra_links: [],
            discovery_error: null,
            blocked_reason: null,
            actions: ["update", "disable", "remove"] },
          state: "not-materialized",
          freshness: makeFreshness({ state: "unknown" }),
          actions: [{ id: "update", risk: "network", needs_lock: true, offline_ok: false }],
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
      recommendations: [
        {
          name: "agent-skills",
          description: "本项目作者维护的通用工作技能：合并审查、方案设计、改动纪律、验证、临时产物整理、面向读者写作、技术图示。",
          url: "https://github.com/ly87ing/agent-skills.git",
          skill_paths: ["safe-merge-review", "solution-shaping", "change-discipline", "verification", "artifact-hygiene", "reader-facing-writing", "technical-diagramming"],
          added: true,
          definition: { url: "https://github.com/ly87ing/agent-skills.git", skill_paths: ["safe-merge-review", "solution-shaping", "change-discipline", "verification", "artifact-hygiene", "reader-facing-writing", "technical-diagramming"] },
        },
      ],
      installed: [
        { agent: "claude-code", name: "artifact-hygiene", link_path: "~/.claude/skills/artifact-hygiene", target: "~/Library/Application Support/agent-manager/skill-versions/aaaa/content", origin: "managed", collection: "agent-skills" },
        { agent: "claude-code", name: "dead-one", link_path: "~/.claude/skills/dead-one", target: "~/gone/dead-one", origin: "dead-link", collection: null },
        { agent: "claude-code", name: "safe-merge-review", link_path: "~/.claude/skills/safe-merge-review", target: "~/elsewhere/safe-merge-review", origin: "collection-extra-link", collection: "agent-skills" },
        { agent: "claude-code", name: "synced", link_path: "~/.claude/skills/synced", target: null, origin: "native-dir", collection: null },
        { agent: "claude-code", name: "vendor-lint", link_path: "~/.claude/skills/vendor-lint", target: "~/.vendor-tool/skills/vendor-lint", origin: "external-link", collection: null },
        { agent: "codex", name: "artifact-hygiene", link_path: "~/.codex/skills/artifact-hygiene", target: "~/Library/Application Support/agent-manager/skill-versions/aaaa/content", origin: "managed", collection: "agent-skills" },
        { agent: "codex", name: "safe-merge-review", link_path: "~/.codex/skills/safe-merge-review", target: "~/Library/Application Support/agent-manager/skill-versions/bbbb/content", origin: "managed", collection: "agent-skills" },
      ],
    },
    { command: "skills.status" },
  );

  const demoRulesBody = "- Use Chinese (Simplified) for interactions and generated content.\n- Prefer the simplest local change.\n";
  // 来源（agent-skills 的 rules/core.md）已有新版：Codex 规则文件还停在上次写入的旧版
  const demoRulesBodyV2 = `${demoRulesBody}- After every change, run the smallest relevant verification that could fail.\n`;
  const rulesStatus = makeEnvelope(
    {
      domain: "rules",
      elements: [
        makeElement({
          id: "rules:codex_global",
          domain: "rules",
          spec: { name: "Codex 全局规则", group: "global-rules", kind: "codex", profile_id: null, target: "~/.codex/AGENTS.md" },
          policy: { enabled: true, agent_id: "codex" },
          observed: {
            target: "/Users/demo/.codex/AGENTS.md",
            target_exists: true,
            content: demoRulesBody,
            base: "aaaa3c8ae2687d47a8e7543d3c8c43ef8dd9f744e3a3e2ce9aa9ad7a90de12a2",
            matched_template: null,
            matched_source_rule: null,
            source_follow: {
              rule: "agent-skills/core",
              applied_revision: "demo-rule-v1",
              current_revision: "demo-rule-v2",
              file_unchanged: true,
              update_available: true,
            },
            blocked_reason: null,
          },
          state: "managed-ok",
          actions: [{ id: "write", risk: "write-local", needs_lock: true, offline_ok: true }],
        }),
        makeElement({
          id: "rules:claude_global",
          domain: "rules",
          spec: { name: "Claude Code 全局规则", group: "global-rules", kind: "claude", profile_id: null, target: "~/.claude/CLAUDE.md" },
          policy: { enabled: true, agent_id: "claude-code" },
          observed: {
            target: "/Users/demo/.claude/CLAUDE.md",
            target_exists: true,
            content: "# 我的 Claude 规则\n- 回答用简体中文。\n",
            base: "bbbb4b5c130887752148845080715867e6c3e6f40e69189067dd7d92db8b40c3",
            matched_template: { id: "tpl-demo-team", name: "团队规则" },
            matched_source_rule: null,
            source_follow: null,
            blocked_reason: null,
          },
          state: "managed-ok",
          actions: [{ id: "write", risk: "write-local", needs_lock: true, offline_ok: true }],
        }),
        makeElement({
          id: "rules:profile:cc-work",
          domain: "rules",
          spec: { name: "Claude · work 实例规则", group: "profile-rules", kind: "claude", profile_id: "cc-work", target: "~/.claude-work/CLAUDE.md" },
          policy: { enabled: true, agent_id: "cc-work" },
          observed: {
            target: "/Users/demo/.claude-work/CLAUDE.md",
            target_exists: false,
            content: null,
            base: "missing",
            matched_template: null,
            matched_source_rule: null,
            source_follow: null,
            blocked_reason: null,
          },
          state: "missing",
          actions: [{ id: "write", risk: "write-local", needs_lock: true, offline_ok: true }],
        }),
      ],
      templates: [
        {
          id: "tpl-demo-team",
          name: "团队规则",
          body: "# 我的 Claude 规则\n- 回答用简体中文。\n",
          revision: "demo-template-revision-1",
          origin: "user",
          created_at: "2026-09-20T08:00:00Z",
          updated_at: "2026-09-20T08:00:00Z",
        },
      ],
      source_rules: [
        { id: "agent-skills/core", collection: "agent-skills", name: "core", body: demoRulesBodyV2, revision: "demo-rule-v2", local: false },
      ],
      suggested_sources: [],
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
              { server: "example-search", env_key: "SEARCH_API_KEY", placeholder: "SEARCH_API_KEY", resolved: true, kind: "env" },
              { server: "example-search", env_key: "CRAWL_API_KEY", placeholder: "CRAWL_API_KEY", resolved: false, kind: "env" },
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
            third_party: true,
            // 实际配置为准：演示外部改过实例模型后的逐字段差异，可覆盖也可采纳
            base: "d".repeat(64),
            fields: [
              { field: "env.ANTHROPIC_AUTH_TOKEN", state: "same", definition: "${WORK_API_KEY}", live: "<hidden>" },
              { field: "model", state: "differs", definition: "claude-sonnet-demo", live: "claude-opus-demo" },
            ],
            blocked_reason: null,
          },
          state: "drifted",
          freshness: makeFreshness({ state: "not-applicable" }),
          actions: [
            { id: "apply", risk: "write-local", needs_lock: true, offline_ok: true },
            { id: "adopt", risk: "write-local", needs_lock: false, offline_ok: true },
          ],
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
    "resources:targets": makeEnvelope({ domain: "resources", targets: [
      { id: "claude-code", name: "Claude", group: "claude-code", kind: "claude", skills_dir: "~/.claude/skills", mcp_path: "~/.claude.json" },
      { id: "codex", name: "Codex", group: "codex", kind: "codex", skills_dir: "~/.codex/skills", mcp_path: "~/.codex/config.toml" },
      { id: "kimi-code", name: "Kimi Code", group: "kimi-code", kind: null },
    ] }, { command: "resources.targets" }),
    "mcp:status": mcpStatus,
    "skills:status": skillsStatus,
    "rules:status": rulesStatus,
    "secrets:status": secretsStatus,
    "profiles:status": profilesStatus,
    "usage:status": usageStatus,
    "usage:scan": makeEnvelope({ domain: "usage", action: "scan", files: 3, bytes: 24_576 }, { command: "usage.scan" }),
    "mcp:check": makeEnvelope({
      domain: "mcp", action: "check", server: "demo", agent: "claude-code", ok: true, stage: "done",
      message: "连接成功，提供 3 个工具", server_info: { name: "demo-mcp", version: "1.4.0" },
      tool_count: 3, tools: ["search", "fetch", "summarize"], detail: null, elapsed_ms: 2310,
    }, { command: "mcp.check" }),
    "usage:sessions": makeEnvelope({
      domain: "usage",
      action: "sessions",
      accounts: [
        { agent: "claude", kind: "claude", sessions: [
          { id: "3f1c2a9e-0d6b-4b8e-9f0a-1c2d3e4f5a6b", cwd: "/Users/demo/code/agent-manager", title: "托盘面板显示额度", updated_at: new Date(Date.now() - 4 * 60_000).toISOString() },
          { id: "9a8b7c6d-5e4f-4a3b-8c2d-1e0f9a8b7c6d", cwd: "/Users/demo/code/site", title: null, updated_at: new Date(Date.now() - 26 * 3600_000).toISOString() },
        ] },
        { agent: "codex", kind: "codex", sessions: [
          { id: "01a11ec8-518b-7913-8e69-3f6761d218df", cwd: "/Users/demo/code/agent-manager", title: "补齐会话列表的测试", updated_at: new Date(Date.now() - 3 * 3600_000).toISOString() },
        ] },
      ],
    }, { command: "usage.sessions" }),
    "aliases:status": makeEnvelope({ domain: "aliases", elements: [] }, { command: "aliases.status" }),
    // 程序：与本机实测同形（2026-09-30）——npm 全局的 wecom-cli、npx 固定版本、uvx 跟分支、应用自带、没声明的候选
    "programs:status": makeEnvelope(
      {
        domain: "programs",
        elements: [
          {
            id: "programs:npm:@wecom/cli", program: "npm:@wecom/cli", kind: "npm", name: "@wecom/cli", bins: ["wecom-cli"], all_bins: ["wecom-cli"],
            mode: "delegated", owner: "npm-global", version: "1.3.4",
            installs: [{ path: "~/.npm-global/bin/wecom-cli", real: "~/.npm-global/lib/node_modules/@wecom/cli/bin/wecom.js", owner: "npm-global", program: "npm:@wecom/cli", version: "1.3.4", app: null, package_dir: "~/.npm-global/lib/node_modules/@wecom/cli" }],
            install: [], runners: [],
            dependents: ["wecomcli-doc", "wecomcli-message", "wecomcli-smartsheet"].map((name) => ({ kind: "skill", name, agents: ["claude-code", "codex"], source: "declared", bins: ["wecom-cli"], any: false })),
            problems: [], commands: { install: "npm install -g @wecom/cli", upgrade: "npm install -g @wecom/cli@latest", uninstall: "npm uninstall -g @wecom/cli" },
            managed_versions: [], managed_current: null,
            freshness: { state: "behind", current: "1.3.4", upstream: "1.3.5", checked_at: new Date().toISOString(), probe: "npm-view", cache_age_sec: 60 },
          },
          {
            id: "programs:npm:chrome-devtools-mcp", program: "npm:chrome-devtools-mcp", kind: "npm", name: "chrome-devtools-mcp", bins: [], all_bins: [],
            mode: "runner", owner: null, version: null, installs: [], install: [],
            runners: [{ server: "chrome-devtools", agent: "claude-code", runner: "npx", version: "1.9.0", pinned: true }],
            dependents: [{ kind: "mcp", name: "chrome-devtools", agents: ["claude-code", "codex"], source: "mcp", bins: [], any: false }],
            problems: [], commands: {}, managed_versions: [], managed_current: null,
            freshness: { state: "behind", current: "1.9.0", upstream: "1.10.1", checked_at: new Date().toISOString(), probe: "npm-view", cache_age_sec: 60 },
          },
          {
            id: "programs:git:example.com/mcp/code", program: "git:example.com/mcp/code", kind: "git", name: "example.com/mcp/code", bins: [], all_bins: [],
            mode: "runner", owner: null, version: null, installs: [], install: [],
            runners: [{ server: "example-code", agent: "codex", runner: "uvx", version: "main", pinned: false }],
            dependents: [{ kind: "mcp", name: "example-code", agents: ["claude-code", "codex"], source: "mcp", bins: [], any: false }],
            problems: [{ code: "unpinned", server: "example-code", agent: "codex", version: "main" }], commands: {}, managed_versions: [], managed_current: null,
            freshness: { state: "unknown", current: "main", upstream: "7a2968335f2198b966864de1ce3655c8e485a653", checked_at: new Date().toISOString(), probe: "git-ls-remote", cache_age_sec: 60 },
          },
          {
            id: "programs:npm:json", program: "npm:json", kind: "npm", name: "json", bins: ["json"], all_bins: ["json"],
            mode: "missing", owner: null, version: null, installs: [], install: [{ kind: "node", package: "json", bins: ["json"] }], runners: [],
            dependents: [{ kind: "skill", name: "jsontool", agents: ["claude-code"], source: "declared", bins: ["json"], any: false }],
            problems: [{ code: "missing", bins: ["json"] }], commands: { install: "npm install -g json" }, managed_versions: [], managed_current: null,
            freshness: { state: "unknown", current: null, upstream: null, checked_at: null, probe: null, cache_age_sec: null },
          },
          {
            id: "programs:path:/Applications/ChatGPT.app/Contents/Resources/cua_node/bin/node_repl", program: "path:/Applications/ChatGPT.app/Contents/Resources/cua_node/bin/node_repl", kind: "path",
            name: "/Applications/ChatGPT.app/Contents/Resources/cua_node/bin/node_repl", bins: ["node_repl"], all_bins: ["node_repl"], mode: "observed", owner: "app", version: null,
            installs: [{ path: "/Applications/ChatGPT.app/Contents/Resources/cua_node/bin/node_repl", real: "/Applications/ChatGPT.app/Contents/Resources/cua_node/bin/node_repl", owner: "app", program: null, version: null, app: "ChatGPT", package_dir: null }],
            install: [], runners: [], dependents: [{ kind: "mcp", name: "node_repl", agents: ["codex"], source: "mcp", bins: ["node_repl"], any: false }],
            problems: [{ code: "version-unknown" }], commands: {}, managed_versions: [], managed_current: null,
            freshness: { state: "unknown", current: null, upstream: null, checked_at: null, probe: null, cache_age_sec: null },
          },
        ],
        suggestions: [{ skill: "xiaoyu-bug-loop", bin: "playwright-cli", program: null, line: "| `PLAYWRIGHT_CLI` | `command -v playwright-cli` |", found: true }],
        user_path: { dirs: ["~/.local/bin", "~/.npm-global/bin", "/usr/local/bin"], entry_candidates: ["~/.local/bin", "~/.npm-global/bin"], error: null },
        entry_dir: null,
        allow_scripts: [],
        links: {},
        backups: [],
      },
      { command: "programs.status" },
    ),
    "programs:preview": makeEnvelope(
      {
        program: "npm:@wecom/cli", action: "upgrade", target_version: "1.3.5",
        steps: [
          { kind: "move", from: "~/.npm-global/lib/node_modules/@wecom/cli", to: "~/Library/Application Support/io.github.ly87ing.agent-manager/programs/backups/demo/package" },
          { kind: "exec", command: "npm install -g --prefix ~/.npm-global --no-fund --no-audit --ignore-scripts @wecom/cli@1.3.5" },
        ],
        notes: ["改动前把 ~/.npm-global/lib/node_modules/@wecom/cli 整个移到备份，失败或回退时原样移回"],
        scripts_ignored: true, backup: "demo",
        manual: ["npm install -g --prefix ~/.npm-global --no-fund --no-audit --ignore-scripts @wecom/cli@1.3.5"],
      },
      { command: "programs.preview" },
    ),
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
          { id: "mcp:example-search", probe: "git-ls-remote", state: "unknown", current: null, upstream: "8f0ae3c7f25ec419631ade7e431fa2531503433e", error: "uvx-local-unavailable" },
          { id: "mcp:my-proxy", probe: "npm-view", state: "current", current: "0.3.1", upstream: "0.3.1", error: null },
          { id: "skill:agent-skills", probe: "git-ls-remote", state: "behind", current: "a1b2c3d4e5f60718293a4b5c6d7e8f9012a3b4c5", upstream: "f0e1d2c3b4a5968778695a4b3c2d1e0f9a8b7c6d", error: null },
        ],
        summary: { probed: 6, current: 1, behind: 3, ahead: 1, unknown: 1 },
      },
      { command: "system.refresh-upstream" },
    ),
    // 接管发现：覆盖五类来源（外部目录、已定义集合的计划外链接、原生真实目录、断链、未登记助手）
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
        summary: { agents: 2, skills_symlink: 3, skills_collection: 1, skills_native: 1, skills_cleanup: 1 },
      },
      { command: "system.discover" },
    ),
  };
}

export function createMockEngineClient(options: { fresh?: boolean } = {}): EngineClient {
  const routes = demoEnvelopes();
  if (options.fresh) {
    // 全新机器：还没有任何技能集合，推荐来源都没添加，各账号目录里也还没有本工具的链接
    const skills = routes["skills:status"].data as { elements: unknown[]; recommendations: Array<{ added: boolean }>; installed: unknown[] };
    skills.elements = [];
    skills.recommendations = skills.recommendations.map((item) => ({ ...item, added: false }));
    skills.installed = [];
  }
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
      mcp: { definitions: {} },
      skills: { "agent-skills": { enabled: true, new_skill_default: "disabled" } },
  };
  // fresh：全新机器，本地层空着——首次使用引导据此出现
  const sqlitePolicy: Record<string, unknown> = options.fresh
    ? {}
    : {
      browser_us_sync: { enabled: false },
      mcp: { definitions: {} },
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

  // ---- 规则：实际文件为准 + 模板（内存态演示） ----
  type MockRuleTemplate = { id: string; name: string; body: string; revision: string; origin: string; created_at: string; updated_at: string };
  type MockRulesData = {
    elements: Array<{ id: string; state: string; spec: { name: string }; observed: Record<string, unknown> }>;
    templates: MockRuleTemplate[];
    source_rules: Array<{ id: string; name: string; collection: string; body: string; revision: string }>;
  };
  const rulesData = () => routes["rules:status"].data as MockRulesData;
  const argOf = (args: string[], flag: string): string | undefined => {
    const index = args.indexOf(flag);
    return index >= 0 ? args[index + 1] : undefined;
  };
  const ruleError = (command: string, message: string): Envelope =>
    makeEnvelope({}, { command, ok: false, errors: [{ code: "RULE_ERROR", element: null, message }] });
  const refreshRuleMatches = () => {
    const data = rulesData();
    for (const element of data.elements) {
      const content = element.observed.content;
      const match = typeof content === "string" ? data.templates.find((item) => item.body === content) : undefined;
      element.observed.matched_template = match ? { id: match.id, name: match.name } : null;
      const rule = typeof content === "string" ? data.source_rules.find((item) => item.body === content) : undefined;
      element.observed.matched_source_rule = rule ? { id: rule.id, name: rule.name, collection: rule.collection } : null;
    }
  };
  let ruleRevision = 0;
  const applyRules = (args: string[], runId: string): Envelope => {
    const data = rulesData();
    const target = data.elements.find((item) => item.id === argOf(args, "--only"));
    if (!target) return ruleError("rules.apply", "参数错误: --only 目标无效");
    if (argOf(args, "--base") !== target.observed.base) {
      return ruleError("rules.apply", `${target.spec.name} 已在外部修改，没有写入`);
    }
    let next: string | undefined;
    if (args.includes("--value")) next = argOf(args, "--value");
    else if (args.includes("--source-rule")) next = data.source_rules.find((item) => item.id === argOf(args, "--source-rule"))?.body;
    else if (args.includes("--template")) next = data.templates.find((item) => item.id === argOf(args, "--template"))?.body;
    else if (args.includes("--from")) {
      const from = data.elements.find((item) => item.id === argOf(args, "--from"))?.observed.content;
      next = typeof from === "string" ? from : undefined;
    }
    if (next === undefined) return ruleError("rules.apply", "找不到要写入的内容");
    const steps: { name: string; ok: boolean; exit_code: number }[] = [];
    const current = target.observed.content;
    const follow = target.observed.source_follow as { file_unchanged?: boolean } | null;
    if (!args.includes("--value") && typeof current === "string" && current.trim() !== ""
      && !data.templates.some((item) => item.body === current)
      && !data.source_rules.some((item) => item.body === current)
      && !follow?.file_unchanged) {
      const now = new Date().toISOString();
      data.templates.push({ id: `tpl-backup-${data.templates.length + 1}`, name: `备份 · ${target.spec.name} · ${now.slice(0, 19).replace("T", " ")}`, body: current, revision: `demo-template-revision-${++ruleRevision}`, origin: "backup", created_at: now, updated_at: now });
      steps.push({ name: `rules:backup ${target.id}`, ok: true, exit_code: 0 });
    }
    const rule = data.source_rules.find((item) => item.id === argOf(args, "--source-rule"));
    // 从来源规则写入：记下跟随；用别的内容写入：不再跟随
    target.observed.source_follow = rule
      ? { rule: rule.id, applied_revision: rule.revision, current_revision: rule.revision, file_unchanged: true, update_available: false }
      : null;
    target.observed.content = next;
    target.observed.target_exists = true;
    target.observed.base = `demo-base-${++ruleRevision}`;
    target.state = "managed-ok";
    steps.push({ name: `rules:write ${target.id}`, ok: true, exit_code: 0 });
    refreshRuleMatches();
    recordRun(runId, "rules.apply");
    return makeEnvelope({ domain: "rules", steps, skipped: [] }, { command: "rules.apply", run_id: runId, lock: { held: true, pid: 4242 } });
  };
  const saveRuleTemplate = (args: string[]): Envelope => {
    const data = rulesData();
    const name = (argOf(args, "--name") ?? "").trim();
    const body = argOf(args, "--value") ?? "";
    if (!name) return ruleError("rules.save-template", "模板名称不能为空");
    const id = argOf(args, "--template");
    const now = new Date().toISOString();
    if (data.templates.some((item) => item.name === name && item.id !== id)) return ruleError("rules.save-template", "已有同名模板");
    let template: MockRuleTemplate;
    if (id) {
      const existing = data.templates.find((item) => item.id === id);
      if (!existing) return ruleError("rules.save-template", "模板不存在");
      if (existing.revision !== argOf(args, "--revision")) return ruleError("rules.save-template", "模板已被改过，请重新打开后再保存");
      Object.assign(existing, { name, body, revision: `demo-template-revision-${++ruleRevision}`, updated_at: now });
      template = existing;
    } else {
      template = { id: `tpl-demo-${++ruleRevision}`, name, body, revision: `demo-template-revision-${ruleRevision}`, origin: "user", created_at: now, updated_at: now };
      data.templates.push(template);
    }
    refreshRuleMatches();
    return makeEnvelope({ domain: "rules", template }, { command: "rules.save-template" });
  };
  const deleteRuleTemplate = (args: string[]): Envelope => {
    const data = rulesData();
    const id = argOf(args, "--template");
    const index = data.templates.findIndex((item) => item.id === id);
    if (index < 0) return ruleError("rules.delete-template", "模板不存在");
    data.templates.splice(index, 1);
    refreshRuleMatches();
    return makeEnvelope({ domain: "rules", deleted: id }, { command: "rules.delete-template" });
  };

  // ---- aliases：两个 rc 文件受管区块里的实际命令（有状态；内置只作推荐，旧版保存的作建议）----
  const TZ_LANG = 'TZ="America/Los_Angeles" LANG="en_US.UTF-8" LC_ALL="en_US.UTF-8"';
  const aliasState: Record<string, string> = {
    claude: `${TZ_LANG} claude --dangerously-skip-permissions`,
    codex: `${TZ_LANG} codex`,
    chrome: 'open --env TZ="America/Los_Angeles" -n -a "Google Chrome" --args --lang=en-US',
  };
  const aliasRecommended = ["grok"].map((name) => ({ name, value: `${TZ_LANG} ${name}`, line: `alias ${name}='${TZ_LANG} ${name}'`, source: "agent-skills" }));
  let aliasLegacy = [
    {
      id: "legacy-0a1b2c3d4e5f6a7b",
      name: "cc",
      value: 'CLAUDE_CONFIG_DIR="$HOME/.claude-cc" claude',
      line: `alias cc='CLAUDE_CONFIG_DIR="$HOME/.claude-cc" claude'`,
      actual: null as string | null,
    },
  ];
  const aliasStatus = (): Envelope =>
    makeEnvelope(
      makeAliasStatus(aliasState, {
        recommendations: aliasRecommended.filter((item) => !(item.name in aliasState)),
        legacy: aliasLegacy.filter((item) => aliasState[item.name] !== item.value),
        external: { gp: "git push" },
      }),
      { command: "aliases.status" },
    );
  const applyAliases = (args: string[], runId: string): Envelope => {
    const set = argOf(args, "--set");
    const removed = argOf(args, "--remove");
    if (set) {
      const rename = argOf(args, "--rename-from");
      if (rename && rename !== set) delete aliasState[rename];
      aliasState[set] = argOf(args, "--value") ?? "";
    } else if (removed) {
      delete aliasState[removed];
    } else if (args.includes("--recommended")) {
      const item = aliasRecommended.find((entry) => entry.name === argOf(args, "--recommended"));
      if (item) aliasState[item.name] = item.value;
    } else if (args.includes("--legacy")) {
      const item = aliasLegacy.find((entry) => entry.id === argOf(args, "--legacy"));
      if (item) aliasState[item.name] = item.value;
      aliasLegacy = aliasLegacy.filter((entry) => entry !== item);
    }
    return applyEnvelope("aliases", args, runId);
  };
  const previewAliases = (args: string[]): Envelope => {
    const candidate = { ...aliasState };
    const set = argOf(args, "--set");
    const removed = argOf(args, "--remove");
    if (set) {
      const rename = argOf(args, "--rename-from");
      if (rename && rename !== set) delete candidate[rename];
      candidate[set] = argOf(args, "--value") ?? "";
    } else if (removed) {
      delete candidate[removed];
    } else if (args.includes("--recommended")) {
      const item = aliasRecommended.find((entry) => entry.name === argOf(args, "--recommended"));
      if (item) candidate[item.name] = item.value;
    } else if (args.includes("--legacy")) {
      const item = aliasLegacy.find((entry) => entry.id === argOf(args, "--legacy"));
      if (item) candidate[item.name] = item.value;
    }
    const content = `# 演示数据；正式预览按实际文件计算\n${Object.entries(candidate).map(([name, value]) => `alias ${name}='${value}'`).join("\n")}\n`;
    return makeEnvelope({
      files: [
        { target: "aliases:zshrc", path: "/Users/demo/.zshrc", exists: true, content },
        { target: "aliases:zprofile", path: "/Users/demo/.zprofile", exists: true, content },
      ],
      skipped: [],
    }, { command: "aliases.preview" });
  };

  const setSecret = (args: string[], runId: string): Envelope => {
    const variable = args[args.indexOf("--var") + 1] ?? "";
    const value = args[args.indexOf("--value") + 1] ?? "";
    const normalize = (placeholder: string): string => /^\$\{([^}]+)\}$/.exec(placeholder)?.[1] ?? placeholder;
    const secretsData = routes["secrets:status"].data as {
      elements: Array<{ observed: { variables?: Array<{ placeholder?: string; env_key?: string; resolved?: boolean; stored?: boolean }>; unresolved?: number; source?: { names?: string[] } } }>;
    };
    const variables = secretsData.elements[0]?.observed.variables ?? [];
    const source = secretsData.elements[0]?.observed.source;
    if (source) {
      const names = source.names ?? [];
      source.names = value ? [...new Set([...names, variable])] : names.filter((name) => name !== variable);
    }
    for (const item of variables) {
      if (normalize(item.placeholder ?? item.env_key ?? "") !== variable) continue;
      item.stored = value.length > 0;
      item.resolved = value.length > 0 || externallyResolvedSecrets.has(variable);
    }
    if (secretsData.elements[0]) {
      secretsData.elements[0].observed.unresolved = variables.filter((item) => item.resolved === false).length;
    }

    const mcpData = routes["mcp:status"].data as {
      elements: Array<{ observed: { agents?: Record<string, { vars?: Record<string, { placeholder?: string; resolved?: boolean; stored?: boolean }> }> } }>;
    };
    for (const element of mcpData.elements) {
      for (const cell of Object.values(element.observed.agents ?? {})) {
        for (const resolution of Object.values(cell.vars ?? {})) {
          if (normalize(resolution.placeholder ?? "") === variable) {
            resolution.resolved = value.length > 0 || externallyResolvedSecrets.has(variable);
            resolution.stored = value.length > 0;
          }
        }
      }
    }
    recordRun(runId, "secrets.set");
    return makeEnvelope({ variable, stored: value.length > 0 }, { command: "secrets.set", run_id: runId });
  };

  // 配置文件原文编辑：只按整文件基准比对
  const configFiles: Record<string, { path: string; format: string; content: string; base: string }> = {
    "agents:claude-code": {
      path: "~/.claude/settings.json",
      format: "json",
      content: '{\n  "env": {\n    "ANTHROPIC_AUTH_TOKEN": "demo-token"\n  },\n  "effortLevel": "high",\n  "theme": "dark"\n}\n',
      base: "f".repeat(64),
    },
    "agents:codex": {
      path: "~/.codex/config.toml",
      format: "toml",
      content: '# 我的 codex 配置\nmodel = "gpt-5"\nmodel_reasoning_effort = "high"\n',
      base: "e".repeat(64),
    },
  };
  const previewRuntime = (args: string[]): Envelope => {
    const element = argOf(args, "--only") ?? "";
    const file = configFiles[element];
    const agents = (routes["agents:status"].data as {
      elements: Array<{ id: string; observed: { current_config?: { base?: string; target?: string }; management?: ReturnType<typeof agentManagementFixture> } }>;
    }).elements;
    const observed = agents.find((item) => item.id === element)?.observed;
    const refuse = (code: string, message: string) => makeEnvelope({}, {
      command: "agents.preview-runtime", ok: false,
      errors: [{ code, element: element || null, message }],
    });
    if (!file || !observed?.management) return refuse("AGENT_ERROR", "没有这个助手的配置文件");
    if (argOf(args, "--base") !== observed.current_config?.base) {
      return refuse("AGENT_CONFIG_CHANGED", "配置文件已在外部修改，请刷新后重新预览");
    }
    let patch: Record<string, unknown>;
    try {
      patch = JSON.parse(argOf(args, "--runtime") ?? "") as Record<string, unknown>;
    } catch {
      return refuse("USAGE_ERROR", "--runtime 必须是 JSON 对象");
    }
    const changes = Object.entries(patch).flatMap(([key, value]) => {
      const field = observed.management!.runtime_fields.find((candidate) => candidate.key === key);
      const targets = field?.options?.find((option) => option.value === value)?.targets ?? field?.targets ?? [];
      return targets.map((target) => {
        let rendered = value;
        if (target.enabled_value && value !== null && value !== false && value !== "") {
          try { rendered = JSON.parse(target.enabled_value) as unknown; }
          catch { rendered = target.enabled_value; }
        }
        return { key: target.key, value: value === null || value === false || value === "" ? undefined : rendered };
      });
    });
    let content: string;
    if (file.format === "json") {
      const root = JSON.parse(file.content) as Record<string, unknown>;
      for (const change of changes) {
        const parts = change.key.split(".");
        let node = root;
        for (const part of parts.slice(0, -1)) {
          if (!node[part] || typeof node[part] !== "object") node[part] = {};
          node = node[part] as Record<string, unknown>;
        }
        if (change.value === undefined) delete node[parts[parts.length - 1]];
        else node[parts[parts.length - 1]] = change.value;
      }
      content = `${JSON.stringify(root, null, 2)}\n`;
    } else {
      const lines = file.content.trimEnd().split("\n");
      for (const change of changes) {
        const index = lines.findIndex((line) => line.startsWith(`${change.key} = `));
        if (change.value === undefined) {
          if (index >= 0) lines.splice(index, 1);
        } else if (index >= 0) lines[index] = `${change.key} = ${JSON.stringify(change.value)}`;
        else lines.push(`${change.key} = ${JSON.stringify(change.value)}`);
      }
      content = `${lines.join("\n")}\n`;
    }
    return makeEnvelope({
      domain: "agents", element, path: observed.current_config?.target ?? file.path,
      exists: true, file_base: "f".repeat(64), content,
    }, { command: "agents.preview-runtime" });
  };

  let baseSeed = 100;
  const configureAgent =(args: string[], runId: string): Envelope => {
    const agentId = args[args.indexOf("--only") + 1];
    const data = routes["agents:status"].data as {
      elements: Array<{ id: string; spec: Record<string, unknown>; observed: Record<string, unknown> }>;
    };
    const element = data.elements.find((item) => item.id === agentId);
    const refuse = (code: string, message: string) => makeEnvelope({}, {
      command: "agents.configure",
      ok: false,
      errors: [{ code, element: agentId ?? null, message }],
    });
    if (!element) return refuse("AGENT_ERROR", `没有这个助手：${agentId ?? ""}`);
    const observed = element.observed.current_config as {
      state: string;
      file: string;
      target?: string;
      config: Record<string, unknown> | null;
      base?: string | null;
      current?: Record<string, unknown> | null;
    } | undefined;
    const baseAt = args.indexOf("--base");
    if (baseAt < 0 || !observed?.base) return refuse("USAGE_ERROR", "agents:configure 需要 --base");
    if (args[baseAt + 1] !== observed.base) {
      return refuse("AGENT_CONFIG_CHANGED", "配置文件已在外部修改（与读取时不一致），文件与库都没有改动；请刷新后重试");
    }

    // 主账号只放官方登录：与引擎一致，切换接入与保存实际配置一律拒绝，只改运行参数
    if (args.includes("--use") || args.includes("--use-default") || args.includes("--save-current") || args.includes("--connection")) {
      return refuse("AGENT_MAIN_OFFICIAL_ONLY", "主账号只放官方登录，这里只改运行参数；第三方接口请新建一个账号");
    }
    const runtimeAt = args.indexOf("--runtime");
    if (runtimeAt < 0) return refuse("USAGE_ERROR", "agents:configure 需要 --runtime <json>");
    const settings =
      element.spec.settings && typeof element.spec.settings === "object" && !Array.isArray(element.spec.settings)
        ? structuredClone(element.spec.settings as Record<string, unknown>)
        : {};
    const liveConfig = (observed.config ?? {}) as Record<string, unknown>;
    const runtime = (liveConfig.runtime ?? {}) as Record<string, unknown>;
    const patch = JSON.parse(args[runtimeAt + 1]) as Record<string, unknown>;
    if ("automation_mode" in patch) delete runtime.automation_raw;
    for (const [key, value] of Object.entries(patch)) {
      if (value === null || value === false || value === "") {
        delete runtime[key];
        delete settings[key];
      } else {
        runtime[key] = value;
        settings[key] = value;
      }
    }
    observed.config = { ...liveConfig, runtime };
    if (observed.state === "missing") observed.state = "ready";
    element.spec.settings = settings;
    baseSeed += 1;
    observed.base = mockBase(baseSeed);
    recordRun(runId, "agents.configure");
    return makeEnvelope(
      { domain: "agents", action: "configure", element: agentId, mode: "runtime", base: observed.base },
      { command: "agents.configure", run_id: runId, lock: { held: true, pid: 4242 } },
    );
  };

  /**
   * 浏览器验收用的 DEV mock 钩子（只在 ?mock=1 下存在）：模拟「本工具以外改了配置文件」，
   * 与切换密钥文件是否可用。界面缓存里的写入基准随之过期，下一次写入会被拒并刷新。
   */
  if (typeof window !== "undefined") {
    (window as unknown as { __agentManagerMock?: unknown }).__agentManagerMock = {
      externalEdit(agentId = "agents:claude-code") {
        const data = routes["agents:status"].data as { elements: Array<{ id: string; observed: Record<string, unknown> }> };
        const observed = data.elements.find((item) => item.id === agentId)?.observed.current_config as
          | { state: string; config: Record<string, unknown> | null; base?: string | null }
          | undefined;
        if (!observed) return false;
        baseSeed += 1;
        observed.state = "ready";
        const config = observed.config ?? {};
        const runtime = (config.runtime ?? {}) as Record<string, unknown>;
        observed.config = { ...config, model: "edited-outside", runtime: { ...runtime, claude_effort_level: "low" } };
        observed.base = mockBase(baseSeed);
        return true;
      },
      setSecretSourceReady(ready: boolean) {
        const secrets = routes["secrets:status"].data as { elements: Array<{ observed: Record<string, unknown> }> };
        if (secrets.elements[0]) {
          secrets.elements[0].observed.source = ready
            ? { configured: true, path: "~/agent-manager/secrets/mcp.env", ready: true, error: null }
            : { configured: false, path: null, ready: false, error: null };
        }
      },
    };
  }

  return {
    async run(command, args = [], runOptions = {}) {
      // 模拟进程耗时，让加载/进行中的 UI 状态可见
      await new Promise((resolve) => setTimeout(resolve, 120));
      const runId = runOptions.runId ?? "20260906T020000Z-mockrun";

      const domain = command.replaceAll(".", ":").split(":")[0];
      const key = argOf(args, "--key") ?? "";
      if (["agents", "accounts", "profiles", "profile", "aliases"].includes(domain)
          || command === "secrets:migrate-legacy"
          || (["policy:set", "policy:delete"].includes(command) && /^(agents|aliases)(\.|$)/.test(key))) {
        return makeEnvelope({}, { command, ok: false, errors: [{ code: "FEATURE_RETIRED", element: null, message: "账号与启动命令管理已移除；本工具仅管理扩展。" }] });
      }
      // ---- 写路径与有状态命令 ----
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
          { run_id: id, operations: [{ id: "op-mock", operation_type: "mcp:apply", target: "mcp:example-search", detail: { result: "ok" }, created_at: "2026-09-05T23:59:00Z" }] },
          { command: "runs.get" },
        );
      }
      if (command === "runs:clear") {
        const deleted = runs.length;
        runs.splice(0, runs.length);
        return makeEnvelope({ deleted }, { command: "runs.clear" });
      }
      if (command === "agents:configure") return configureAgent(args, runId);
      if (command === "mcp:check") {
        const agent = argOf(args, "--agent") ?? "claude-code";
        const id = argOf(args, "--only") ?? "";
        const status = routes["mcp:status"].data as { elements: Array<{ id: string; observed: { agents: Record<string, { base: string; extra: Record<string, unknown>; entry: { startup_timeout_sec?: number } }> } }> };
        const cell = status.elements.find((item) => item.id === id)?.observed.agents[agent];
        const failed = agent === "codex";
        return makeEnvelope({ ...routes["mcp:check"].data as object, agent, server: id.replace(/^mcp:/, ""), base: cell?.base,
          ok: !failed, stage: failed ? "initialize" : "done", exit_code: failed ? 1 : null,
          message: failed ? "命令退出了，没有回应初始化" : "连接成功，提供 3 个工具",
          detail: failed ? "Demo: server exited before initialize; credential value ***" : null,
          server_info: failed ? null : { name: "demo-mcp", version: "1.4.0" }, tools: failed ? null : ["search", "fetch", "summarize"], tool_count: failed ? null : 3,
          cwd: cell?.extra.cwd ?? argOf(args, "--cwd") ?? "/Users/demo", cwd_source: cell?.extra.cwd ? "config" : argOf(args, "--cwd") ? "selected" : "home",
          startup_timeout_sec: cell?.entry.startup_timeout_sec ?? (agent === "codex" ? 10 : 30), tool_timeout_sec: agent === "codex" ? 60 : 30,
        }, { command: "mcp.check" });
      }
      if (command === "mcp:apply" && argOf(args, "--action") === "clean-markers") {
        const status = routes["mcp:status"].data as { elements: Array<{ id: string; observed: { agents: Record<string, { base: string; legacy_marker: boolean; notice: string | null; actions: string[] }> } }> };
        const cell = status.elements.find((item) => item.id === argOf(args, "--only"))?.observed.agents[argOf(args, "--agent") ?? ""];
        if (cell) { cell.legacy_marker = false; cell.notice = null; cell.base = mockBase(++baseSeed); cell.actions = cell.actions.filter((action) => action !== "clean-markers"); }
        return applyEnvelope("mcp", args, runId);
      }
      if (command === "agents:preview-runtime") return previewRuntime(args);
      if (command === "accounts:command") {
        const only = argOf(args, "--only") ?? "";
        const operation = argOf(args, "--operation") ?? "launch";
        const launch_value = `"/demo/agent-manager" --account-launch --home "/demo/home" --data-dir "/demo/data" --only "${only}" --operation ${operation} --`;
        return makeEnvelope({ launch_value, terminal_command: `${argOf(args, "--shell") === "powershell" ? "& " : ""}${launch_value}`, controlled: true }, { command: "accounts.command" });
      }
      if (command === "profiles:create") {
        const id = (argOf(args, "--only") ?? "").replace(/^profiles:/, "");
        return policySet(["--layer", "local", "--key", `agents.definitions.${id}`, "--value", argOf(args, "--definition") ?? "{}", "--expect-definition", "null"]);
      }
      if (command === "agents:auth-status") {
        const only = args[args.indexOf("--only") + 1] ?? "";
        // 全新机器：本机文件说明不了登录状态，点「确认」（--probe）才用官方命令查
        if (options.fresh && !args.includes("--probe")) {
          return makeEnvelope(
            { element: only, state: "unknown", logged_in: null, method: null, account: null, third_party: false, mixed: false, reason: "本机登录记录里没有账号；需要时用官方命令确认" },
            { command: "agents.auth-status" },
          );
        }
        if (options.fresh) {
          return makeEnvelope(
            { element: only, state: "logged-in", logged_in: true, method: "claude.ai", account: { email: "YOUR_EMAIL@example.com", org: null, plan: "Max 5x" }, third_party: false, mixed: false, reason: null },
            { command: "agents.auth-status" },
          );
        }
        return makeEnvelope(
          { element: only, state: "logged-out", logged_in: false, method: null, third_party: only.startsWith("profiles:"), mixed: false, reason: null },
          { command: "agents.auth-status" },
        );
      }
      if (command === "mcp:preview") {
        const agent = argOf(args, "--agent") ?? "";
        const to = argOf(args, "--to");
        const action = argOf(args, "--action");
        const name = (argOf(args, "--only") ?? "").replace(/^mcp:/, "");
        const status = routes["mcp:status"].data as {
          targets: Array<{ agent: string; path: string; format: string; exists: boolean }>;
          elements: Array<{ id: string; observed: { agents: Record<string, { entry?: Record<string, unknown>; source: string }> } }>;
        };
        const target = status.targets.find((item) => item.agent === (to ?? agent));
        if (!target) throw new EngineBridgeError({ kind: "invalid_args", message: "找不到 MCP 目标" });
        const source = status.elements.find((item) => item.id === `mcp:${name}`);
        if (action === "clean-markers") {
          const entry = source?.observed.agents[agent]?.entry ?? {};
          const content = `# Demonstration only\n[mcp_servers.${name}]\ncommand = ${JSON.stringify(entry.command)}\nargs = ${JSON.stringify(entry.args ?? [])}\n`;
          const original = `${content}_MANAGED_BY = "agent-manager"\n`;
          return makeEnvelope({ files: [{ path: target.path, exists: true, original, content }] }, { command: "mcp.preview" });
        }
        if (action === "remove") {
          if (source?.observed.agents[agent]?.source === "saved") {
            return makeEnvelope({ files: [] }, { command: "mcp.preview" });
          }
          const remaining = status.elements.flatMap((item) => {
            const cell = item.observed.agents[agent];
            return item.id === `mcp:${name}` || !cell || cell.source === "saved" || !cell.entry
              ? [] : [{ name: item.id.replace(/^mcp:/, ""), entry: cell.entry }];
          });
          const content = target.format === "codex-toml"
            ? `# 演示数据；正式预览按实际文件计算\n${remaining.map((item) => {
              const fields = ["command", "args", "url", "enabled"]
                .filter((field) => item.entry[field] !== undefined)
                .map((field) => `${field} = ${JSON.stringify(item.entry[field])}`);
              return `[mcp_servers.${item.name}]\n${fields.join("\n")}`;
            }).join("\n\n")}\n`
            : JSON.stringify({ [target.format === "opencode-json" ? "mcp" : "mcpServers"]:
              Object.fromEntries(remaining.map((item) => [item.name, item.entry])) }, null, 2);
          return makeEnvelope({ files: [{ path: target.path, exists: target.exists, content }] }, { command: "mcp.preview" });
        }
        const spec = to ? source?.observed.agents[agent]?.entry ?? {} : JSON.parse(argOf(args, "--spec") ?? "{}") as Record<string, unknown>;
        const content = target.format === "codex-toml"
          ? `[mcp_servers.${name}]\ncommand = ${JSON.stringify(spec.command ?? "npx")}\nargs = ${JSON.stringify(spec.args ?? [])}\n`
          : JSON.stringify({ [target.format === "opencode-json" ? "mcp" : "mcpServers"]: { [name]: spec } }, null, 2);
        return makeEnvelope({ files: [{ path: target.path, exists: target.exists, content }] }, { command: "mcp.preview" });
      }
      if (command === "profiles:preview") {
        const raw = argOf(args, "--definition") ?? argOf(args, "--candidate");
        if (!raw) throw new EngineBridgeError({ kind: "invalid_args", message: "缺少实例定义" });
        const definition = JSON.parse(raw) as { profile?: {
          kind?: string; config_dir?: string; env?: Record<string, string>;
          model?: string; base_url?: string; env_key?: string;
        } };
        const profile = definition.profile ?? {};
        const isCodex = profile.kind === "codex";
        const dir = (profile.config_dir ?? "~/.claude-new").replace(/\/$/, "");
        const path = `${dir}/${isCodex ? "config.toml" : "settings.json"}`;
        const officialCodex = isCodex && !profile.base_url && !profile.env_key;
        const content = officialCodex ? "" : isCodex
          ? [`# 演示数据；正式预览按实际文件计算`,
              ...(profile.model ? [`model = ${JSON.stringify(profile.model)}`] : []),
              ...(profile.base_url ? [`base_url = ${JSON.stringify(profile.base_url)}`] : []),
              ...(profile.env_key ? [`env_key = ${JSON.stringify(profile.env_key)}`] : []),
            ].join("\n") + "\n"
          : JSON.stringify({
              _demo_notice: "演示数据；正式预览按实际文件计算",
              ...(profile.env ? { env: profile.env } : {}),
              ...(profile.model ? { model: profile.model } : {}),
            }, null, 2) + "\n";
        const items = (argOf(args, "--items") ?? "").split(",").filter(Boolean);
        const exact = args.includes("--exact");
        return makeEnvelope({
          path, exists: false, base: "missing", content, will_write: !officialCodex,
          existing_profile_skipped: false, files: [],
          seed_results: items.map((item) => ({ item, state: "absent", detail: `${item}：演示数据，正式预览按实际文件计算` })),
          ...(exact ? { file_base: "missing", content_base: "d".repeat(64) } : {}),
        }, { command: "profiles.preview" });
      }
      if (command === "profiles:seed") {
        const id = (argOf(args, "--only") ?? "").replace(/^profiles:/, "");
        const items = (argOf(args, "--items") ?? "").split(",").filter(Boolean);
        const demo: Record<string, { state: string; detail: string; skipped?: string[] }> = {
          permissions: { state: "copied", detail: "已带上权限规则" },
          statusline: { state: "absent", detail: "默认环境没有设置状态栏" },
          rules: { state: "exists", detail: "独立环境已有 CLAUDE.md，没有改" },
          mcp: { state: "copied", detail: "已复制 fs；含明文凭据或格式不认识、没有复制：gh（需要的话在 MCP 页逐条复制）", skipped: ["gh"] },
        };
        return makeEnvelope(
          {
            domain: "profiles",
            action: "seed",
            element: `profiles:${id}`,
            results: items.map((item) => ({ item, ...(demo[item] ?? { state: "absent", detail: "" }) })),
          },
          { command: "profiles.seed", run_id: runId },
        );
      }
      if (command === "rules:apply") return applyRules(args, runId);
      if (command === "rules:save-template") return saveRuleTemplate(args);
      if (command === "rules:delete-template") return deleteRuleTemplate(args);
      if (command === "aliases:status") return aliasStatus();
      if (command === "aliases:preview") return previewAliases(args);
      if (command === "aliases:apply") return applyAliases(args, runId);
      if (command === "aliases:discard-legacy") {
        const id = argOf(args, "--legacy");
        aliasLegacy = aliasLegacy.filter((entry) => entry.id !== id);
        return makeEnvelope({ domain: "aliases", discarded: id, source_removed: aliasLegacy.length === 0 }, { command: "aliases.discard-legacy" });
      }
      if (command.endsWith(":apply") || command === "profiles:adopt" || command === "profiles:apply-definition") {
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
