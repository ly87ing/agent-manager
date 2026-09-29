import type { EngineElement, PolicyLayers } from "./contract";
import type { SkillCollectionObserved } from "./observed";

/** 集合元素的 id 是 `skill:<集合>`，技能是 `skill:<集合>/<技能>`。 */
export function isCollection(element: EngineElement): boolean {
  return element.id.startsWith("skill:") && !element.id.includes("/");
}

/** 库里这个集合的完整定义（编辑时整条替换的基底）；读不到为 null。 */
export function fullSkillDefinition(layers: PolicyLayers | null, name: string): Record<string, unknown> | null {
  const repositories = (layers?.sqlite as { skills?: { repositories?: Record<string, unknown> } } | null | undefined)?.skills?.repositories;
  const definition = repositories?.[name];
  return typeof definition === "object" && definition !== null ? (definition as Record<string, unknown>) : null;
}

/** 来源里的全部技能；旧引擎没有 available 时退回按 discovered（当作顶层技能）。 */
export function availableSkills(observed: SkillCollectionObserved) {
  return observed.available ?? (observed.discovered ?? []).map((name) => ({ name, path: name }));
}
