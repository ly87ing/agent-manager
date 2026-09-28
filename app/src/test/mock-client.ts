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
                credential: { configured: true, kind: "token", variable: null, saved_var: null },
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
              current: { profile: null, default: false, last_written: null, unverified: [], drifted_from: null },
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
              current: { profile: null, default: true, official: true, last_written: null, unverified: [], drifted_from: null },
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
              codex: { ...mcpCellBase, state: "enabled", source: "file", target: "/Users/demo/.codex/config.toml", transport: "stdio", entry: { command: "npx", args: ["-y", "chrome-devtools-mcp@1.2.0"], env: {}, startup_timeout_sec: 120 }, extra: { cwd: "/Users/demo/work", enabled_tools: ["take_screenshot"] }, base: "2".repeat(64), legacy_marker: true, notice: "条目含旧版 Agent Manager 标记，编辑或启停时会顺带清除", command_available: true, actions: ["edit", "disable", "copy", "remove"] },
            },
          },
          state: "managed-ok",
          freshness: makeFreshness({ state: "behind", current: "1.2.0", upstream: "1.8.0", checked_at: CHECKED_AT, probe: "npm-view", cache_age_sec: 1820 }),
        }),
        makeElement({
          id: "mcp:grok-search",
          domain: "mcp",
          spec: { name: "grok-search", saved: null },
          observed: {
            agents: {
              "claude-code": {
                ...mcpCellBase,
                state: "enabled",
                source: "file",
                target: "/Users/demo/.claude.json",
                transport: "stdio",
                entry: { command: "uvx", args: ["--from", "git+https://github.com/GuDaStudio/GrokSearch@grok-with-tavily", "grok-search"], env: { GROK_API_KEY: "<literal>", TAVILY_API_KEY: "${TAVILY_API_KEY}" } },
                vars: { TAVILY_API_KEY: { placeholder: "${TAVILY_API_KEY}", resolved: false, stored: false, kind: "env" } },
                base: "3".repeat(64),
                command_available: true,
                actions: ["edit", "disable", "copy", "remove"],
              },
              codex: { ...mcpCellBase, state: "disabled", source: "file", target: "/Users/demo/.codex/config.toml", transport: "stdio", entry: { command: "uvx", args: ["grok-search"], env: { GROK_API_KEY: "<literal>" } }, base: "4".repeat(64), command_available: true, actions: ["edit", "enable", "copy", "remove"] },
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
          id: "mcp:serena",
          domain: "mcp",
          spec: {
            name: "serena",
            saved: {
              definition: { command: "uvx", args: ["--from", "git+https://github.com/oraios/serena@main", "serena", "start-mcp-server"], env: {}, extra_args_by_agent: { "claude-code": ["--context", "claude-code"], codex: ["--context", "codex"] } },
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
            spec: { name: "Claude Code Hooks", group: "global-hooks", kind: "claude", scope: "hooks-key", profile_id: null, target: "~/.claude/settings.json" },
            policy: { enabled: true, agent_id: "claude-code" },
            observed: {
              target: "/Users/demo/.claude/settings.json",
              target_exists: true,
              hooks: {
                SessionStart: [{ hooks: [{ type: "command", command: "echo demo-hook" }] }],
                PreToolUse: [{ matcher: "Edit|MultiEdit|Write", hooks: [{ type: "command", command: "sh ~/agent-manager/agent-rule/hooks/block-dist-edits.sh", timeout: 30 }] }],
              },
              hook_count: 2,
              base: "demo-hooks-base-claude",
              cleanup: [{ id: "cleanup-0123456789abcdef", event: "PreToolUse", matcher: "Edit|MultiEdit|Write", command: "sh ~/agent-manager/agent-rule/hooks/block-dist-edits.sh" }],
              legacy: [{ id: "legacy-0123456789abcdef", source_key: "hooks.claude", event: "Stop", matcher: null, command: "echo saved-but-not-applied" }],
              blocked_reason: null,
            },
            state: "managed-ok",
            actions: [{ id: "write", risk: "write-local", needs_lock: true, offline_ok: true }],
          }),
          makeElement({
            id: "hooks:codex",
            domain: "hooks",
            spec: { name: "Codex Hooks", group: "global-hooks", kind: "codex", scope: "whole-file", profile_id: null, target: "~/.codex/hooks.json" },
            policy: { enabled: true, agent_id: "codex" },
            observed: {
              target: "/Users/demo/.codex/hooks.json",
              target_exists: false,
              hooks: {},
              hook_count: 0,
              base: "missing",
              cleanup: [],
              legacy: [],
              blocked_reason: null,
            },
            state: "missing",
            actions: [{ id: "write", risk: "write-local", needs_lock: true, offline_ok: true }],
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

  // ---- Hooks：实际配置为准 + 建议（内存态演示） ----
  type MockHookSuggestion = { id: string; event: string; matcher: string | null; command: string };
  type MockHooksElement = { id: string; state: string; spec: { name: string; kind: string }; observed: { target: string; hooks: Record<string, unknown[]>; base: string; hook_count: number; cleanup: MockHookSuggestion[]; legacy: MockHookSuggestion[]; target_exists: boolean } };
  const hooksElements = () => (routes["hooks:status"].data as { elements: MockHooksElement[] }).elements;
  const hookError = (command: string, message: string): Envelope =>
    makeEnvelope({}, { command, ok: false, errors: [{ code: "HOOK_ERROR", element: null, message }] });
  const countHooks = (hooks: Record<string, unknown[]>): number =>
    Object.values(hooks).reduce((sum, entries) => sum + entries.reduce((n: number, entry) => n + ((entry as { hooks?: unknown[] }).hooks?.length ?? 0), 0), 0);
  const withoutCommand = (hooks: Record<string, unknown[]>, command: string): Record<string, unknown[]> => {
    const out: Record<string, unknown[]> = {};
    for (const [event, entries] of Object.entries(hooks)) {
      const kept = entries
        .map((entry) => {
          const typed = entry as { hooks: { command?: string }[] };
          return { ...typed, hooks: typed.hooks.filter((hook) => hook.command !== command) };
        })
        .filter((entry) => entry.hooks.length > 0);
      if (kept.length > 0) out[event] = kept;
    }
    return out;
  };
  let hooksRevision = 0;
  const previewHooks = (args: string[]): Envelope => {
    const target = hooksElements().find((item) => item.id === argOf(args, "--only"));
    if (!target) return hookError("hooks.preview", "Hooks 目标不存在");
    if (argOf(args, "--base") !== target.observed.base) return hookError("hooks.preview", "目标已在外部修改，请刷新后重试");
    const hooks = JSON.parse(argOf(args, "--value") ?? "{}") as Record<string, unknown[]>;
    const root = target.spec.kind === "claude" ? { theme: "dark", hooks } : { hooks };
    return makeEnvelope({ path: target.observed.target, content: `${JSON.stringify(root, null, 2)}\n`, exists: target.observed.target_exists, will_write: true }, { command: "hooks.preview" });
  };
  const applyHooks = (args: string[], runId: string): Envelope => {
    const target = hooksElements().find((item) => item.id === argOf(args, "--only"));
    if (!target) return hookError("hooks.apply", "参数错误: --only 目标无效");
    if (argOf(args, "--base") !== target.observed.base) {
      return hookError("hooks.apply", `${target.spec.name} 已在外部修改（与打开时读到的内容不同），没有写入；请刷新后重新操作`);
    }
    let next: Record<string, unknown[]> = target.observed.hooks;
    if (args.includes("--value")) {
      next = JSON.parse(argOf(args, "--value") ?? "{}") as Record<string, unknown[]>;
    } else if (args.includes("--cleanup")) {
      const item = target.observed.cleanup.find((c) => c.id === argOf(args, "--cleanup"));
      if (!item) return hookError("hooks.apply", "这条清理建议已不存在，请刷新后重试");
      next = withoutCommand(next, item.command);
      target.observed.cleanup = target.observed.cleanup.filter((c) => c !== item);
    } else if (args.includes("--legacy")) {
      const item = target.observed.legacy.find((c) => c.id === argOf(args, "--legacy"));
      if (!item) return hookError("hooks.apply", "这条旧版 hooks 已不存在或不属于这个目标，请刷新后重试");
      next = { ...next, [item.event]: [...(next[item.event] ?? []), { ...(item.matcher ? { matcher: item.matcher } : {}), hooks: [{ type: "command", command: item.command }] }] };
      target.observed.legacy = target.observed.legacy.filter((c) => c !== item);
    }
    target.observed.hooks = next;
    target.observed.hook_count = countHooks(next);
    target.observed.base = `demo-hooks-base-${++hooksRevision}`;
    target.observed.target_exists = true;
    target.state = "managed-ok";
    recordRun(runId, "hooks.apply");
    return makeEnvelope(
      { domain: "hooks", steps: [{ name: `hooks:write ${target.id}`, ok: true, exit_code: 0 }], skipped: [] },
      { command: "hooks.apply", run_id: runId, lock: { held: true, pid: 4242 } },
    );
  };
  const discardLegacyHook = (args: string[]): Envelope => {
    const id = argOf(args, "--legacy");
    for (const element of hooksElements()) {
      const before = element.observed.legacy.length;
      element.observed.legacy = element.observed.legacy.filter((item) => item.id !== id);
      if (element.observed.legacy.length !== before) {
        return makeEnvelope({ domain: "hooks", discarded: id, source_removed: true }, { command: "hooks.discard-legacy" });
      }
    }
    return hookError("hooks.discard-legacy", "这条旧版 hooks 已不存在（可能已加入或已丢弃），请刷新后重试");
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
  const readConfigFile = (args: string[]): Envelope => {
    const element = argOf(args, "--only") ?? "";
    const file = configFiles[element];
    if (!file) {
      return makeEnvelope({}, { command: "agents.config-file", ok: false, errors: [{ code: "AGENT_ERROR", element, message: "这个助手没有可直接编辑的配置文件" }] });
    }
    return makeEnvelope(
      { domain: "agents", element, ...file, exists: true, blocked_reason: null },
      { command: "agents.config-file" },
    );
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
      exists: true, content,
    }, { command: "agents.preview-runtime" });
  };
  const previewDefault = (args: string[]): Envelope => {
    const element = argOf(args, "--only") ?? "";
    const file = configFiles[element];
    const agents = (routes["agents:status"].data as {
      elements: Array<{ id: string; observed: { current_config?: { base?: string; target?: string } } }>;
    }).elements;
    const observed = agents.find((item) => item.id === element)?.observed.current_config;
    if (!file || !observed || argOf(args, "--base") !== observed.base) {
      return makeEnvelope({}, { command: "agents.preview-default", ok: false,
        errors: [{ code: "AGENT_CONFIG_CHANGED", element, message: "配置文件已在外部修改，请刷新后重新预览" }] });
    }
    let content: string;
    if (file.format === "json") {
      const root = JSON.parse(file.content) as Record<string, unknown>;
      const env = root.env as Record<string, unknown> | undefined;
      if (env) {
        for (const key of ["ANTHROPIC_BASE_URL", "ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_API_KEY",
          "ANTHROPIC_MODEL", "ANTHROPIC_DEFAULT_OPUS_MODEL", "ANTHROPIC_DEFAULT_SONNET_MODEL",
          "ANTHROPIC_DEFAULT_HAIKU_MODEL", "ANTHROPIC_DEFAULT_FABLE_MODEL", "API_TIMEOUT_MS"]) delete env[key];
        if (Object.keys(env).length === 0) delete root.env;
      }
      content = `${JSON.stringify(root, null, 2)}\n`;
    } else {
      content = `${file.content.split("\n").filter((line) => !/^(model_provider|model)\s*=/.test(line)).join("\n").trimEnd()}\n`;
    }
    return makeEnvelope({ domain: "agents", element, path: observed.target ?? file.path,
      exists: true, file_base: "f".repeat(64), will_write: content !== file.content, content }, { command: "agents.preview-default" });
  };
  const editConfigFile = (args: string[], runId: string): Envelope => {
    const element = argOf(args, "--only") ?? "";
    const file = configFiles[element];
    if (!file || argOf(args, "--file-base") !== file.base) {
      return makeEnvelope({}, { command: "agents.edit-config", ok: false, errors: [{ code: "AGENT_CONFIG_CHANGED", element, message: `${file?.path ?? element} 已在外部修改` }] });
    }
    file.content = argOf(args, "--value") ?? file.content;
    file.base = (parseInt(file.base.slice(0, 8), 16) + 1).toString(16).padStart(64, "0");
    recordRun(runId, "agents.edit-config");
    return makeEnvelope({ domain: "agents", element, path: file.path, changed: [], base: file.base }, { command: "agents.edit-config", run_id: runId });
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

    const codex = element.spec.bin === "codex";
    const settings =
      element.spec.settings && typeof element.spec.settings === "object" && !Array.isArray(element.spec.settings)
        ? structuredClone(element.spec.settings as Record<string, unknown>)
        : {};
    const profiles = (settings.profiles && typeof settings.profiles === "object" && !Array.isArray(settings.profiles)
      ? settings.profiles
      : {}) as Record<string, Record<string, unknown>>;
    const liveConfig = (observed.config ?? {}) as Record<string, unknown>;
    const runtime = (liveConfig.runtime ?? {}) as Record<string, unknown>;
    let backfill: Record<string, unknown> | null = null;
    const secretSourceReady = () => {
      const secrets = routes["secrets:status"].data as { elements: Array<{ observed: { source?: { configured?: boolean; ready?: boolean } } }> };
      const source = secrets.elements[0]?.observed.source;
      return Boolean(source?.configured && source.ready);
    };
    /**
     * 切换或保存前回填还没保存的实际配置，与引擎同一套令牌规则：已选密钥文件就写进去，
     * 没选且没带放弃标志就拒绝。返回拒绝信封或 null。
     */
    const saveLive = (): Envelope | null => {
      if (observed.current?.profile || observed.current?.default || args.includes("--discard-live")) return null;
      const credential = liveConfig.credential as { configured?: boolean; kind?: string | null; saved_var?: string | null } | undefined;
      const tokenUnsaved = Boolean(
        credential?.configured && (credential.kind === "token" || credential.kind === "api_key") && !credential.saved_var,
      );
      const discardToken = args.includes("--discard-live-token");
      if (tokenUnsaved && !discardToken && !secretSourceReady()) {
        return refuse(
          "AGENT_LIVE_TOKEN_UNSAVED",
          "当前实际配置里的令牌还没有保存在任何变量里，切换会丢失它：先到“设置 → 密钥”选择外部密钥文件（令牌会写进该文件，库里只存变量名），或确认放弃这个令牌后带 --discard-live-token 重试",
        );
      }
      const name = `imported-${baseSeed}`;
      const tokenVar = tokenUnsaved && !discardToken ? `IMPORTED_${baseSeed}_TOKEN` : null;
      profiles[name] = {
        label: "导入的配置（演示）",
        note: "切换前从助手实际配置自动保存",
        ...(liveConfig.base_url ? { base_url: liveConfig.base_url } : {}),
        ...(liveConfig.model ? { model: liveConfig.model } : {}),
        ...(tokenVar ? { token_var: tokenVar } : {}),
      };
      backfill = {
        name,
        created: true,
        label: "导入的配置（演示）",
        token: !tokenUnsaved ? "none" : discardToken ? "discarded" : "adopted",
        token_var: tokenVar,
      };
      return null;
    };
    const live = (profile: string | null): Record<string, unknown> => {
      const selected = profile ? profiles[profile] : null;
      return {
        provider: codex ? (profile ?? "openai") : "anthropic",
        base_url: selected?.base_url ?? null,
        model: selected?.model ?? null,
        small_fast_model: selected?.small_fast_model ?? null,
        model_aliases: selected?.model_aliases ?? { opus: null, sonnet: null, haiku: null },
        timeout_ms: selected?.timeout_ms ?? null,
        credential: selected
          ? { configured: Boolean(selected.token_var), kind: codex ? "env" : selected.auth ?? "token", variable: codex ? selected.token_var ?? null : null, saved_var: codex ? null : selected.token_var ?? null }
          : { configured: false, kind: null, variable: null, saved_var: null },
        runtime,
      };
    };

    let current = observed.current ?? null;
    const runtimeAt = args.indexOf("--runtime");
    const useAt = args.indexOf("--use");
    if (runtimeAt >= 0) {
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
    } else if (useAt >= 0 || args.includes("--use-default")) {
      const profile = useAt >= 0 ? args[useAt + 1] : null;
      const connectionAt = args.indexOf("--connection");
      if (profile && connectionAt >= 0) profiles[profile] = JSON.parse(args[connectionAt + 1]);
      if (profile && !(profile in profiles)) return refuse("AGENT_ERROR", `找不到接入:${profile}`);
      const refused = saveLive();
      if (refused) return refused;
      if (profile) settings.active = profile;
      else delete settings.active;
      observed.state = "ready";
      observed.config = live(profile);
      current = { profile, default: profile === null, last_written: profile, unverified: [], drifted_from: null };
    } else if (args.includes("--save-current")) {
      const refused = saveLive();
      if (refused) return refused;
      current = backfill
        ? { ...current, profile: (backfill as { name: string }).name, default: false }
        : current;
    }
    settings.profiles = profiles;
    element.spec.settings = settings;
    baseSeed += 1;
    observed.base = mockBase(baseSeed);
    observed.current = current;
    recordRun(runId, "agents.configure");
    return makeEnvelope(
      { domain: "agents", action: "configure", element: agentId, active: settings.active ?? null, current: current?.profile ?? null, backfill, discarded_live: args.includes("--discard-live"), base: observed.base },
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
      if (command === "agents:preview-runtime") return previewRuntime(args);
      if (command === "agents:preview-default") return previewDefault(args);
      if (command === "agents:config-file") return readConfigFile(args);
      if (command === "agents:edit-config") return editConfigFile(args, runId);
      if (command === "agents:auth-status") {
        const only = args[args.indexOf("--only") + 1] ?? "";
        return makeEnvelope(
          { element: only, state: "logged-out", logged_in: false, method: null, third_party: only.startsWith("profiles:"), mixed: false, reason: null },
          { command: "agents.auth-status" },
        );
      }
      if (command === "agents:check") {
        return makeEnvelope(
          { ok: true, detail: "模拟连接正常", latency_ms: 128 },
          { command: "agents.check" },
        );
      }
      if (command === "agents:models") {
        return makeEnvelope(
          { endpoint: "https://api.example.com/v1/models", count: 3, models: ["mock-model-large", "mock-model-fast", "mock-model-mini"] },
          { command: "agents.models" },
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
      if (command === "profiles:seed") {
        const id = (argOf(args, "--only") ?? "").replace(/^profiles:/, "");
        const items = (argOf(args, "--items") ?? "").split(",").filter(Boolean);
        const demo: Record<string, { state: string; detail: string; skipped?: string[] }> = {
          permissions: { state: "copied", detail: "已带上权限规则" },
          statusline: { state: "absent", detail: "默认环境没有设置状态栏" },
          hooks: { state: "copied", detail: "已带上 2 条 hooks" },
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
      if (command === "hooks:apply") return applyHooks(args, runId);
      if (command === "hooks:preview") return previewHooks(args);
      if (command === "hooks:discard-legacy") return discardLegacyHook(args);
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
