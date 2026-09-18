import {
  CONTRACT_ID,
  type ApplyResultData,
  type AuditData,
  type EngineElement,
  type Envelope,
  type Freshness,
  type Mode,
  type PolicyData,
  type RunEntry,
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
  return { offline: false, ...overrides };
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
