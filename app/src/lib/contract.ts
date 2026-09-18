/**
 * Contract v1（contract/engine-v1/*.schema.json）的 TypeScript 镜像。
 * 字段命名与 schema 完全一致（snake_case），可空字段遵循 schema 的
 * 「required but nullable」约定：键恒在，值为 null。
 */

export const CONTRACT_ID = "agent-manager/engine/v1";

// ---------- envelope ----------

export interface Mode {
  offline: boolean;
}

export interface LockInfo {
  held: boolean;
  pid: number | null;
}

export interface Diagnostic {
  code: string;
  element: string | null;
  message: string;
}

export interface Envelope<T = unknown> {
  contract: string;
  run_id: string;
  command: string;
  mode: Mode;
  lock: LockInfo;
  log_file: string | null;
  ok: boolean;
  data: T;
  warnings: Diagnostic[];
  errors: Diagnostic[];
}

export interface SettingsPreferences {
  offline: boolean;
  upstreamTtlSec: number;
  logRetentionDays: number;
}

export interface SettingsData {
  preferences: SettingsPreferences;
  appVersion: string;
  contractVersion: string;
  resourceDir: string;
  dataDir: string;
  logDir: string;
  updaterConfigured: boolean;
  maintenanceWarning?: string | null;
}

// ---------- element（双轴模型：收敛轴 state + 上游轴 freshness） ----------

export type ElementState =
  | "managed-ok"
  | "drifted"
  | "missing"
  | "not-materialized"
  | "unmanaged-detected"
  | "unsupported"
  | "disabled";

export type FreshnessState = "current" | "behind" | "ahead" | "unknown" | "not-applicable";

export type RiskLevel = "read" | "write-local" | "exec-local" | "network" | "install-remove";

export interface Freshness {
  state: FreshnessState;
  current: string | null;
  upstream: string | null;
  checked_at: string | null;
  probe: string | null;
  cache_age_sec: number | null;
}

export interface ElementAction {
  id: string;
  risk: RiskLevel;
  needs_lock: boolean;
  offline_ok: boolean;
}

export interface EngineElement {
  id: string;
  domain: string;
  spec: Record<string, unknown>;
  policy: Record<string, unknown>;
  observed: Record<string, unknown>;
  state: ElementState;
  freshness: Freshness;
  actions: ElementAction[];
}

// ---------- <domain>:status ----------

export interface StatusData {
  domain: string;
  elements: EngineElement[];
  /** 仅 mcp 域输出：各助手现有配置中未受管的导入候选（掩码形态，绝无明文）。 */
  import_candidates?: McpImportCandidate[];
}

// ---------- mcp:status import_candidates ----------

export interface McpImportCandidateSpec {
  /** stdio 形态 */
  command?: string;
  args?: string[];
  /** 键为变量名，值恒为 "<literal>"（掩码）。 */
  env?: Record<string, string>;
  /** remote 形态 */
  url?: string;
  /** 值为 "<literal>"（掩码）或 "${VAR}"（Codex env_http_headers 引用直通）。 */
  headers?: Record<string, string>;
}

export interface McpImportCandidate {
  name: string;
  agent: string;
  transport: "stdio" | "remote" | string;
  spec: McpImportCandidateSpec;
  /** true = 条目带旧体系写入的管理标记但本工具无绑定记录（此前由本工具管理）。 */
  previously_managed?: boolean;
}

// ---------- profiles（助手实例） ----------

/** 实例类型：决定配置目录变量与目标文件格式。 */
export type ProfileKind = "claude" | "codex";

export interface ProfileSpec {
  display_name: string;
  base_agent: string;
  kind: ProfileKind | string;
  config_dir: string;
  base_url: string | null;
  model: string | null;
}

export interface ProfileEnvRef {
  name: string;
  kind: "reference" | "literal" | string;
  /** 引用形式时的变量名；直接写值时为 null。 */
  variable: string | null;
  /** 当前环境里是否给得出取值；直接写值时为 null。 */
  resolved: boolean | null;
}

export interface ProfileObserved {
  base_agent: string;
  kind: ProfileKind | string;
  /** 启动时指向独立配置目录的环境变量名（CLAUDE_CONFIG_DIR / CODEX_HOME）。 */
  config_env: string;
  config_dir: string;
  config_dir_exists: boolean;
  target: string;
  target_exists: boolean;
  env_refs: ProfileEnvRef[];
  /** 建议的启动命令取值，可直接存成同名快捷命令。 */
  launch_value: string;
  blocked_reason: string | null;
}

// ---------- system:discover（只读接管发现） ----------

/** 接管方式词汇表（引擎侧 manage 字段）：
 * native = 官方路径/原生安装方案，只标识不接管；symlink = 外部资源以软链接纳管；
 * collection = 走已定义集合的接管通道；register = 登记定义（助手 CLI 与 MCP 条目）；
 * cleanup = 断链或旧体系残留，只建议清理。 */
