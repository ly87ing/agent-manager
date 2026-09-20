import type { EngineElement } from "./contract";
import { isMcpTakeoverBlocked } from "./mcpImport";
import { agentCellsOf, type McpSpec } from "./observed";
import type { BatchApplyRow } from "./useBatchApply";

type McpSyncCell = ReturnType<typeof agentCellsOf>[string] & {
  assigned?: boolean;
  enabled?: boolean;
  actions?: string[];
  blocked_reason?: string | null;
};

export interface McpSyncRow extends BatchApplyRow {
  reason: string;
}

export interface McpSyncBlocked {
  agent: string;
  reason: string;
}

export interface McpSyncPlan {
  rows: McpSyncRow[];
  blocked: McpSyncBlocked[];
  takeoverCount: number;
  removeCount: number;
}

function isAllowed(spec: McpSpec, agent: string): boolean {
  return spec.agents == null || spec.agents.includes(agent);
}

function firstAction(cell: McpSyncCell, candidates: string[]): string | null {
  const actions = cell.actions ?? [];
  return candidates.find((action) => actions.includes(action)) ?? null;
}

/**
 * 把一个 MCP 服务的当前状态收敛成最小逐助手动作集合。
 * 服务库启用时，定义中的 agents（null 表示全部）是期望落地范围；停用时则移除
 * 已启用的受管配置但保留绑定。外部同名条目只生成显式 takeover 行，交给 UI 汇总确认。
 */
export function buildMcpSyncPlan(element: EngineElement): McpSyncPlan {
  const spec = element.spec as McpSpec;
  const libraryEnabled = (element.policy as { enabled?: boolean }).enabled !== false;
  const rows: McpSyncRow[] = [];
  const blocked: McpSyncBlocked[] = [];

  for (const [agent, rawCell] of Object.entries(agentCellsOf(element))) {
    const cell = rawCell as McpSyncCell;
    const allowed = isAllowed(spec, agent);
    let action: string | null = null;
    let takeover = false;
    let reason = "";

    if (!libraryEnabled) {
      if (cell.assigned && cell.enabled !== false) {
        action = firstAction(cell, ["disable"]);
        reason = "服务已停用，将移除这个助手中的运行配置。";
      } else if (!cell.assigned && cell.state === "leftover") {
        action = firstAction(cell, ["remove"]);
        reason = "分配已取消，将清理这个助手中的旧配置。";
      }
    } else if (!allowed) {
      if (cell.assigned && cell.enabled !== false) {
        action = firstAction(cell, ["disable"]);
        reason = "这个助手不在应用范围内，将移除运行配置。";
      } else if (!cell.assigned && cell.state === "leftover") {
        action = firstAction(cell, ["remove"]);
        reason = "这个助手不在应用范围内，将清理旧配置。";
      }
    } else if (cell.state !== "managed-ok") {
      if (isMcpTakeoverBlocked(cell.blocked_reason)) {
        action = cell.assigned ? "update" : "add";
        takeover = true;
        reason = "将用服务库定义接管同名条目。";
      } else if (cell.state === "leftover") {
        action = "add";
        takeover = true;
        reason = "将重新纳入管理并按服务库定义同步。";
      } else {
        action = firstAction(cell, ["update", "add", "enable"]);
        if (action) reason = "将按服务库定义同步到这个助手。";
      }
    }

    if (action) {
      rows.push({
        elementId: element.id,
        agent,
        action,
        label: `${element.id} @ ${agent}`,
        takeover,
        reason,
      });
      continue;
    }

    const needsSync = libraryEnabled
      ? allowed && cell.state !== "managed-ok" && cell.state !== "agent-disabled"
      : (cell.assigned && cell.enabled !== false) || cell.state === "leftover";
    if (needsSync) {
      blocked.push({
        agent,
        reason: cell.blocked_reason ?? "当前配置无法自动同步，请先检查目标文件或服务定义。",
      });
    }
  }

  return {
    rows,
    blocked,
    takeoverCount: rows.filter((row) => row.takeover).length,
    removeCount: rows.filter((row) => row.action === "remove").length,
  };
}
