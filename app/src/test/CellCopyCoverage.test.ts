import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { MCP_CELL_COPY, SKILL_CELL_COPY } from "../lib/ui-copy";

/**
 * 引擎每加一个格子状态，界面就必须有对应的中文说法，否则 MatrixCell 会回落到把
 * 英文状态名（not-assigned 之类）直接显给用户。这条守卫从引擎源码里取状态集合比对。
 */
function enginePolicyStates(file: string): string[] {
  const source = readFileSync(file, "utf8");
  // 引擎里格子状态一律写成 cell_state("xxx") / state: "xxx".to_string() 之类的字面量，
  // 这里取所有 kebab-case 字面量再与已知文案求交，未知的即为缺口。
  return [...new Set(source.match(/"[a-z]+(?:-[a-z]+)+"/g) ?? [])].map((token) => token.slice(1, -1));
}

const KNOWN_NON_CELL = new Set([
  "allow-takeover", "dry-run", "write-local", "exec-local", "install-remove", "read-only",
  "not-applicable", "agent-manager", "content-type", "skill-paths", "source-path",
]);

describe("格子文案覆盖", () => {
  it("技能：引擎输出的格子状态都有中文说法", () => {
    const states = enginePolicyStates("../engine-rs/src/native_skills/status.rs")
      .filter((state) => !KNOWN_NON_CELL.has(state))
      .filter((state) => CANDIDATE_SKILL_STATES.has(state));
    expect(states.filter((state) => !(state in SKILL_CELL_COPY))).toEqual([]);
  });

  it("MCP：引擎输出的格子状态都有中文说法", () => {
    const states = enginePolicyStates("../engine-rs/src/native_mcp/status.rs")
      .filter((state) => !KNOWN_NON_CELL.has(state))
      .filter((state) => CANDIDATE_MCP_STATES.has(state));
    expect(states.filter((state) => !(state in MCP_CELL_COPY))).toEqual([]);
  });
});

/** 引擎源码里的 kebab 字面量不止格子状态，这里圈定要覆盖的那些。 */
const CANDIDATE_SKILL_STATES = new Set([
  "linked", "missing-link", "dead-link", "drifted", "extra-link", "disabled",
  "unsupported", "not-assigned", "not-materialized", "managed-ok",
]);
const CANDIDATE_MCP_STATES = new Set([
  "managed-ok", "drifted", "missing", "absent", "agent-disabled", "not-materialized",
  "leftover", "probe-error", "disabled", "unsupported",
]);
