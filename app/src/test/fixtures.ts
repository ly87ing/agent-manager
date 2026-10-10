import {
  CONTRACT_ID,
  type AliasLegacySuggestion,
  type AliasRecommendation,
  type SuggestedSource,
  type ApplyResultData,
  type AuditData,
  type ElementAction,
  type EngineElement,
  type Envelope,
  type Freshness,
  type Mode,
  type PolicyData,
  type RunEntry,
  type StatusData,
} from "../lib/contract";
import type { AgentManagement, AgentRuntimeField, AgentRuntimeSummary } from "../lib/observed";

export function controlledCommand(element: string, operation = "launch", shell = "posix"): string {
  return `${shell === "powershell" ? "& " : ""}"/isolated/Agent Manager" --account-launch --home "/isolated/home" --data-dir "/isolated/data" --only "${element}" --operation ${operation} --`;
}

export async function accountCommandRoute(args: string[]): Promise<Envelope> {
  const value = (flag: string) => args[args.indexOf(flag) + 1];
  const element = value("--only");
  const operation = value("--operation");
  return makeEnvelope({ controlled: true, launch_value: controlledCommand(element, operation), terminal_command: controlledCommand(element, operation, value("--shell")) }, { command: "accounts.command" });
}

const summary = (label: string, extras: Partial<AgentRuntimeSummary> = {}): AgentRuntimeSummary => ({
  label,
  default: "客户端默认",
  ...extras,
});

