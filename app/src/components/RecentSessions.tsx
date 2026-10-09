import { Copy } from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";
import type { Envelope } from "../lib/contract";
import { useEngineQuery } from "../lib/queries";
import { agoLabel, launchCommandOf, resumeCommand, type SessionsData } from "../lib/sessions";
import { useAccountList } from "../lib/useAccountNames";
import { usageAccountKey } from "../lib/usageModel";
import { AgentIcon } from "./AgentIcon";

const SHOWN = 10;

/** 用量与会话对同一个账号的记法可能不同（claude / claude-code），按账号键比。 */
function sameAccount(a: string, b: string): boolean {
  const left = usageAccountKey(a);
  const right = usageAccountKey(b);
  return left.kind === right.kind && left.id === right.id;
}

/** 把主目录写成 ~，目录太长时只看得清结尾那几段也够认。 */
function shortDir(cwd: string): string {
  return cwd.replace(/^\/(Users|home)\/[^/]+/, "~");
}

/**
 * 最近会话：每个账号目录里最近的会话，复制一条「进目录 + 同一个账号接着聊」的命令。
 * 只读会话文件开头，不写任何东西，也不替你打开终端。
 */
export function RecentSessions({ agent, labelOf }: { agent: string; labelOf: (agent: string) => string }) {
  const query = useEngineQuery<SessionsData>("usage:sessions");
  const groups = useAccountList();
  const [now] = useState(() => new Date());
  const envelope = query.data as Envelope<SessionsData> | undefined;
  if (!envelope?.ok) return null;

  const rows = envelope.data.accounts
    .filter((account) => !agent || sameAccount(account.agent, agent))
    .flatMap((account) => account.sessions.map((session) => ({ account, session })))
    .sort((a, b) => b.session.updated_at.localeCompare(a.session.updated_at))
    .slice(0, SHOWN);
  if (rows.length === 0) return null;

  const copy = async (command: string) => {
    try {
      await navigator.clipboard.writeText(command);
      toast.success("已复制，到终端里粘贴运行");
    } catch {
      toast.error("复制失败，请检查剪贴板权限");
    }
  };

  return (
    <section data-testid="recent-sessions" className="rounded-lg border bg-card shadow-sm">
      <div className="flex items-baseline gap-2 border-b px-5 py-3">
        <h3 className="text-sm font-medium">最近会话</h3>
        <span className="text-xs text-muted-foreground">复制的命令先进当时的目录，再用同一个账号接着聊</span>
      </div>
      <ul className="divide-y">
        {rows.map(({ account, session }) => {
          const launch = launchCommandOf(account.agent, groups);
          return (
            <li key={`${account.agent}/${session.id}`} data-testid={`session-${session.id}`} className="flex items-center gap-3 px-5 py-2">
              <AgentIcon kind={account.kind} />
              <div className="min-w-0 flex-1">
                <div className="truncate text-sm" title={session.title ?? undefined}>
                  {session.title ?? <span className="text-muted-foreground">（没有标题）</span>}
                </div>
                <div className="flex min-w-0 items-center gap-1.5 text-xs text-muted-foreground">
                  {!agent && <span className="shrink-0">{labelOf(account.agent)}</span>}
                  {!agent && <span aria-hidden>·</span>}
                  {session.cwd && <span className="truncate font-mono" title={session.cwd}>{shortDir(session.cwd)}</span>}
                  {session.cwd && <span aria-hidden>·</span>}
                  <span className="shrink-0">{agoLabel(session.updated_at, now)}</span>
                </div>
              </div>
              <button
                type="button"
                data-testid={`session-copy-${session.id}`}
                disabled={!launch}
                title={launch ? resumeCommand(account.kind, launch, session) : "这个账号还没有启动命令，先到账号页写入"}
                onClick={() => launch && void copy(resumeCommand(account.kind, launch, session))}
                className="inline-flex w-24 shrink-0 items-center justify-center gap-1 rounded-md border px-2.5 py-1.5 text-xs hover:bg-accent disabled:opacity-40"
              >
                <Copy className="h-3.5 w-3.5" aria-hidden /> 复制命令
              </button>
            </li>
          );
        })}
      </ul>
    </section>
  );
}