export type TakeoverManage = "native" | "symlink" | "collection" | "register" | "cleanup";

export interface DiscoverAgentCandidate {
  id: string;
  display_name: string;
  bin: string;
  bin_path: string;
  version: string | null;
  origin: "npm" | "brew" | "native" | "system" | string;
  npm_package: string | null;
  manage: "register";
  /** true = 登记后可由本工具升级/卸载（仅可靠反查到包名的 npm 渠道）。 */
  managed_after_takeover: boolean;
  suggested_definition: Record<string, unknown>;
}

export interface DiscoverSkillItem {
  agent: string;
  name: string;
  link_path: string;
  target: string | null;
  origin: "native-dir" | "external-link" | "collection-extra-link" | "legacy-link" | "dead-link" | string;
  manage: TakeoverManage;
  /** 命中已定义集合时的集合名（collection-extra-link）。 */
  collection: string | null;
  /** 参与聚合的源根（external-link）。 */
  group: string | null;
  suggested_collection?: string;
  has_skill_md: boolean;
}

export interface DiscoverSkillGroup {
  name: string;
  source_path: string;
  manage: "symlink";
  /** true = 该源根已登记为本地来源集合，接管直接落到既有集合。 */
  existing_collection: boolean;
  agents: string[];
  skills: { name: string; agents: string[]; has_skill_md: boolean }[];
  suggested_definition: { source_path: string; skill_paths: string[]; agents: string[] };
}

export interface DiscoverMcpCandidate extends McpImportCandidate {
  origin: "unmanaged" | "previously-managed" | string;
  manage: "register";
}

export interface DiscoverData {
  domain: "discover";
  agents?: DiscoverAgentCandidate[];
  skills?: { items: DiscoverSkillItem[]; groups: DiscoverSkillGroup[] };
  mcp?: DiscoverMcpCandidate[];
  summary: Record<string, number>;
}

// ---------- system:audit（data 沿用 doctor 报告结构，契约 v2 才正式固化） ----------

export type AuditEntryStatus = "ok" | "notice" | "warning" | "error";

export interface AuditEntry {
  name: string;
  status: AuditEntryStatus;
  message: string;
  category?: string;
}

export type AuditStatus = "healthy" | "warning" | "error";

export interface AuditSummary {
  errors: number;
  warnings: number;
  warning_categories: {
    optional_tools: number;
    unmaterialized: number;
    official_gaps: number;
  };
}

export interface AuditData {
  status: AuditStatus;
  version: {
    commit: string;
    date: string;
  } | null;
  sections: Record<string, AuditEntry[]>;
  summary: AuditSummary;
}

// ---------- system:refresh-upstream ----------
export type ProbeState = Exclude<FreshnessState, "not-applicable">;

export interface RefreshUpstreamEntry {
  id: string;
  probe: string | null;
  state: ProbeState;
  current: string | null;
  upstream: string | null;
  error: string | null;
}

export interface RefreshUpstreamSummary {
  probed: number;
  current: number;
  behind: number;
  ahead: number;
  unknown: number;
}

export interface RefreshUpstreamData {
  domain: "system";
  verb: "refresh-upstream";
  /** 探测缓存落 SQLite upstream_probes 表后不再有缓存文件路径，恒 null。 */
  cache_file: string | null;
  elements: RefreshUpstreamEntry[];
  summary: RefreshUpstreamSummary;
}

// ---------- <domain>:apply（apply-result schema） ----------

export interface ApplyStep {
  name: string;
  ok: boolean;
  exit_code: number;
}

export interface ApplyResultData {
  domain: string;
  steps: ApplyStep[];
  skipped: string[];
}

// ---------- runs:list / runs:get ----------

export interface RunEntry {
  run_id: string;
  operations: number;
  last_at: string;
}

export interface RunOperation {
  id: string;
  operation_type: string;
  target: string | null;
  detail: unknown;
  created_at: string;
}

export interface RunsListData {
  runs: RunEntry[];
}

export interface RunDetailData {
  run_id: string;
  operations: RunOperation[];
}

export interface RunsClearData {
  domain: "runs";
  deleted: number;
}

export interface LogsClearData {
  domain: "logs";
  deleted: number;
}

// ---------- policy:get / policy:set ----------

export type PolicyLayer = "local";
export type PolicyLayerName = "builtin" | "sqlite";

export interface PolicyLayers {
  sqlite: Record<string, unknown> | null;
}

/** 一个 schema 同时服务 get/set，不适用的字段 nullable-required。 */
export interface PolicyData {
  layers: PolicyLayers | null;
  effective: Record<string, unknown> | null;
  attribution: Record<string, PolicyLayerName> | null;
  layer: PolicyLayerName | null;
  key: string | null;
  value: unknown;
  path: string | null;
}
