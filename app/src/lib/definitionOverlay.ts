import type { PolicyLayers } from "./contract";

/**
 * 定义覆盖层（引擎 P4a）：mcp.definitions.<name> 覆盖 config/mcp.json，
 * skills.repositories.<name> 覆盖 skills.json。移除用 tombstone 遮蔽同名定义。
 */

export type DefinitionSection = "mcp" | "skills" | "agents" | "tools";

export function definitionKeyPrefix(section: DefinitionSection): string {
  if (section === "agents") return "agent_definitions";
  if (section === "tools") return "tool_definitions";
  return section === "mcp" ? "mcp.definitions" : "skills.repositories";
}

export interface DefinitionRemovalPlan {
  kind: "tombstone";
  /** SQLite 中的本地策略写入层。 */
  layer: "local";
  /** policy key（…/<name>）。 */
  key: string;
}

function layerHasDefinition(layer: Record<string, unknown> | null, section: DefinitionSection, name: string): boolean {
  if (!layer) return false;
  if (section === "agents" || section === "tools") {
    const defs = layer[definitionKeyPrefix(section)];
    return typeof defs === "object" && defs !== null && name in defs;
  }
  const root = layer[section];
  if (typeof root !== "object" || root === null) return false;
  const defs = (root as Record<string, unknown>)[section === "mcp" ? "definitions" : "repositories"];
  if (typeof defs !== "object" || defs === null) return false;
  return name in (defs as Record<string, unknown>);
}

/**
 * 移除写 tombstone，避免删除覆盖片段后下层同名定义重新生效。
 * 所有 tombstone 都写入 local，避免删除定义后下层同名定义重新生效。
 */
export function planDefinitionRemoval(
  _layers: PolicyLayers | null,
  section: DefinitionSection,
  name: string,
): DefinitionRemovalPlan {
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
