import type { AgentUsage } from "./observed";
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
  ok: boolean;
  data: T;
  warnings: Diagnostic[];
  errors: Diagnostic[];
}

export interface SettingsPreferences {
  logRetentionDays: number;
}

export interface SettingsData {
  preferences: SettingsPreferences;
  appVersion: string;
  contractVersion: string;
  resourceDir: string;
  dataDir: string;
  updaterConfigured: boolean;
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
  /** 仅 mcp 域输出：各助手的 MCP 配置文件（未配置 mcp_config 的助手不在其中）。 */
  targets?: McpTarget[];
  /** 仅 skills 域：编译内置的推荐技能来源。不是元素，不计入待同步。 */
  recommendations?: SkillRecommendation[];
  /** 仅 skills 域：各助手技能目录里的实际条目（当前视图，只展示）。 */
  installed?: SkillInstalledEntry[];
  /** 仅 skills 域：本工具没管理的技能（其他来源）、Codex 共用目录与重名。只读。 */
  unmanaged?: SkillUnmanagedView;
  /** 仅 rules 域：本机库里已保存的规则模板（含旧版来源即时推导出的模板）。 */
  templates?: RuleTemplate[];
  /** 仅 rules 域：技能来源里的规则模板（集合根下 rules/*.md，README 除外）。只读，不入库。 */
  source_rules?: RuleSourceRule[];
  /** 仅 rules 域：没有来源规则时的推荐来源；collection 为已登记的同名/同地址集合。 */
  suggested_sources?: SuggestedSource[];
  /** 仅 aliases 域：两个 rc 文件受管区块里的实际别名，按名合并。这就是当前配置。 */
  aliases?: AliasCurrent[];
  /** 仅 aliases 域：来源推荐别名（技能来源里的 aliases/*.json；任一文件里已有同名的不推荐）。只读，不计入待同步。 */
  alias_recommendations?: AliasRecommendation[];
  /** 仅 aliases 域：没有来源别名时的推荐来源；collection 为已登记的同名/同地址集合。 */
  alias_suggested_sources?: SuggestedSource[];
  /** 仅 aliases 域：旧版本保存、实际区块里还没有的别名（已保存未生效）。 */
  alias_legacy?: AliasLegacySuggestion[];
}

// ---------- aliases:status 顶层 ----------

export interface AliasCurrent {
  name: string;
  /** 本工具写法 alias name='value' 时的取值；其他写法为 null（只能整条改写或删除）。 */
  value: string | null;
  line: string;
  /** 区块里定义了它的文件（元素 id）。 */
  targets: string[];
  /** false = 两个文件里的写法不一样。 */
  consistent: boolean;
}

export interface AliasRecommendation {
  name: string;
  value: string;
  line: string;
  /** 提供它的技能来源（集合名）。 */
  source: string;
}

export interface AliasLegacySuggestion {
  id: string;
  name: string;
  value: string;
  line: string;
  /** 区块里现在定义这个名字的那一行；没有为 null。 */
  actual: string | null;
}

// ---------- rules:status templates / recommended ----------

export type RuleTemplateOrigin = "user" | "backup" | "legacy";

export interface RuleTemplate {
  id: string;
  name: string;
  body: string;
  revision: string;
  origin: RuleTemplateOrigin;
  created_at: string;
  updated_at: string;
}

/** 一条来源规则：id 为 `<集合>/<名字>`，传给 rules:apply --source-rule。 */
export interface RuleSourceRule {
  id: string;
  collection: string;
  name: string;
  body: string;
  /** 正文 sha256：来源规则的版本。 */
  revision: string;
  /** 本地目录来源（改了即时生效）；否则是 git 镜像，靠「更新资源库」取新版本。 */
  local: boolean;
}

export interface SuggestedSource {
  name: string;
  description: string;
  url: string;
  /** 已登记的同名/同地址集合；null 表示还没添加。 */
  collection: string | null;
}

// ---------- skills:status recommendations / installed ----------

export interface SkillRecommendation {
  name: string;
  description: string;
  url: string;
  skill_paths: string[];
  /** 已有同名或同地址的集合：不再提供添加，避免覆盖用户已有的同名集合。 */
  added: boolean;
  /** 添加时保存的集合定义：只含 url 与 skill_paths，不限助手、不做任何分配。 */
  definition: Record<string, unknown>;
}

/** managed = 本工具写的链接；native-dir = 真实目录；external-link / collection-extra-link = 指向外部路径的链接；
 * legacy-link = 旧版本留下、没有分配记录的链接；dead-link = 目标已不存在的链接。 */
export type SkillInstalledOrigin =
  | "managed"
  | "native-dir"
  | "external-link"
  | "collection-extra-link"
  | "legacy-link"
  | "dead-link";

