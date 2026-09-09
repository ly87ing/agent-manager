/**
 * Contract v1（contract/engine-v1/*.schema.json）的 TypeScript 镜像。
 * 字段命名与 schema 完全一致（snake_case），可空字段遵循 schema 的
 * 「required but nullable」约定：键恒在，值为 null。
 */

export const CONTRACT_ID = "agent-manager/engine/v1";

// ---------- envelope ----------

export interface Mode {
  dry_run: boolean;
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
  defaultDryRun: boolean;
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
}

// ---------- <domain>:plan ----------

export type PlanOp =
  | "write-file"
  | "run-cmd"
  | "symlink"
  | "unlink"
  | "install"
  | "uninstall"
  | "git-pull"
  | "render";

export interface PlanAction {
  op: PlanOp;
  element: string;
  target: string | null;
  risk: RiskLevel;
  before_digest: string | null;
  after_digest: string | null;
  backup_path: string | null;
  cmd: string | null;
  note: string | null;
}

export interface PlanSummary {
  read: number;
  "write-local": number;
  "exec-local": number;
  network: number;
  "install-remove": number;
}

export interface PlanData {
  domain: string;
  actions: PlanAction[];
  summary: PlanSummary;
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
  cache_file: string;
  elements: RefreshUpstreamEntry[];
  summary: RefreshUpstreamSummary;
}

// ---------- <domain>:apply / system:converge（apply-result / converge schema） ----------

export interface ApplyStep {
  name: string;
  ok: boolean;
  exit_code: number;
}

export interface SnapshotEcho {
  run_id: string;
  path: string;
  file_count: number;
  link_count: number;
}

export interface ApplyResultData {
  domain: string;
  dry_run: boolean;
  steps: ApplyStep[];
  snapshot: SnapshotEcho | null;
  skipped: string[];
}

export interface ConvergeData {
  pipeline: "update";
  dry_run: boolean;
  steps: ApplyStep[];
  snapshot: SnapshotEcho | null;
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

// ---------- snapshots:list / snapshots:restore ----------

export interface SnapshotEntry {
  run_id: string;
  path: string;
  created_at: string | null;
  command: string | null;
  file_count: number;
  link_count: number;
}

export interface RestoreResultData {
  restored_run: string;
  files_restored: number;
  files_deleted: number;
  links_restored: number;
  skipped: string[];
  resnapshot_run_id: string;
}

export interface SnapshotsData {
  snapshots: SnapshotEntry[] | null;
  restore: RestoreResultData | null;
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
