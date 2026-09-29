import { useQueries } from "@tanstack/react-query";
import { Copy, LogIn, MoreHorizontal, Plus } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { toast } from "sonner";
import { PageError, PageLoading } from "../components/PageState";
import { buildAccounts, type Account, type AgentAccounts } from "../lib/accounts";
import { isStale, isWindowExpired, usageTone, windowLabel } from "../lib/agentUsage";
import type { StatusData } from "../lib/contract";
import { runEngine } from "../lib/engine";
import type { AgentUsage, OfficialAuthStatus } from "../lib/observed";
import { authTerminalCommand } from "../lib/profileDefinition";
import { engineKeys, useEngineQuery } from "../lib/queries";
import { useAliasSource } from "../lib/useAliasSource";
import { cn } from "../lib/utils";

const TONE_BAR = { ok: "bg-success", warn: "bg-warning", critical: "bg-destructive" } as const;

async function copyText(text: string, message: string) {
  try {
    await navigator.clipboard.writeText(text);
    toast.success(message);
  } catch {
    toast.error("复制失败，请检查剪贴板权限");
  }
}

/** 一行放得下的额度：每个未过重置点的窗口一小段条。 */
function QuotaCells({ usage, testId }: { usage: AgentUsage | null; testId: string }) {
  const windows = (usage?.windows ?? []).filter((w) => !isWindowExpired(w.resets_at));
  if (!usage || windows.length === 0) return <span className="text-xs text-muted-foreground">—</span>;
  const stale = isStale(usage.checked_at);
  return (
    <div data-testid={testId} className="flex items-center gap-3" title={`读自 ${usage.source}`}>
      {windows.map((w) => (
        <span key={w.window_minutes} className="flex items-center gap-1.5 text-xs">
          <span className="text-muted-foreground">{windowLabel(w.window_minutes)}</span>
          <span className="h-1.5 w-14 overflow-hidden rounded bg-muted">
            <span
              className={cn("block h-full", stale ? "bg-muted-foreground/40" : TONE_BAR[usageTone(w.used_percent)])}
              style={{ width: `${Math.min(100, Math.max(0, w.used_percent))}%` }}
            />
          </span>
          <span className={cn("w-9 tabular-nums", stale && "text-muted-foreground")}>
            {Math.round(w.used_percent)}%{stale ? "?" : ""}
          </span>
        </span>
      ))}
    </div>
  );
}

