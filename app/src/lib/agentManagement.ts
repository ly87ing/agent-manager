import type { EngineElement } from "./contract";
import type { AgentManagement, AgentsObserved } from "./observed";

/**
 * 读取后端适配器能力。契约的 observed 是自由对象，所以这里 fail-close：
 * schema 不完整时按不支持处理，绝不根据名字猜一个写入适配器。
 */
export function agentManagementOf(element: EngineElement): AgentManagement | null {
  const raw = (element.observed as AgentsObserved).management;
  if (
    !raw ||
    typeof raw.adapter !== "string" ||
    typeof raw.schema_version !== "number" ||
    !Array.isArray(raw.runtime_fields)
  ) {
    return null;
  }
  return raw;
}

export function connectionKindOf(element: EngineElement): "claude" | "codex" | null {
  const kind = agentManagementOf(element)?.connection_kind;
  return kind === "claude" || kind === "codex" ? kind : null;
}

/** 主助手由适配器声明；实例由后端 profile.kind 明确声明。两者都只接受 Claude/Codex。 */
export function supportedAgentKindOf(element: EngineElement): "claude" | "codex" | null {
  const primary = connectionKindOf(element);
  if (primary) return primary;
  const profile = (element.spec as Record<string, unknown>).profile;
  if (!profile || typeof profile !== "object" || Array.isArray(profile)) return null;
  const kind = (profile as Record<string, unknown>).kind;
  return kind === "claude" || kind === "codex" ? kind : null;
}
