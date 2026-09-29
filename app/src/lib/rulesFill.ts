import type { EngineElement } from "./contract";
import type { RulesObserved, RulesSpec } from "./observed";

/**
 * 规则默认分配给全部账号（docs/MULTI-ACCOUNT-DESIGN.md 决策 2）。
 *
 * 规则文件常被逐账号改过，已有内容一律不动；补齐只给还没有规则文件的账号，
 * 从同一种助手的主账号复制（rules:apply --from，base = missing，引擎写前再核对）。
 */
export interface RulesFillRow {
  elementId: string;
  name: string;
  from: string;
}

function spec(element: EngineElement): RulesSpec {
  return element.spec as unknown as RulesSpec;
}

function observed(element: EngineElement): RulesObserved {
  return element.observed as unknown as RulesObserved;
}

function writable(element: EngineElement): boolean {
  return element.actions.some((action) => action.id === "write") && typeof observed(element).base === "string";
}

export function rulesFillRows(elements: EngineElement[]): RulesFillRow[] {
  return elements
    .filter((element) => spec(element).group === "profile-rules" && writable(element) && observed(element).base === "missing")
    .flatMap((element) => {
      const main = elements.find((item) =>
        spec(item).group === "global-rules" && spec(item).kind === spec(element).kind && Boolean(observed(item).content?.trim()));
      return main ? [{ elementId: element.id, name: spec(element).name, from: main.id }] : [];
    });
}

export function rulesFillArgs(row: RulesFillRow): string[] {
  return ["--only", row.elementId, "--base", "missing", "--from", row.from];
}
