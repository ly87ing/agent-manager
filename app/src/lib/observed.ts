/**
 * 各域 spec/policy/observed 的页面级读取类型。
 * 契约层这三层是 free-form object（element.schema.json），这里的接口是
 * 对引擎当前 payload 的防御性镜像：全部字段可选，读取方须容忍缺省。
 */

// ---------- agents ----------

export interface AgentsSpec {
  display_name?: string;
  bin?: string;
  preferred_provider?: string;
  providers?: Record<string, { managed?: boolean }>;
}

export interface AgentsPolicy {
  enabled?: boolean;
}

export interface AgentRuntimeTarget {
  /** 最终写入客户端配置的真实键路径。 */
  key: string;
  /** boolean/特殊枚举启用时展示的字面量。 */
  enabled_value?: string;
}

export interface AgentRuntimeOption {
  value: string;
  label: string;
}

export interface AgentRuntimeSummary {
  label: string;
  enabled?: string;
  default: string;
  values?: Record<string, string>;
}

export interface AgentRuntimeField {
  key: string;
  type: "boolean" | "enum" | "positive_integer" | string;
  section: string;
  section_description?: string;
  label: string;
  description: string;
  placeholder?: string;
  danger?: boolean;
  requires_yolo_confirmation?: boolean;
  options?: AgentRuntimeOption[];
  targets: AgentRuntimeTarget[];
  summary?: AgentRuntimeSummary;
}

/** Rust 适配器随 agents:status 返回的可编辑能力；未知助手为 null。 */
export interface AgentManagement {
  adapter: string;
  schema_version: number;
  connection_kind: "claude" | "codex" | string;
  supports_profiles: boolean;
  runtime_fields: AgentRuntimeField[];
}

export interface AgentsObserved {
  /** 实际检测到的安装来源（native / brew / system…）；未安装为 null。 */
  install_source?: string | null;
  installed?: boolean;
  version?: string | null;
  management?: AgentManagement | null;
  /** 助手自己写在本机的额度记账；没有这类记录的助手为 null。 */
  usage?: AgentUsage | null;
  /** 官方认证文件的脱敏状态；不含账号或凭据正文。 */
  auth?: AgentAuthStatus | null;
  /** 客户端配置文件的只读脱敏观察值；不含 token/API key 正文。 */
  current_config?: AgentCurrentConfigStatus | null;
}

export interface AgentCurrentConfigCredential {
  configured: boolean;
  kind?: string | null;
  variable?: string | null;
}

export interface AgentCurrentConfig {
  provider?: string | null;
  base_url?: string | null;
  model?: string | null;
  small_fast_model?: string | null;
  credential?: AgentCurrentConfigCredential | null;
  runtime?: Record<string, unknown>;
}

export interface AgentCurrentConfigStatus {
  state: "missing" | "ready" | "invalid" | "unsafe" | "unreadable" | "too-large" | string;
  file: string;
  config?: AgentCurrentConfig | null;
}

export interface AgentAuthStatus {
  state: "missing" | "ready" | "unrecognized" | "invalid" | "unsafe" | "unreadable" | "too-large" | string;
  authenticated: boolean;
  mode?: string | null;
  storage: "file" | string;
  file: string;
}

/** 一个计量窗口：window_minutes 是窗口长度（300=5 小时，10080=7 天）。 */
export interface AgentUsageWindow {
  window_minutes: number;
  used_percent: number;
  resets_at: string | null;
}

export interface AgentUsage {
  /** 读的是哪个本地文件，用来说明数字从哪来。 */
  source: string;
  /** 助手最后一次记账的时刻；数字只和它一样新。 */
  checked_at: string | null;
  windows: AgentUsageWindow[];
  plan?: string;
  credits?: string;
}

// ---------- mcp ----------

/** server × agent 单元格词汇表（引擎 _mcp_cell_state）。 */
export type McpCellState =
  | "managed-ok"
  | "drifted"
  | "missing"
  | "not-materialized"
  | "leftover"
  | "agent-disabled"
  | "probe-error"
  | "absent";

export interface McpAgentCell {
  state: McpCellState | string;
  target?: string;
  target_exists?: boolean;
}

export interface McpSpec {
  command?: string;
  args?: string[];
  startup_timeout_sec?: number;
  agents?: string[] | null;
  env?: Record<string, string>;
  extra_args_by_agent?: Record<string, string[]>;
  /** 远程 HTTP 定义：含 url 即远程（与 command 互斥）；type 为历史遗留输入，不持久化。 */
  url?: string;
  headers?: Record<string, string>;
  type?: string;
}