/** 与 Rust 适配器的 status schema 对齐；页面测试只消费契约，不再根据 bin 猜能力。 */
export function agentManagementFixture(kind: "claude" | "codex"): AgentManagement {
  const automationMode: AgentRuntimeField = {
    key: "automation_mode",
    type: "enum",
    presentation: "choice_cards",
    section: "权限与执行",
    section_description: kind === "claude"
      ? "Auto 是否可用由 Claude 当前账号、模型与组织设置决定；完全放开会在保存前再次确认。"
      : "建议使用智能自动；完全放开会在保存前再次确认。",
    label: "自动化程度",
    description: "选择助手如何处理需要权限的操作。",
    options: kind === "claude"
      ? [
          { value: "manual", label: "手动确认", description: "由你确认敏感操作。", targets: [{ key: "permissions.defaultMode", enabled_value: '"manual"' }] },
          { value: "auto", label: "智能自动", description: "由 Claude 自动判断；仅在当前账号、模型与组织设置支持时可用。", targets: [{ key: "permissions.defaultMode", enabled_value: '"auto"' }] },
          { value: "full", label: "完全放开", description: "绕过所有权限检查。", danger: true, requires_confirmation: true, targets: [{ key: "permissions.defaultMode", enabled_value: '"bypassPermissions"' }] },
        ]
      : [
          { value: "manual", label: "手动确认", description: "保留沙箱，越界操作由你确认。", targets: [{ key: "approval_policy", enabled_value: '"on-request"' }, { key: "sandbox_mode", enabled_value: '"workspace-write"' }, { key: "approvals_reviewer", enabled_value: '"user"' }] },
          { value: "auto", label: "智能自动", description: "保留沙箱，由审查器自动判断。", recommended: true, targets: [{ key: "approval_policy", enabled_value: '"on-request"' }, { key: "sandbox_mode", enabled_value: '"workspace-write"' }, { key: "approvals_reviewer", enabled_value: '"auto_review"' }] },
          { value: "full", label: "完全放开", description: "不询问且移除沙箱。", danger: true, requires_confirmation: true, targets: [{ key: "approval_policy", enabled_value: '"never"' }, { key: "sandbox_mode", enabled_value: '"danger-full-access"' }] },
        ],
    targets: [],
    summary: summary("自动化", { values: { manual: "手动确认", auto: "智能自动", full: "完全放开" } }),
  };
  const fields: AgentRuntimeField[] = kind === "claude"
    ? [
        automationMode,
        {
          key: "default_model", type: "model", section: "界面与模型行为", label: "默认模型",
          description: "写入 settings.json 顶层 model；env.ANTHROPIC_MODEL 优先。留空撤掉此处的覆盖。",
          placeholder: "例如 claude-sonnet-5", targets: [{ key: "model" }], summary: summary("默认模型"),
        },
        {
          key: "statusline", type: "enum", section: "界面与模型行为", label: "状态栏",
          description: "仅移除本工具旧状态栏；保留自定义配置。",
          options: [{ value: "", label: "移除本工具旧状态栏" }],
          targets: [{ key: "statusLine.command", enabled_value: "<Agent Manager statusline.sh>" }],
          summary: summary("状态栏", { default: "保持现状", values: { builtin: "用量状态栏" } }),
        },
        {
          key: "claude_effort_level", type: "enum", section: "界面与模型行为", label: "默认推理强度",
          description: "写入 settings.json 的 effortLevel，作为没有单独保存级别的模型的默认值；在 Claude Code 里用 /effort 按模型保存的级别优先。留空即撤掉。",
          options: ["low", "medium", "high", "xhigh"].map((value) => ({ value, label: value })),
          targets: [{ key: "effortLevel" }], summary: summary("推理强度"),
        },
        ...[
          ["claude_tool_search", "工具搜索", "env.ENABLE_TOOL_SEARCH", '"true"'],
          ["claude_agent_teams", "Agent Teams", "env.CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS", '"1"'],
          ["claude_disable_autoupdate", "禁止自动更新", "env.DISABLE_AUTOUPDATER", '"1"'],
          ["claude_disable_artifact", "禁用 Artifact", "enableArtifact", "false"],
          ["claude_hide_attribution", "隐藏提交与 PR 署名", "attribution.commit", '""'],
        ].map(([key, label, target, enabled]) => ({
          key, type: "boolean", section: "高级能力", label,
          description: `${label} 开关。`,
          targets: key === "claude_hide_attribution"
            ? [
                { key: "attribution.commit", enabled_value: '""' },
                { key: "attribution.pr", enabled_value: '""' },
              ]
            : [{ key: target, enabled_value: enabled }],
          ...(key === "claude_tool_search" || key === "claude_agent_teams"
            ? { summary: summary(label, { enabled: "开启", default: "默认" }) }
            : {}),
        })),
      ]
    : [
        automationMode,
        {
          key: "default_model", type: "model", section: "模型与上下文", label: "默认模型",
          description: "写入 config.toml 顶层 model；留空撤掉此处的覆盖。",
          placeholder: "例如 gpt-6-sol", targets: [{ key: "model" }], summary: summary("默认模型"),
        },
        {
          key: "codex_reasoning_effort", type: "enum", section: "模型与上下文", label: "推理强度",
          description: "选择 Codex 的模型推理强度。",
          options: ["minimal", "low", "medium", "high", "xhigh"].map((value) => ({ value, label: value })),
          targets: [{ key: "model_reasoning_effort" }], summary: summary("推理强度"),
        },
        {
          key: "codex_web_search", type: "enum", section: "模型与上下文", label: "Web 搜索模式",
          description: "选择 Codex 使用的 Web 搜索策略。",
          options: ["disabled", "cached", "indexed", "live"].map((value) => ({ value, label: value })),
          targets: [{ key: "web_search" }], summary: summary("Web 搜索"),
        },
        ...[
          ["codex_context_window", "上下文窗口（tokens）", "model_context_window", "上下文"],
          ["codex_auto_compact_token_limit", "自动压缩阈值（tokens）", "model_auto_compact_token_limit", "自动压缩"],
        ].map(([key, label, target, summaryLabel]) => ({
          key, type: "positive_integer", section: "模型与上下文", label,
          description: "留空使用 Codex 默认值。", targets: [{ key: target }], summary: summary(summaryLabel),
        })),
      ];
  return {
    adapter: kind,
    schema_version: 1,
    connection_kind: kind,
    supports_profiles: true,
    runtime_fields: fields,
  };
}

export function makeFreshness(overrides: Partial<Freshness> = {}): Freshness {
  return {
    state: "not-applicable",
    current: null,
    upstream: null,
    checked_at: null,
    probe: null,
    cache_age_sec: null,
    ...overrides,
  };
}

export function makeElement(overrides: Partial<EngineElement> = {}): EngineElement {
  return {
    id: "mcp:demo",
    domain: "mcp",
    spec: {},
    policy: {},
    observed: {},
    state: "managed-ok",
    freshness: makeFreshness(),
    actions: [],
    ...overrides,
  };
}

export function makeMode(overrides: Partial<Mode> = {}): Mode {
  return { offline: false, ...overrides };
}

export function makeEnvelope<T>(data: T, overrides: Partial<Envelope<T>> = {}): Envelope<T> {
  return {
    contract: CONTRACT_ID,
    run_id: "20260905T000000Z-test01",
    command: "test.command",
    mode: makeMode(),
    lock: { held: false, pid: null },
    ok: true,
    data,
    warnings: [],
    errors: [],
    ...overrides,
  };
}

