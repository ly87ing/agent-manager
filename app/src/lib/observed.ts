/**
 * 各域 spec/policy/observed 的页面级读取类型。
 * 契约层这三层是 free-form object（element.schema.json），这里的接口是
 * 对引擎当前 payload 的防御性镜像：全部字段可选，读取方须容忍缺省。
 */

// ---------- agents ----------

export interface AgentsSpec {
  display_name?: string;
  bin?: string;
  preferred_provider?: string;
  providers?: Record<string, { managed?: boolean }>;
}

export interface AgentsPolicy {
  enabled?: boolean;
  auto_install?: boolean;
  auto_update?: boolean;
  preferred_provider?: string;
}

export interface AgentsObserved {
  providers?: string[];
  classified?: {
    managed?: string[];
    deprecated?: string[];
    system_only?: string[];
    unsupported?: string[];
  };
  installed?: boolean;
  preferred_active?: boolean;
}

// ---------- mcp ----------

/** server × agent 单元格词汇表（引擎 _mcp_cell_state）。 */
export type McpCellState =
  | "managed-ok"
  | "drifted"
  | "missing"
  | "not-materialized"
  | "leftover"
  | "agent-disabled"
  | "probe-error"
  | "absent";

export interface McpAgentCell {
  state: McpCellState | string;
  target?: string;
  target_exists?: boolean;
}

export interface McpSpec {
  command?: string;
  args?: string[];
  startup_timeout_sec?: number;
  agents?: string[] | null;
  env?: Record<string, string>;
  extra_args_by_agent?: Record<string, string[]>;
}

export interface McpEnvResolution {
  placeholder?: string;
  resolved?: boolean;
}

export interface McpObserved {
  agents?: Record<string, McpAgentCell>;
  env?: Record<string, McpEnvResolution>;
}

// ---------- skills ----------

/** skill × agent 单元格词汇表（引擎 _skill_cell_state）。 */
export type SkillCellState = "linked" | "missing-link" | "dead-link" | "drifted" | "extra-link" | "disabled";

export interface SkillAgentCell {
  state: SkillCellState | string;
  link_path?: string;
  target_dir_exists?: boolean;
}

export interface SkillCollectionSpec {
  url?: string;
  skill_paths?: string[];
  agents?: string[] | null;
  matrix_mode?: boolean;
  new_skill_default?: string;
}

export interface SkillCollectionObserved {
  repo_cache?: string;
  repo_cached?: boolean;
  owner_marked?: boolean;
  discovered?: string[];
  discovery_inbox?: string[];
  extra_links?: string[];
}

export interface SkillSpec {
  collection?: string;
  name?: string;
  source_path?: string;
}

export interface SkillPolicy {
  enabled?: boolean;
  agents?: Record<string, boolean>;
}

export interface SkillObserved {
  agents?: Record<string, SkillAgentCell>;
  discovery_inbox?: boolean;
}

// ---------- rules ----------

export interface RulesSpec {
  name?: string;
  group?: string;
  source?: string;
  target?: string;
  source_dir?: string;
  dist_dir?: string;
}

export interface RulesObserved {
  source?: string;
  target?: string;
  source_exists?: boolean;
  target_exists?: boolean;
  build_stale?: boolean;
  src_latest_mtime?: number;
  products?: string[];
  missing_products?: string[];
  stale_products?: string[];
}

// ---------- secrets ----------

export interface SecretsRepoObserved {
  path?: string;
  exists?: boolean;
  is_git?: boolean;
  env_files?: string[];
}

export interface SecretsEnvVariable {
  server?: string;
  env_key?: string;
  placeholder?: string;
  resolved?: boolean;
}

export interface SecretsMcpEnvObserved {
  variables?: SecretsEnvVariable[];
  unresolved?: number;
}

/** 读取 observed.agents 映射（mcp / skills 的 per-agent 单元格）。 */
export function agentCellsOf(element: { observed: Record<string, unknown> }): Record<string, { state: string; target?: string; link_path?: string; target_exists?: boolean }> {
  const cells = element.observed["agents"];
  if (typeof cells !== "object" || cells === null || Array.isArray(cells)) {
    return {};
  }
  return cells as Record<string, { state: string; target?: string; link_path?: string; target_exists?: boolean }>;
}
