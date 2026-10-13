

// ---------- mcp ----------

/** 服务 × 助手 单元格状态：enabled / disabled（官方字段停用或已保存未生效的快照）/ invalid（结构无法识别）。 */
export type McpCellState = "enabled" | "disabled" | "invalid";

/**
 * 脱敏后的已建模字段：env/headers 引用显示 ${VAR}；env 里不像凭据的字面量照原值显示；
 * 像凭据的 env 字面量与全部 header 字面量为 <literal>（提示见单元格的 literal_hints）。
 */
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
  /** 显示成 <literal> 的原值各自的提示（像凭据的只露头尾，如 sk-…6789；普通请求头给完整值）。 */
  literal_hints?: { env?: Record<string, string>; headers?: Record<string, string> };
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
  /** 引擎已确认跨账号复制不会带入凭据；缺少此字段时不可自动补齐。 */
  copy_safe?: boolean;
  copy_blocked_reason?: string | null;
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

// ---------- skills ----------

/** skill × agent 单元格词汇表（引擎 _skill_cell_state）。 */
export type SkillCellState = "linked" | "missing-link" | "dead-link" | "drifted" | "extra-link" | "other-source" | "disabled";

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
  /** git 来源的跟踪方式；缺省（null）为自动：有版本号标签跟最新版本，否则跟主分支。 */
  track?: import("./skillTrack").TrackSpec | null;
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
  /** 来源里全部技能（不受 skill_paths 限制）：名字与相对来源根的路径。 */
  available?: { name: string; path: string }[];
  discovery_error?: SkillDiscoveryError | null;
  discovery_inbox?: string[];
  extra_links?: string[];
  actions?: string[];
  blocked_reason?: string | null;
  /** git 来源：镜像当前所在版本；跟踪方式写错时的原因。本机目录来源为 null。 */
  version?: {
    current: import("./skillTrack").VersionRef | null;
    track_error: string | null;
  } | null;
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
  /** 「改成我的版本」时记下的来历：来自哪个集合的哪个版本、上游现在是否有新版。 */
  forked_from?: { collection: string; version: string; upstream_version: string | null; upstream_changed: boolean } | null;
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
    /** 文件里已有的变量名（只有名字，没有取值）。 */
    names?: string[];
  };
  legacy_database_values?: number;
  /** 接入配置与独立环境引用的凭据变量：用在哪里、有没有取值；不含取值。 */
  provider_keys?: ProviderKeyVariable[];
}

export interface ProviderKeyVariable {
  variable: string;
  resolved: boolean;
  stored: boolean;
  used_by: (
    | { kind: "connection"; agent: string; name: string; label: string }
    | { kind: "profile"; id: string; label: string }
  )[];
}

/** 读取 observed.agents 映射（mcp / skills 的 per-agent 单元格）。 */
export function agentCellsOf(element: { observed: Record<string, unknown> }): Record<string, { state: string; target?: string; link_path?: string; target_exists?: boolean }> {
  const cells = element.observed["agents"];
  if (typeof cells !== "object" || cells === null || Array.isArray(cells)) {
    return {};
  }
  return cells as Record<string, { state: string; target?: string; link_path?: string; target_exists?: boolean }>;
}