export function makeAuditData(overrides: Partial<AuditData> = {}): AuditData {
  return {
    status: "healthy",
    version: { commit: "abc1234", date: "2026-09-05" },
    sections: {
      runtime: [{ name: "engine-rs", status: "ok", message: "状态正常" }],
    },
    summary: {
      errors: 0,
      warnings: 0,
      warning_categories: { optional_tools: 0, unmaterialized: 0, official_gaps: 0 },
    },
    ...overrides,
  };
}

// ---------- Phase 2 命令族 builders ----------

export function makeApplyResult(overrides: Partial<ApplyResultData> = {}): ApplyResultData {
  return {
    domain: "mcp",
    steps: [
      { name: "mcp-sync", ok: true, exit_code: 0 },
    ],
    skipped: [],
    ...overrides,
  };
}

export function makeRunEntry(overrides: Partial<RunEntry> = {}): RunEntry {
  return {
    run_id: "20260906T010000Z-mock01",
    operations: 1,
    last_at: "2026-09-06T01:00:00Z",
    ...overrides,
  };
}

export function makePolicyData(overrides: Partial<PolicyData> = {}): PolicyData {
  return {
    layers: { sqlite: { browser_us_sync: { enabled: false } } },
    effective: {
      agents: { "claude-code": { enabled: true } },
      mcp: { "grok-search": { enabled: true } },
      browser_us_sync: { enabled: false },
    },
    attribution: { agents: "builtin", mcp: "builtin", browser_us_sync: "sqlite" },
    layer: null,
    key: null,
    value: null,
    path: null,
    ...overrides,
  };
}

/** 元素动作：测试大多只关心 id，其余字段给不影响判定的默认值。 */
export function makeAction(id: string, overrides: Partial<ElementAction> = {}): ElementAction {
  return { id, risk: "write-local", needs_lock: true, offline_ok: true, ...overrides };
}

/**
 * aliases:status 的 data：两个 rc 文件的受管区块都写着同样的 `aliases`（本工具写法），
 * 区块外的只读别名、推荐与已保存未生效的建议按需给出。与引擎 aliases 域的输出形状一致。
 */
export function makeAliasStatus(
  aliases: Record<string, string>,
  options: {
    recommendations?: AliasRecommendation[];
    suggestedSources?: SuggestedSource[];
    legacy?: AliasLegacySuggestion[];
    external?: Record<string, string>;
    blockedZprofile?: string;
  } = {},
): StatusData {
  const line = (name: string, value: string) => `alias ${name}='${value}'`;
  const entries = Object.entries(aliases).map(([name, value]) => ({ name, value, line: line(name, value) }));
  const target = (id: string, file: string, flag: string, blocked?: string) =>
    makeElement({
      id,
      domain: "aliases",
      spec: { name: id, target: file, marker: "agent-manager shell aliases", base_flag: flag },
      policy: { enabled: true },
      observed: {
        target: `/Users/demo/${file.slice(2)}`,
        target_exists: !blocked,
        block_present: blocked ? null : entries.length > 0,
        base: blocked ? null : entries.length > 0 ? `base-${id}` : "missing",
        entries: blocked ? [] : entries,
        external:
          blocked || file !== "~/.zshrc"
            ? []
            : Object.entries(options.external ?? {}).map(([name, value]) => ({ name, value, line: line(name, value) })),
        blocked_reason: blocked ?? null,
      },
      state: blocked ? "unmanaged-detected" : entries.length > 0 ? "managed-ok" : "missing",
      actions: blocked ? [] : [makeAction("write")],
    });
  const zprofileBlocked = options.blockedZprofile;
  return {
    domain: "aliases",
    elements: [
      target("aliases:zshrc", "~/.zshrc", "--zshrc-base"),
      target("aliases:zprofile", "~/.zprofile", "--zprofile-base", zprofileBlocked),
    ],
    aliases: entries
      .map((entry) => ({
        ...entry,
        targets: zprofileBlocked ? ["aliases:zshrc"] : ["aliases:zshrc", "aliases:zprofile"],
        consistent: true,
      }))
      .sort((a, b) => a.name.localeCompare(b.name)),
    alias_recommendations: options.recommendations ?? [],
    alias_suggested_sources: options.suggestedSources ?? [],
    alias_legacy: options.legacy ?? [],
  };
}
