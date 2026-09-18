import type { PolicyLayers } from "./contract";

/**
 * 定义覆盖层（引擎 P4a）：mcp.definitions.<name> 覆盖 config/mcp.json，
 * skills.repositories.<name> 覆盖 skills.json。移除用 tombstone 遮蔽同名定义。
 */

export type DefinitionSection = "mcp" | "skills" | "agents";

/** 各定义区在 policy 里的落点：<根>.<定义容器>（引擎读取路径，见 agents.definitions / mcp.definitions / skills.repositories）。 */
const DEFINITION_PATH: Record<DefinitionSection, { root: DefinitionSection; container: string }> = {
  agents: { root: "agents", container: "definitions" },
  mcp: { root: "mcp", container: "definitions" },
  skills: { root: "skills", container: "repositories" },
};

export function definitionKeyPrefix(section: DefinitionSection): string {
  const { root, container } = DEFINITION_PATH[section];
  return `${root}.${container}`;
}

/** 定义容器在层对象里的键名（policy:get 分层结构的读取用）。 */
export function definitionContainerKey(section: DefinitionSection): string {
  return DEFINITION_PATH[section].container;
}

export interface DefinitionRemovalSpec {
  kind: "tombstone";
  /** SQLite 中的本地策略写入层。 */
  layer: "local";
  /** policy key（…/<name>）。 */
  key: string;
}

function layerHasDefinition(layer: Record<string, unknown> | null, section: DefinitionSection, name: string): boolean {
  if (!layer) return false;
  const root = layer[DEFINITION_PATH[section].root];
  if (typeof root !== "object" || root === null) return false;
  const defs = (root as Record<string, unknown>)[definitionContainerKey(section)];
  if (typeof defs !== "object" || defs === null) return false;
  return name in (defs as Record<string, unknown>);
}

/**
 * 移除写 tombstone，避免删除覆盖片段后下层同名定义重新生效。
 * 所有 tombstone 都写入 local，避免删除定义后下层同名定义重新生效。
 */
export function buildDefinitionRemoval(
  _layers: PolicyLayers | null,
  section: DefinitionSection,
  name: string,
): DefinitionRemovalSpec {
  const key = `${definitionKeyPrefix(section)}.${name}`;
  return { kind: "tombstone", layer: "local", key };
}

/** 条目的定义来源（用于 UI 注明「仓库默认值不受影响」）。 */
export function definitionOrigin(
  layers: PolicyLayers | null,
  section: DefinitionSection,
  name: string,
): "builtin" | "sqlite" {
  return layerHasDefinition(layers?.sqlite ?? null, section, name) ? "sqlite" : "builtin";
}