export interface SkillInstalledEntry {
  agent: string;
  name: string;
  link_path: string;
  target: string | null;
  origin: SkillInstalledOrigin | string;
  /** 本工具链接所属集合，或同名的已定义集合（collection-extra-link）。 */
  collection: string | null;
}

/** 本工具没管理的一个技能名：在哪些账号目录或共用目录里、识别出的来源、能不能纳入。 */
export interface SkillUnmanaged {
  name: string;
  locations: {
    /** 账号（助手 id 或实例 id）；共用目录里的为 null。 */
    agent: string | null;
    /** 共用目录 id（codex-shared）；账号目录里的为 null。 */
    shared: string | null;
    path: string;
    target: string | null;
    origin: "native-dir" | "external-link";
  }[];
  source: {
    repository: string | null;
    path: string | null;
    version: string | null;
    installer: "npx-skills" | null;
    evidence: ("skill-lock" | "skill-release" | "git")[];
  } | null;
  adoptable: boolean;
  blocked_reason: "claude-synced" | "no-skill-md" | null;
}

export interface SkillUnmanagedView {
  skills: SkillUnmanaged[];
  /** 对多个账号生效的共用目录；目前只有 Codex 的 ~/.agents/skills。 */
  shared_dirs: { id: string; kind: "codex"; path: string; exists: boolean }[];
  /** 同一个技能名既在某个 Codex 账号目录里、又在共用目录里：Codex 会加载两份。 */
  duplicates: { name: string; agent: string; paths: string[] }[];
}

// ---------- mcp:status targets ----------

export interface McpTarget {
  agent: string;
  path: string;
  format: "claude-json" | "codex-toml" | "opencode-json" | "unsupported" | string;
  exists: boolean;
  /** false = 该助手在本工具里已停用：条目只读展示。 */
  agent_enabled: boolean;
  /** field = 停用写官方 enabled=false，条目留在文件；snapshot = 停用时移出条目、保存在本工具里。 */
  disable_mode: "field" | "snapshot" | string;
  /** 文件或路径问题（解析失败、符号链接、格式不支持、与其他助手共用文件）；非空时该助手的条目无法列出与操作。 */
  error: string | null;
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
  /** 这个实例自己的额度：实例多半是另一个账号，各算各的。 */
  usage?: AgentUsage | null;
  /** 用第三方接口或凭据：目录里不能有官方账号登录。 */
  third_party?: boolean;
  /** 库里的定义按现在的规则解析不了：只有 blocked_reason，界面只给编辑与移除。 */
  invalid?: boolean;
  /** 实例目录挡不住的系统级设置（组织下发的设置、共享的联合身份档案等）：只有来源与键名。 */
  system_overrides?: { source: string; name: string }[];
  /** 写入基准：实例目标文件本工具负责部分的摘要，文件不存在为 missing；读不了时为 null。 */
  base?: string | null;
  /** 当前完整实例定义的摘要，更新定义和实际文件时一起做写前比对。 */
  definition_base?: string | null;
  /** 实例定义与实际文件逐字段对比；凭据只给是否一致，取值为 <hidden>。 */
  fields?: ProfileField[];
  blocked_reason: string | null;
}

export interface ProfileField {
  /** env.<变量名> / model，Codex 另有 model_provider、provider.base_url 等。 */
  field: string;
  /** same = 一致；differs = 不同；missing = 实际里没有；unresolved = 定义引用的变量取不到值。 */
  state: "same" | "differs" | "missing" | "unresolved" | string;
  definition: string | null;
  live: string | null;
  /** credential = 第三方凭据（定义里是变量名，实际写的是取值），界面单独说明。 */
  purpose?: "credential" | string;
  /** credential 项引用的变量名。 */
  variable?: string | null;
}

// ---------- system:discover（只读接管发现） ----------

/** 接管方式词汇表（引擎侧 manage 字段）：
 * native = 官方路径/原生安装方案，只标识不接管；symlink = 外部资源以软链接纳管；
 * collection = 走已定义集合的接管通道；register = 登记定义（助手 CLI）；
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
  /** 兼容引擎探测字段；当前 UI 不据此提供安装、升级或卸载入口。 */
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
  /** linked=false：源目录里有、但还没被任何助手链接过，接管时一并收录。 */
  skills: { name: string; agents: string[]; has_skill_md: boolean; linked?: boolean }[];
  suggested_definition: { source_path: string; skill_paths: string[]; agents: string[] };
}

export interface DiscoverData {
  domain: "discover";
  agents?: DiscoverAgentCandidate[];
  skills?: { items: DiscoverSkillItem[]; groups: DiscoverSkillGroup[] };
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
