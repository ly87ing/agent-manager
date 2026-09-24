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
  description?: string;
  recommended?: boolean;
  danger?: boolean;
  requires_confirmation?: boolean;
  targets?: AgentRuntimeTarget[];
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
  presentation?: "choice_cards" | string;
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
  /** 客户端配置文件的只读脱敏观察值；不含 token/API key 正文。 */
  current_config?: AgentCurrentConfigStatus | null;
  /** 会压过主配置里官方账号的外部设置（shell 配置、进程环境、apiKeyHelper 等）；只有来源与变量名。 */
  credential_overrides?: { source: string; name: string }[];
}

export interface AgentCurrentConfigCredential {
  configured: boolean;
  kind?: string | null;
  variable?: string | null;
  /** Claude：哪个已保存接入的令牌变量解析值与实际令牌一致；没有为 null。不含取值。 */
  saved_var?: string | null;
}

/** 引擎按内容识别出的当前接入。 */
export interface AgentCurrentConnection {
  /** 与实际配置逐项一致的已保存接入；null 表示助手默认账号或还没保存。 */
  profile: string | null;
  /** 实际没有任何接入键：助手默认账号。 */
  default: boolean;
  /** 库里记下的上次写入的接入，只作记录，不代表当前。 */
  last_written?: string | null;
  /** 地址、模型都对上，但令牌变量在本进程取不到值、无法核对的接入。 */
  unverified?: string[];
  /** 还没保存、但与上次写入的那套身份一致：切换时外部改动会保存进它。 */
  drifted_from?: string | null;
}

export interface AgentCurrentConfig {
  provider?: string | null;
  base_url?: string | null;
  model?: string | null;
  small_fast_model?: string | null;
  model_aliases?: {
    opus?: string | null;
    sonnet?: string | null;
    haiku?: string | null;
  } | null;
  timeout_ms?: string | number | null;
  credential?: AgentCurrentConfigCredential | null;
  runtime?: Record<string, unknown>;
  /** Claude：/effort 按模型保存在 modelSettings 里的推理强度（只读展示）。 */
  claude_model_efforts?: Record<string, string> | null;
  /** Claude：旧版本写进 settings.json env 的单次会话覆盖变量（优先于一切设置）。 */
  claude_env_overrides?: Record<string, string> | null;
  /** Codex：会让 Codex 拒绝加载整份配置的写法（固定措辞，不含取值）。 */
  fatal_issues?: string[] | null;
  /** Codex：新写法的权限配置；设了它，沙箱以它为准，sandbox_mode 不再生效。 */
  default_permissions?: string | null;
}

export interface AgentCurrentConfigStatus {
  state: "missing" | "ready" | "invalid" | "unsafe" | "unreadable" | "too-large" | string;
  file: string;
  /** 运行设置保存时使用的真实目标文件。 */
  target?: string;
  config?: AgentCurrentConfig | null;
  /** 写入基准：写入时原样带回；没有（文件读不了）就不能写。 */
  base?: string | null;
  current?: AgentCurrentConnection | null;
}

