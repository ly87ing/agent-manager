import type { McpAgentCell } from "../lib/observed";

/** 条目一句话摘要：stdio 为命令行，远程为地址。 */
export function mcpEntrySummary(cell: McpAgentCell): string {
  const entry = cell.entry ?? {};
  if (cell.transport === "remote") return entry.url ?? "";
  return [entry.command ?? "", ...(entry.args ?? [])].join(" ").trim();
}

/** 按包名跑的启动器：取第一个位置参数当服务名。 */
const RUNNERS = new Set(["npx", "uvx", "bunx", "pnpx", "pipx", "dlx"]);
/** 这些选项后面跟一个值，不是服务名。 */
const VALUE_FLAGS = new Set(["--from", "--with", "-p", "--package", "--python", "--index-url"]);

function baseName(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).pop() ?? path;
}

/**
 * 给人看的一句话：「本地命令 · chrome-devtools-mcp@1.9.0」「远程 · developers.openai.com」。
 * 完整命令放在悬停提示里（mcpEntrySummary）。
 */
export function mcpBrief(cell: McpAgentCell): string {
  const entry = cell.entry ?? {};
  if (cell.transport === "remote") {
    try {
      return `远程 · ${new URL(entry.url ?? "").host}`;
    } catch {
      return `远程 · ${entry.url ?? ""}`;
    }
  }
  const command = entry.command ?? "";
  const args = entry.args ?? [];
  if (RUNNERS.has(baseName(command))) {
    let from = "";
    for (let i = 0; i < args.length; i += 1) {
      const arg = args[i];
      if (VALUE_FLAGS.has(arg)) {
        if (arg === "--from") from = args[i + 1] ?? "";
        i += 1;
        continue;
      }
      if (arg.startsWith("-") || arg === "run") continue;
      return `本地命令 · ${arg}${/^git\+|\.git$|github\.com/.test(from) ? "（git）" : ""}`;
    }
  }
  return `本地命令 · ${baseName(command)}`;
}
