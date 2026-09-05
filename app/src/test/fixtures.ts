import {
  CONTRACT_ID,
  type ApplyResultData,
  type AuditData,
  type EngineElement,
  type Envelope,
  type Freshness,
  type Mode,
  type PlanAction,
  type PolicyData,
  type ProfileData,
  type RunEntry,
  type SnapshotEntry,
} from "../lib/contract";

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
  return { dry_run: false, offline: false, ...overrides };
}

export function makeEnvelope<T>(data: T, overrides: Partial<Envelope<T>> = {}): Envelope<T> {
  return {
    contract: CONTRACT_ID,
    run_id: "20260905T000000Z-test01",
    command: "test.command",
    mode: makeMode(),
    lock: { held: false, pid: null },
    log_file: null,
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
      config: [{ name: "manager.json", status: "ok", message: "格式正确" }],
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
    dry_run: false,
    steps: [
      { name: "mcp-sync", ok: true, exit_code: 0 },
    ],
    snapshot: { run_id: "20260906T010000Z-mock01", path: "/tmp/mock-snapshots/20260906T010000Z-mock01", file_count: 3, link_count: 0 },
    skipped: [],
    ...overrides,
  };
}

export function makeRunEntry(overrides: Partial<RunEntry> = {}): RunEntry {
  return {
    run_id: "20260906T010000Z-mock01",
    log_file: "/tmp/agent_manager_sync_mock_20260906T010000Z-mock01.log",
    size_bytes: 2048,
    mtime: "2026-09-06T01:00:00Z",
    is_latest: false,
    ...overrides,
  };
}

export function makeSnapshotEntry(overrides: Partial<SnapshotEntry> = {}): SnapshotEntry {
  return {
    run_id: "20260906T010000Z-mock01",
    path: "/tmp/mock-snapshots/20260906T010000Z-mock01",
    created_at: "2026-09-06T01:00:00Z",
    command: "mcp.apply",
    file_count: 3,
    link_count: 1,
    ...overrides,
  };
}

export function makePolicyData(overrides: Partial<PolicyData> = {}): PolicyData {
  return {
    layers: {
      baseline: { agents: { "claude-code": { enabled: true } }, mcp: { "grok-search": { enabled: true } } },
      synced: null,
      local: { browser_us_sync: { enabled: false } },
    },
    effective: {
      agents: { "claude-code": { enabled: true } },
      mcp: { "grok-search": { enabled: true } },
      browser_us_sync: { enabled: false },
    },
    attribution: { agents: "baseline", mcp: "baseline", browser_us_sync: "local" },
    layer: null,
    key: null,
    value: null,
    path: null,
    ...overrides,
  };
}

export function makeProfileData(overrides: Partial<ProfileData> = {}): ProfileData {
  return {
    verb: "status",
    state: "clean",
    backend: { type: "git", url: "https://***@git.example.com/demo/profile-sync.git", configured: true },
    machine_id: "demo-mac",
    local: { path: "/repo/config/manager.synced.json", exists: true, key_count: 2 },
    base: { exists: true },
    ahead: 0,
    behind: 0,
    last_sync: "2026-09-05T12:00:00Z",
    conflicts: [],
    snapshot: null,
    published: null,
    gate: null,
    ...overrides,
  };
}

export function makePlanAction(overrides: Partial<PlanAction> = {}): PlanAction {
  return {
    op: "write-file",
    element: "mcp:demo",
    target: "/tmp/demo.json",
    risk: "write-local",
    before_digest: null,
    after_digest: null,
    backup_path: null,
    cmd: null,
    note: null,
    ...overrides,
  };
}
