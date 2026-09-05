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
  };
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
