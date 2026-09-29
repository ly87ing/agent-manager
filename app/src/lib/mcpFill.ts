import type { EngineElement, McpTarget } from "./contract";
import type { McpAgentCell, McpObserved } from "./observed";

/**
 * MCP 默认分配给全部账号（docs/MULTI-ACCOUNT-DESIGN.md 决策 2）。
 *
 * 在某个账号的配置文件里启用着的服务，补到同一格式（同一种助手）里还没有它的账号：
 * 从同格式的账号原样复制，不做 Claude↔Codex 转换（那会丢字段，跨助手仍用「复制到…」）。
 * 账号上已有这个服务（含停用）就不动——停用是用户的例外。
 */
export interface McpFillRow {
  elementId: string;
  name: string;
  from: string;
  base: string;
  to: string;
}

function cellsOf(element: EngineElement): Record<string, McpAgentCell> {
  return (element.observed as McpObserved).agents ?? {};
}

export function mcpFillRows(elements: EngineElement[], targets: McpTarget[], agents?: string[]): McpFillRow[] {
  const writable = targets.filter((target) => target.error === null && target.agent_enabled);
  const formatOf = new Map(targets.map((target) => [target.agent, target.format]));
  return elements.flatMap((element) => {
    const cells = cellsOf(element);
    const sources = Object.entries(cells).filter(([agent, cell]) =>
      cell.source === "file" && cell.state === "enabled" && cell.actions.includes("copy") && formatOf.has(agent));
    return writable
      .filter((target) => !cells[target.agent] && (!agents || agents.includes(target.agent)))
      .flatMap((target) => {
        const source = sources.find(([agent]) => formatOf.get(agent) === target.format);
        return source
          ? [{ elementId: element.id, name: element.id.replace(/^mcp:/, ""), from: source[0], base: source[1].base, to: target.agent }]
          : [];
      });
  });
}

export function mcpFillArgs(row: McpFillRow): string[] {
  return ["--only", row.elementId, "--agent", row.from, "--to", row.to, "--action", "copy", "--base", row.base];
}

/**
 * 新账号除了从主账号带上的 MCP，还会从同格式账号补齐哪些服务：
 * 在同格式的其他账号上启用着、主账号却没有的。
 */
export function mcpInUseElsewhere(elements: EngineElement[], targets: McpTarget[], mainAgent: string): string[] {
  const format = targets.find((target) => target.agent === mainAgent)?.format;
  if (!format) return [];
  const sameFormat = new Set(targets.filter((target) => target.format === format).map((target) => target.agent));
  return elements
    .filter((element) => {
      const cells = cellsOf(element);
      return !cells[mainAgent] && Object.entries(cells).some(([agent, cell]) =>
        sameFormat.has(agent) && cell.source === "file" && cell.state === "enabled" && cell.actions.includes("copy"));
    })
    .map((element) => element.id.replace(/^mcp:/, ""));
}
