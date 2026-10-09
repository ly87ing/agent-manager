import type { AgentAccounts } from "./accounts";
import { usageAccountKey } from "./usageModel";

/** `usage:sessions` 的一条：账号目录里最近的一个会话。 */
export interface RecentSession {
  id: string;
  cwd: string | null;
  title: string | null;
  updated_at: string;
}

export interface SessionsData {
  accounts: { agent: string; kind: "claude" | "codex"; sessions: RecentSession[] }[];
}

/** 用量里的账号记法（claude / codex / claude:<实例>）对应的启动命令；还没有启动命令时为 null。 */
export function launchCommandOf(agent: string, groups: AgentAccounts[]): string | null {
  const { id, kind } = usageAccountKey(agent);
  if (!kind) return null;
  const isMain = id === "claude-code" || id === "codex";
  for (const group of groups) {
    if (group.kind !== kind) continue;
    const account = group.accounts.find((item) => (isMain ? item.isMain : item.elementId === `profiles:${id}`));
    if (account) return account.command;
  }
  return null;
}

function posixQuote(value: string): string {
  return /^[A-Za-z0-9_./~-]+$/.test(value) ? value : `'${value.replace(/'/g, `'"'"'`)}'`;
}

/**
 * 回到这个会话的命令：先进它当时的工作目录（Claude 按目录找会话），再用同一个账号的启动命令接着聊。
 * 目录是 Windows 盘符路径时按 cmd 的写法。
 */
export function resumeCommand(kind: "claude" | "codex", launch: string, session: RecentSession): string {
  const resume = kind === "codex" ? `${launch} resume ${session.id}` : `${launch} --resume ${session.id}`;
  if (!session.cwd) return resume;
  if (/^[A-Za-z]:[\\/]/.test(session.cwd)) return `cd /d "${session.cwd}" && ${resume}`;
  return `cd ${posixQuote(session.cwd)} && ${resume}`;
}

/** 「3 分钟前」「2 小时前」「5 天前」。 */
export function agoLabel(at: string, now: Date = new Date()): string {
  const time = new Date(at).getTime();
  if (Number.isNaN(time)) return "";
  const minutes = Math.max(0, Math.round((now.getTime() - time) / 60000));
  if (minutes < 1) return "刚刚";
  if (minutes < 60) return `${minutes} 分钟前`;
  const hours = Math.round(minutes / 60);
  if (hours < 48) return `${hours} 小时前`;
  return `${Math.round(hours / 24)} 天前`;
}
