import type { PolicyLayer, PolicyLayers } from "./contract";

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
  /** 目标层。 */
  layer: PolicyLayer;
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
 * 已有 local 覆盖时必须在 local 层遮蔽；其余使用用户选择的默认层。
 */
export function planDefinitionRemoval(
  layers: PolicyLayers | null,
  section: DefinitionSection,
  name: string,
  defaultLayer: PolicyLayer,
): DefinitionRemovalPlan {
  const key = `${definitionKeyPrefix(section)}.${name}`;
  if (layerHasDefinition(layers?.local ?? null, section, name)) {
    return { kind: "tombstone", layer: "local", key };
  }
  return { kind: "tombstone", layer: defaultLayer, key };
}

/** 条目的定义来源（用于 UI 注明「仓库默认值不受影响」）。 */
export function definitionOrigin(
  layers: PolicyLayers | null,
  section: DefinitionSection,
  name: string,
): "baseline" | "local" | "synced" {
  if (layerHasDefinition(layers?.local ?? null, section, name)) return "local";
  if (layerHasDefinition(layers?.synced ?? null, section, name)) return "synced";
  return "baseline";
}
