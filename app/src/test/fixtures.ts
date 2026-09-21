import {
  CONTRACT_ID,
  type ApplyResultData,
  type AuditData,
  type ElementAction,
  type EngineElement,
  type Envelope,
  type Freshness,
  type Mode,
  type PolicyData,
  type RunEntry,
} from "../lib/contract";
import type { AgentManagement, AgentRuntimeField, AgentRuntimeSummary } from "../lib/observed";

const summary = (label: string, extras: Partial<AgentRuntimeSummary> = {}): AgentRuntimeSummary => ({
  label,
  default: "客户端默认",
  ...extras,
});

/** 与 Rust 适配器的 status schema 对齐；页面测试只消费契约，不再根据 bin 猜能力。 */
export function agentManagementFixture(kind: "claude" | "codex"): AgentManagement {
  const yolo: AgentRuntimeField = {
    key: "yolo",
    type: "boolean",
    section: "权限与执行",
    section_description: "高风险设置仍会在保存后要求二次确认。",
    label: "跳过权限确认",
    description: "开启后执行命令和修改文件不再逐次询问。",
    danger: true,
    requires_yolo_confirmation: true,
    targets: kind === "claude"
      ? [{ key: "permissions.defaultMode", enabled_value: '"bypassPermissions"' }]
      : [
          { key: "approval_policy", enabled_value: '"never"' },
          { key: "sandbox_mode", enabled_value: '"danger-full-access"' },
        ],
    summary: summary("权限确认", { enabled: "已跳过" }),
  };
  const fields: AgentRuntimeField[] = kind === "claude"
    ? [
        yolo,
        {
          key: "statusline", type: "enum", section: "界面与模型行为", label: "状态栏",
          description: "选择是否使用本地用量状态栏。",
          options: [{ value: "", label: "保持现有状态栏" }, { value: "builtin", label: "使用 Agent Manager 用量状态栏" }],
          targets: [{ key: "statusLine.command", enabled_value: "<Agent Manager statusline.sh>" }],
          summary: summary("状态栏", { default: "保持现状", values: { builtin: "用量状态栏" } }),
        },
        {
          key: "claude_effort_level", type: "enum", section: "界面与模型行为", label: "推理强度",
          description: "留空即恢复 Claude 自己的默认值。",
          options: ["auto", "low", "medium", "high", "xhigh", "max"].map((value) => ({ value, label: value })),
          targets: [{ key: "env.CLAUDE_CODE_EFFORT_LEVEL" }], summary: summary("推理强度"),
        },
        ...[
          ["claude_tool_search", "工具搜索", "env.ENABLE_TOOL_SEARCH", '"true"'],
          ["claude_agent_teams", "Agent Teams", "env.CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS", '"1"'],
          ["claude_disable_artifact", "禁用 Artifact", "env.CLAUDE_CODE_DISABLE_ARTIFACT", '"1"'],
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
        yolo,
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