export interface McpEnvResolution {
  placeholder?: string;
  resolved?: boolean;
}

export interface McpObserved {
  agents?: Record<string, McpAgentCell>;
  env?: Record<string, McpEnvResolution>;
  /** 远程 headers 占位符解析块（与 env 同级：只输出占位符与布尔）。 */
  headers?: Record<string, McpEnvResolution>;
  /** stdio 命令能否在本机解析；远程服务没有本地命令，恒 null。 */
  command_available?: boolean | null;
  blocked_reason?: string | null;
}

/** 一条快捷命令在终端配置里的实际落地情况。 */
export interface AliasEntry {
  name: string;
  /** 同步后应有的那一行。 */
  expected: string;
  /** 终端配置里现在的那一行；还没写入时为 null。 */
  actual: string | null;
  /** applied=已生效 outdated=配置里是旧的 missing=区块里没有 not-materialized=还没写过 */
  state: "applied" | "outdated" | "missing" | "not-materialized" | string;
}

export interface AliasesObserved {
  target?: string;
  entries?: AliasEntry[];
  blocked_reason?: string | null;
}

// ---------- skills ----------

/** skill × agent 单元格词汇表（引擎 _skill_cell_state）。 */
export type SkillCellState = "linked" | "missing-link" | "dead-link" | "drifted" | "extra-link" | "disabled";

export interface SkillAgentCell {
  state: SkillCellState | string;
  link_path?: string;
  target_dir_exists?: boolean;
  assigned?: boolean;
  enabled?: boolean;
  /** 链接是否指向本工具的受管基目录内；false 即外部链接（接管需显式确认）。 */
  managed?: boolean;
  version?: string | null;
  available_version?: string | null;
  actions?: string[];
  blocked_reason?: string | null;
  target?: string | null;
}

export type SkillAgentObserved = SkillAgentCell;

export interface SkillCollectionSpec {
  url?: string | null;
  /** 本机目录来源（与 url 互斥）：链接直接指向该目录，不做版本快照。 */
  source_path?: string | null;
  skill_paths?: string[];
  agents?: string[] | null;
  matrix_mode?: boolean;
  new_skill_default?: string;
}

/** 集合发现失败的固定语义块（引擎 skills:status observed.discovery_error）。 */
export interface SkillDiscoveryError {
  /** 固定词汇表：missing-dir / out-of-bounds / invalid-path / non-string-item / empty-list / invalid-shape / discovery-failed。 */
  category?: string;
  skill_paths_item?: string | null;
  message?: string;
}

export interface SkillCollectionObserved {
  repo_cache?: string;
  repo_cached?: boolean;
  owner_marked?: boolean;
  discovered?: string[];
  discovery_error?: SkillDiscoveryError | null;
  discovery_inbox?: string[];
  extra_links?: string[];
  actions?: string[];
  blocked_reason?: string | null;
}

export interface SkillSpec {
  collection?: string;
  name?: string;
  source_path?: string;
}

export interface SkillPolicy {
  enabled?: boolean;
  agents?: Record<string, boolean>;
}

export interface SkillObserved {
  agents?: Record<string, SkillAgentCell>;
  discovery_inbox?: boolean;
}

// ---------- rules ----------

export interface RulesSpec {
  name: string;
  group: string;
  source_keys: string[];
  target: string;
}

export interface RulesObserved {
  target: string;
  target_exists: boolean | null;
  expected_digest: string;
  applied_digest: string | null;
  shared_revision: string;
  adapter_revision: string;
  blocked_reason: string | null;
  registration_needed: boolean;
}

// ---------- secrets ----------

export interface SecretsEnvVariable {
  server?: string;
  env_key?: string;
  placeholder?: string;
  resolved?: boolean;
  /** true = 取值保存在本机库里（在这里填过）；false = 取值来自进程环境或还没有。 */
  stored?: boolean;
  /** "env"（stdio 环境变量）或 "header"（远程 HTTP header 占位符）。 */
  kind?: "env" | "header" | string;
}

export interface SecretsMcpEnvObserved {
  variables?: SecretsEnvVariable[];
  unresolved?: number;
}

/** 读取 observed.agents 映射（mcp / skills 的 per-agent 单元格）。 */
export function agentCellsOf(element: { observed: Record<string, unknown> }): Record<string, { state: string; target?: string; link_path?: string; target_exists?: boolean }> {
  const cells = element.observed["agents"];
  if (typeof cells !== "object" || cells === null || Array.isArray(cells)) {
    return {};
  }
  return cells as Record<string, { state: string; target?: string; link_path?: string; target_exists?: boolean }>;
}