/** agents:auth-status：官方命令查到的登录状态；不含账号或凭据。 */
export interface OfficialAuthStatus {
  element: string;
  state: "logged-in" | "logged-out" | "unknown";
  logged_in: boolean | null;
  /** claude.ai / console / oauth_token / chatgpt / api_key / other；有登录记录但方式未知时为 null。 */
  method: string | null;
  /** 这个环境用第三方接口或凭据。 */
  third_party: boolean;
  /** 第三方环境里有官方账号登录：两种凭据混在同一个配置目录。 */
  mixed: boolean;
  /** 无法确认时的原因。 */
  reason: string | null;
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

/** 服务 × 助手 单元格状态：enabled / disabled（官方字段停用或已保存未生效的快照）/ invalid（结构无法识别）。 */
export type McpCellState = "enabled" | "disabled" | "invalid";

/** 脱敏后的已建模字段：env/headers 引用显示 ${VAR}，字面量一律 <literal>。 */
export interface McpEntryView {
  command?: string;
  args?: string[];
  env?: Record<string, string>;
  /** 只有 Codex 建模。 */
  startup_timeout_sec?: number;
  url?: string;
  headers?: Record<string, string>;
}

export interface McpVarResolution {
  placeholder: string;
  resolved: boolean;
  /** 取值来自已选择的外部密钥文件。 */
  stored?: boolean;
  kind?: "env" | "header" | string;
}

export interface McpAgentCell {
  state: McpCellState | string;
  /** file = 助手配置文件里的条目；saved = 保存在本工具里的停用快照。 */
  source: "file" | "saved" | string;
  target?: string;
  target_exists?: boolean;
  transport?: "stdio" | "remote";
  entry?: McpEntryView;
  /** 本工具不建模、写入时原样保留的官方字段（非凭据类原样，其余 <hidden>）。 */
  extra?: Record<string, unknown>;
  vars?: Record<string, McpVarResolution>;
  /** 条目摘要：编辑、启停、删除、复制时作为 --base 回传。 */
  base: string;
  legacy_marker?: boolean;
  /** 文件里有条目，同时本工具还保存着一份停用快照。 */
  has_saved?: boolean;
  /** stdio 命令能否在本机解析；远程与快照恒 null。 */
  command_available?: boolean | null;
  notice?: string | null;
  blocked_reason?: string | null;
  reason?: string;
  actions: string[];
}

export interface McpSavedDefinition {
  /** 未分配到任何助手的已保存定义（引用原样、字面量掩码）。 */
  definition: {
    command?: string;
    args?: string[];
    env?: Record<string, string>;
    startup_timeout_sec?: number;
    extra_args_by_agent?: Record<string, string[]>;
    url?: string;
    headers?: Record<string, string>;
  };
  valid: boolean;
}

export interface McpSpec {
  name: string;
  saved: McpSavedDefinition | null;
}

export interface McpObserved {
  agents?: Record<string, McpAgentCell>;
}

/** rc 文件里的一行别名定义。 */
export interface AliasLine {
  name: string;
  /** 本工具写法时的取值；其他写法为 null。 */
  value: string | null;
  line: string;
}

export interface AliasesObserved {
  /** 绝对路径。 */
  target: string;
  target_exists: boolean | null;
  block_present: boolean | null;
  /** 写入基准：区块正文摘要；没有区块为 "missing"；不能读写时为 null。 */
  base: string | null;
  /** 受管区块里的别名。 */
  entries: AliasLine[];
  /** 区块外用户自己的别名：只读，不由本工具写入。 */
  external: AliasLine[];
  blocked_reason: string | null;
}

export interface AliasesSpec {
  name: string;
  target: string;
  /** aliases:apply 里给出这个文件写入基准的参数。 */
  base_flag: string;
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
  /** 来源已取回且正常，却没有这个技能（上游删了）：只能移除这条分配。 */
  source_missing?: boolean;
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
  group: "global-rules" | "profile-rules" | string;
  kind: "claude" | "codex" | string;
  profile_id: string | null;
  /** 展示路径（~/...）。 */
  target: string;
}

export interface RulesObserved {
  /** 绝对路径。 */
  target: string;
  target_exists: boolean | null;
  /** 规则文件实际全文；文件缺失、已停用或不能安全读取时为 null。 */
  content: string | null;
  /** 写入基准：文件字节摘要；文件不存在为 "missing"；已停用或不能读写时为 null。 */
  base: string | null;
  matched_template: { id: string; name: string } | null;
  /** 文件内容与某条来源规则逐字一致。 */
  matched_source_rule: { id: string; name: string; collection: string } | null;
  /** 最后一次从来源规则写入的记录；没有则为 null。 */
  source_follow: RuleSourceFollow | null;
  blocked_reason: string | null;
}

/** 跟随来源：上次写入的是哪条来源规则、文件是否还是那一版、来源是否有新版本。 */
export interface RuleSourceFollow {
  rule: string;
  applied_revision: string;
  /** 来源规则当前版本；来源已停用、删除或没取回时为 null。 */
  current_revision: string | null;
  file_unchanged: boolean;
  update_available: boolean;
}

export type { RuleTemplate, RuleSourceRule, SuggestedSource } from "./contract";

// ---------- hooks ----------

export interface HooksSpec {
  name: string;
  group: "global-hooks" | "profile-hooks" | string;
  kind: "claude" | "codex" | string;
  /** hooks-key：只改 settings.json 的 hooks 键；whole-file：hooks.json 整份。 */
  scope: "hooks-key" | "whole-file" | string;
  profile_id: string | null;
  /** 展示路径（~/...）。 */
  target: string;
}

/** 一条清理建议：实际 hooks 里命中内置旧签名的 hook。 */
export interface HookCleanupSuggestion {
  id: string;
  event: string;
  matcher: string | null;
  command: string;
}

/** 一条旧版本保存、还没生效的 hook。 */
export interface HookLegacySuggestion extends HookCleanupSuggestion {
  source_key: string;
}

export interface HooksObserved {
  /** 绝对路径。 */
  target: string;
  target_exists: boolean | null;
  /** 实际 hooks（事件 → 条目数组）；已停用或不能安全读取时为 null。 */
  hooks: Record<string, unknown> | null;
  hook_count: number;
  /** 写入基准；没有 hooks 为 "missing"；已停用或不能读写时为 null。 */
  base: string | null;
  cleanup: HookCleanupSuggestion[];
  legacy: HookLegacySuggestion[];
  blocked_reason: string | null;
}

// ---------- secrets ----------

export interface SecretsEnvVariable {
  server?: string;
  env_key?: string;
  placeholder?: string;
  resolved?: boolean;
  /** true = 取值来自已选择的外部密钥文件；false = 来自进程环境或还没有。 */
  stored?: boolean;
  /** "env"（stdio 环境变量）或 "header"（远程 HTTP header 占位符）。 */
  kind?: "env" | "header" | string;
}

export interface SecretsMcpEnvObserved {
  variables?: SecretsEnvVariable[];
  unresolved?: number;
  source?: {
    configured?: boolean;
    path?: string | null;
    ready?: boolean;
    error?: string | null;
  };
  legacy_database_values?: number;
}

/** 读取 observed.agents 映射（mcp / skills 的 per-agent 单元格）。 */
export function agentCellsOf(element: { observed: Record<string, unknown> }): Record<string, { state: string; target?: string; link_path?: string; target_exists?: boolean }> {
  const cells = element.observed["agents"];
  if (typeof cells !== "object" || cells === null || Array.isArray(cells)) {
    return {};
  }
  return cells as Record<string, { state: string; target?: string; link_path?: string; target_exists?: boolean }>;
}

/** 官方命令报告的登录方式 → 界面用语；方式未知时为 null。 */
export function authMethodLabel(method: string | null): string | null {
  switch (method) {
    case null:
      return null;
    case "claude.ai":
      return "Claude 订阅账号";
    case "console":
      return "Claude Console 账号";
    case "oauth_token":
      return "长期令牌";
    case "chatgpt":
      return "ChatGPT 账号";
    case "api_key":
      return "API key";
    default:
      return "其他方式";
  }
}
