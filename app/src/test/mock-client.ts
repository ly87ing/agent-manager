import type { Envelope, RunEntry } from "../lib/contract";
import { EngineBridgeError, type EngineClient } from "../lib/engine";
import { createLaunchMock } from "./launch-mock";
import { makeAuditData, makeElement, makeEnvelope, makeFreshness, makeRunEntry } from "./fixtures";

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
          description: "提供代码审查、方案设计、改动验证、文档写作等通用技能。推荐规则可在规则页预览后单独应用。",
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

  return {
    "system:audit": audit,
    "resources:targets": makeEnvelope({ domain: "resources", targets: [
      { id: "claude-code", name: "Claude", group: "claude-code", kind: "claude", skills_dir: "~/.claude/skills", mcp_path: "~/.claude.json" },
      { id: "codex", name: "Codex", group: "codex", kind: "codex", skills_dir: "~/.codex/skills", mcp_path: "~/.codex/config.toml" },
      { id: "kimi-code", name: "Kimi Code", group: "kimi-code", kind: null },
    ] }, { command: "resources.targets" }),
    "mcp:status": mcpStatus,
    "skills:status": skillsStatus,
    "rules:status": rulesStatus,
    "secrets:status": secretsStatus,
    "mcp:check": makeEnvelope({
      domain: "mcp", action: "check", server: "demo", agent: "claude-code", ok: true, stage: "done",
      message: "连接成功，提供 3 个工具", server_info: { name: "demo-mcp", version: "1.4.0" },
      tool_count: 3, tools: ["search", "fetch", "summarize"], detail: null, elapsed_ms: 2310,
    }, { command: "mcp.check" }),
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
          { id: "mcp:chrome-devtools", probe: "npm-view", state: "behind", current: "1.2.0", upstream: "1.8.0", error: null },
          { id: "mcp:example-search", probe: "git-ls-remote", state: "unknown", current: null, upstream: "8f0ae3c7f25ec419631ade7e431fa2531503433e", error: "uvx-local-unavailable" },
          { id: "mcp:my-proxy", probe: "npm-view", state: "current", current: "0.3.1", upstream: "0.3.1", error: null },
          { id: "skill:agent-skills", probe: "git-ls-remote", state: "behind", current: "a1b2c3d4e5f60718293a4b5c6d7e8f9012a3b4c5", upstream: "f0e1d2c3b4a5968778695a4b3c2d1e0f9a8b7c6d", error: null },
        ],
        summary: { probed: 4, current: 1, behind: 2, ahead: 0, unknown: 1 },
      },
      { command: "system.refresh-upstream" },
    ),
    // 接管发现：覆盖外部目录、已定义集合的计划外链接、原生真实目录和断链。
    "system:discover": makeEnvelope(
      {
        domain: "discover",
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
        summary: { skills_symlink: 3, skills_collection: 1, skills_native: 1, skills_cleanup: 1 },
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
    mcp: "mcp-sync",
    skills: "skills-link",
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

  let baseSeed = 100;

  /** Browser mock hook for toggling external MCP secret-file availability. */
  if (typeof window !== "undefined") {
    (window as unknown as { __agentManagerMock?: unknown }).__agentManagerMock = {
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

  const launch = createLaunchMock();
  return {
    async run(command, args = [], runOptions = {}) {
      // 模拟进程耗时，让加载/进行中的 UI 状态可见
      await new Promise((resolve) => setTimeout(resolve, 120));
      const runId = runOptions.runId ?? "20260906T020000Z-mockrun";
      if (command.startsWith("launch:")) return launch(command, args);

      const domain = command.replaceAll(".", ":").split(":")[0];
      const key = argOf(args, "--key") ?? "";
      if (["agents", "accounts", "profiles", "profile", "aliases", "usage"].includes(domain)
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
      if (command === "config:export") {
        return makeEnvelope(
          {
            format: "agent-manager/config-backup/v1",
            base: "a".repeat(64),
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
      if (command === "rules:apply") return applyRules(args, runId);
      if (command === "rules:save-template") return saveRuleTemplate(args);
      if (command === "rules:delete-template") return deleteRuleTemplate(args);
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