function RowMenu({ items }: { items: { label: string; onSelect: () => void; testId: string }[] }) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const close = (event: MouseEvent) => {
      if (!ref.current?.contains(event.target as Node)) setOpen(false);
    };
    window.addEventListener("mousedown", close);
    return () => window.removeEventListener("mousedown", close);
  }, [open]);
  return (
    <div ref={ref} className="relative">
      <button
        type="button"
        aria-label="更多操作"
        aria-expanded={open}
        onClick={() => setOpen((value) => !value)}
        className="rounded p-1.5 text-muted-foreground hover:bg-accent hover:text-foreground"
      >
        <MoreHorizontal className="h-4 w-4" aria-hidden />
      </button>
      {open && (
        <div role="menu" className="absolute right-0 z-20 mt-1 min-w-40 rounded-md border bg-popover p-1 shadow-lg">
          {items.map((item) => (
            <button
              key={item.testId}
              type="button"
              role="menuitem"
              data-testid={item.testId}
              onClick={() => {
                setOpen(false);
                item.onSelect();
              }}
              className="block w-full rounded px-2.5 py-1.5 text-left text-sm hover:bg-accent"
            >
              {item.label}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

function AccountRow({
  account,
  group,
  auth,
  onManage,
}: {
  account: Account;
  group: AgentAccounts;
  auth: OfficialAuthStatus | "pending" | null;
  onManage: () => void;
}) {
  const id = account.elementId;
  const loginCommand = authTerminalCommand(group.kind, group.bin, "login", account.isMain ? undefined : account.configDir);
  const loggedOut = account.type === "official" && auth !== "pending" && auth?.state === "logged-out";
  const typeText =
    account.type === "api"
      ? `API · ${account.apiHost ?? "未填地址"}`
      : auth === "pending"
        ? "官方 · 检查中"
        : auth?.state === "logged-in"
          ? "官方 · 已登录"
          : auth?.state === "logged-out"
            ? "官方 · 未登录"
            : "官方";
  const badges: { text: string; tone: "warn" | "muted" }[] = [];
  if (!account.isMain && !account.command) badges.push({ text: "没有启动命令", tone: "warn" });
  if (account.drifted) badges.push({ text: "目录被外部改过", tone: "warn" });
  if (account.mainHasThirdParty) badges.push({ text: "主账号含第三方接入", tone: "warn" });
  if (account.blockedReason) badges.push({ text: "登记读不了", tone: "warn" });

  return (
    <li data-testid={`account-row-${id}`} className="flex items-center gap-3 px-4 py-3">
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <span className="truncate font-medium">{account.name}</span>
          {account.isMain && (
            <span className="rounded bg-muted px-1.5 py-0.5 text-[11px] text-muted-foreground">主账号</span>
          )}
          {badges.map((badge) => (
            <span
              key={badge.text}
              className={cn(
                "rounded px-1.5 py-0.5 text-[11px]",
                badge.tone === "warn" ? "bg-warning/15 text-warning" : "bg-muted text-muted-foreground",
              )}
            >
              {badge.text}
            </span>
          ))}
        </div>
        <div className="mt-0.5 flex items-center gap-2 text-xs text-muted-foreground">
          {account.command && <code className="font-mono text-foreground">{account.command}</code>}
          <span data-testid={`account-type-${id}`} className={cn(loggedOut && "text-warning")}>
            {typeText}
          </span>
          {account.model && <span className="truncate">· {account.model}</span>}
        </div>
      </div>
      <div className="hidden md:block">
        <QuotaCells usage={account.usage} testId={`account-quota-${id}`} />
      </div>
      {loggedOut ? (
        <button
          type="button"
          data-testid={`account-login-${id}`}
          onClick={() => void copyText(loginCommand, "已复制登录命令，到终端里运行")}
          className="inline-flex w-20 items-center justify-center gap-1 rounded-md bg-primary px-2.5 py-1.5 text-xs text-primary-foreground hover:bg-primary/90"
        >
          <LogIn className="h-3.5 w-3.5" aria-hidden /> 登录
        </button>
      ) : (
        <button
          type="button"
          data-testid={`account-copy-${id}`}
          disabled={!account.command}
          onClick={() => account.command && void copyText(account.command, `已复制启动命令 ${account.command}`)}
          className="inline-flex w-20 items-center justify-center gap-1 rounded-md border px-2.5 py-1.5 text-xs hover:bg-accent disabled:opacity-40"
        >
          <Copy className="h-3.5 w-3.5" aria-hidden /> 复制
        </button>
      )}
      <RowMenu
        items={[
          ...(account.type === "official"
            ? [{ label: "复制登录命令", testId: `account-menu-login-${id}`, onSelect: () => void copyText(loginCommand, "已复制登录命令") }]
            : []),
          { label: "复制目录路径", testId: `account-menu-dir-${id}`, onSelect: () => void copyText(account.configDir, "已复制目录路径") },
          { label: "编辑与管理", testId: `account-menu-manage-${id}`, onSelect: onManage },
        ]}
      />
    </li>
  );
}

/** 账号页：每个助手的全部账号并列，一行一个；编辑与新建在 P2 改为抽屉，暂时进入旧管理页。 */
export default function AccountsPage({ onManage }: { onManage: () => void }) {
  const agentsQuery = useEngineQuery<StatusData>("agents:status");
  const profilesQuery = useEngineQuery<StatusData>("profiles:status");
  const aliasSource = useAliasSource();
  const [selected, setSelected] = useState<string | null>(null);

  const groups =
    agentsQuery.data?.ok
      ? buildAccounts(
          agentsQuery.data.data.elements,
          profilesQuery.data?.ok ? profilesQuery.data.data.elements : [],
          aliasSource.source?.aliases ?? {},
        )
      : [];
  const group = groups.find((item) => item.agentId === selected) ?? groups[0];
  const officialIds = (group?.accounts ?? []).filter((a) => a.type === "official").map((a) => a.elementId);
  const authQueries = useQueries({
    queries: officialIds.map((only) => ({
      queryKey: engineKeys.request("agents:auth-status", ["--only", only]),
      queryFn: () => runEngine<OfficialAuthStatus>("agents:auth-status", ["--only", only]),
      staleTime: 5 * 60_000,
    })),
  });

  if (agentsQuery.isPending) return <PageLoading />;
  if (agentsQuery.isError) return <PageError title="账号读取失败" error={agentsQuery.error} />;
  if (!group)
    return (
      <>
        <h1 className="sr-only">账号</h1>
        <p className="text-sm text-muted-foreground">没有识别到 Claude Code 或 Codex。</p>
      </>
    );

  const authOf = (account: Account): OfficialAuthStatus | "pending" | null => {
    const index = officialIds.indexOf(account.elementId);
    if (index < 0) return null;
    const query = authQueries[index];
    if (query.isPending) return "pending";
    return query.data?.ok ? query.data.data : null;
  };

  return (
    <div className="space-y-3">
      <h1 className="sr-only">账号</h1>
      <div className="flex items-center gap-3">
        <div role="tablist" className="inline-flex rounded-lg bg-muted p-1">
          {groups.map((item) => (
            <button
              key={item.agentId}
              type="button"
              role="tab"
              aria-selected={item.agentId === group.agentId}
              data-testid={`accounts-tab-${item.agentId}`}
              onClick={() => setSelected(item.agentId)}
              className={cn(
                "rounded-md px-3 py-1 text-sm",
                item.agentId === group.agentId ? "bg-background font-medium shadow-sm" : "text-muted-foreground",
              )}
            >
              {item.label}
              <span className="ml-1.5 text-xs text-muted-foreground">{item.accounts.length}</span>
            </button>
          ))}
        </div>
        <button
          type="button"
          data-testid="accounts-add"
          onClick={onManage}
          className="ml-auto inline-flex items-center gap-1 rounded-md bg-primary px-3 py-1.5 text-sm text-primary-foreground hover:bg-primary/90"
        >
          <Plus className="h-4 w-4" aria-hidden /> 添加账号
        </button>
      </div>
      <ul data-testid="accounts-list" className="divide-y rounded-lg border bg-card">
        {group.accounts.map((account) => (
          <AccountRow
            key={account.elementId}
            account={account}
            group={group}
            auth={authOf(account)}
            onManage={onManage}
          />
        ))}
      </ul>
    </div>
  );
}
