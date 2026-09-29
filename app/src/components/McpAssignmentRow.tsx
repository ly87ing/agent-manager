import type { McpAgentCell } from "../lib/observed";

/** 条目一句话摘要：stdio 为命令行，远程为地址。 */
export function mcpEntrySummary(cell: McpAgentCell): string {
  const entry = cell.entry ?? {};
  if (cell.transport === "remote") return entry.url ?? "";
  return [entry.command ?? "", ...(entry.args ?? [])].join(" ").trim();
}
