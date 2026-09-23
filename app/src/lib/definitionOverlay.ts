import type { PolicyLayers } from "./contract";

/**
 * 定义覆盖层（引擎 P4a）：agents.definitions.<name> 与 skills.repositories.<name>
 * 覆盖引擎内置的同名定义。移除用 tombstone 遮蔽同名定义。
 * MCP 不在其中：MCP 条目以各助手配置文件为准，直接由 mcp:apply 逐条编辑。
 */

export type DefinitionSection = "skills" | "agents";

/** 各定义区在 policy 里的落点：<根>.<定义容器>（引擎读取路径，见 agents.definitions / skills.repositories）。 */
const DEFINITION_PATH: Record<DefinitionSection, { root: DefinitionSection; container: string }> = {
  agents: { root: "agents", container: "definitions" },
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
