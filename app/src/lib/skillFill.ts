import type { EngineElement } from "./contract";
import type { SkillAgentCell, SkillObserved, SkillSpec } from "./observed";
import type { BatchApplyRow } from "./useBatchApply";

/**
 * 技能默认分配给全部账号（docs/MULTI-ACCOUNT-DESIGN.md 决策 2）。
 *
 * 「在用」= 至少一个账号上已分配且启用。在用的技能在其余账号上还没分配时，补齐就是给它们 add。
 * 某个账号上停用（disabled）是用户的例外，不补；从没被任何账号选用的技能（新发现）要用户先「添加」，
 * 添加即加到全部账号。补齐只新增链接，引擎对已有真实目录或外部链接仍 fail-closed。
 */
const ACTIVE_STATES = new Set(["linked", "missing-link", "dead-link", "drifted"]);

export function isSkillElement(element: EngineElement): boolean {
  return Boolean((element.spec as SkillSpec).collection);
}

function cellsOf(skill: EngineElement): Record<string, SkillAgentCell> {
  return (skill.observed as SkillObserved).agents ?? {};
}

function isActive(cell: SkillAgentCell): boolean {
  return cell.assigned !== false && ACTIVE_STATES.has(cell.state);
}

function canAdd(cell: SkillAgentCell | undefined): boolean {
  return cell?.state === "not-assigned" && (cell.actions ?? []).includes("add");
}

function row(skill: EngineElement, agent: string, verb: string): BatchApplyRow {
  const name = (skill.spec as SkillSpec).name ?? skill.id;
  return { elementId: skill.id, agent, action: "add", label: `${verb} ${name} @ ${agent}`, takeover: false };
}

/** 在用技能在哪些账号上缺分配；给了 agents 时只看这些账号（新建账号后只补它自己）。 */
export function fillRows(elements: EngineElement[], agents?: string[]): BatchApplyRow[] {
  return elements.filter(isSkillElement).flatMap((skill) => {
    const cells = cellsOf(skill);
    if (!Object.values(cells).some(isActive)) return [];
    return Object.entries(cells)
      .filter(([agent, cell]) => (!agents || agents.includes(agent)) && canAdd(cell))
      .map(([agent]) => row(skill, agent, "补齐"));
  });
}

/** 添加一个还没被任何账号选用的技能：一次加到全部可加的账号。 */
export function addToAllRows(skill: EngineElement): BatchApplyRow[] {
  return Object.entries(cellsOf(skill))
    .filter(([, cell]) => canAdd(cell))
    .map(([agent]) => row(skill, agent, "添加"));
}

/** 在用的技能数：新账号建好后会链接的就是这些。 */
export function inUseSkillCount(elements: EngineElement[]): number {
  return elements.filter(isSkillElement).filter((skill) => Object.values(cellsOf(skill)).some(isActive)).length;
}

/**
 * 各账号在用、来源里已有新版本的技能：「全部更新」把它们换到新版本。
 * 只看版本变化；链接损坏的修复、新发现技能的添加都不在这里。
 */
export function versionUpdateRows(elements: EngineElement[]): BatchApplyRow[] {
  return elements.filter(isSkillElement).flatMap((skill) =>
    Object.entries(cellsOf(skill))
      .filter(([, cell]) =>
        cell.state === "linked" && (cell.actions ?? []).includes("update") && Boolean(cell.available_version) && cell.version !== cell.available_version)
      .map(([agent]) => ({ ...row(skill, agent, "更新"), action: "update" })),
  );
}
