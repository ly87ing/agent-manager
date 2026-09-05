import {
  CONTRACT_ID,
  type AuditData,
  type EngineElement,
  type Envelope,
  type Freshness,
  type Mode,
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
