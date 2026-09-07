import type { PolicyLayer, PolicyLayers } from "./contract";

/**
 * 定义覆盖层（引擎 P4a）：mcp.definitions.<name> 覆盖 config/mcp.json，
 * skills.repositories.<name> 覆盖 skills.json。基线条目的移除 = tombstone
 * （{"_delete": true}）；覆盖层条目的移除 = policy:delete。
 */

export type DefinitionSection = "mcp" | "skills";

export function definitionKeyPrefix(section: DefinitionSection): string {
  return section === "mcp" ? "mcp.definitions" : "skills.repositories";
}

export interface DefinitionRemovalPlan {
  /** overlay = policy:delete；tombstone = policy:set {"_delete": true}。 */
  kind: "overlay" | "tombstone";
  /** 目标层。 */
  layer: PolicyLayer;
  /** policy key（…/<name>）。 */
  key: string;
}

function layerHasDefinition(layer: Record<string, unknown> | null, section: DefinitionSection, name: string): boolean {
  if (!layer) return false;
  const root = layer[section];
  if (typeof root !== "object" || root === null) return false;
  const defs = (root as Record<string, unknown>)[section === "mcp" ? "definitions" : "repositories"];
  if (typeof defs !== "object" || defs === null) return false;
  return name in (defs as Record<string, unknown>);
}

/**
 * 移除决策：定义在 local 层 → 从 local 删除；其次 synced；两层都没有
 * （基线/未定义）→ 往默认可写层写 tombstone。同名定义横跨两层时只删 local
 * （local 优先于 synced 生效，删掉它后 synced 片段会浮现——属刻意保守，界面注明）。
 */
export function planDefinitionRemoval(
  layers: PolicyLayers | null,
  section: DefinitionSection,
  name: string,
  defaultLayer: PolicyLayer,
): DefinitionRemovalPlan {
  const key = `${definitionKeyPrefix(section)}.${name}`;
  if (layerHasDefinition(layers?.local ?? null, section, name)) {
    return { kind: "overlay", layer: "local", key };
  }
  if (layerHasDefinition(layers?.synced ?? null, section, name)) {
    return { kind: "overlay", layer: "synced", key };
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
