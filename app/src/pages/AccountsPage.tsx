import { useQueries } from "@tanstack/react-query";
import { Activity, Copy, LogIn, MoreHorizontal, Plus } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { toast } from "sonner";
import { AccountDrawer, type DrawerMode } from "../components/AccountDrawer";
import { ConfirmModal } from "../components/ConfirmModal";
import { LaunchCommandsDrawer } from "../components/LaunchCommandsDrawer";
import { PageError, PageLoading } from "../components/PageState";
import { buildAccounts, type Account, type AgentAccounts } from "../lib/accounts";
import { isStale, isWindowExpired, usageTone, windowLabel } from "../lib/agentUsage";
import type { StatusData } from "../lib/contract";
import { runEngine } from "../lib/engine";
import { secretSourceOf } from "../lib/secretSource";
import { currentConfigOf } from "../lib/agentCurrent";
import type { SharedStatus } from "../lib/profileShared";
import { useAccountOps } from "../lib/useAccountOps";
import type { AgentUsage, OfficialAuthStatus } from "../lib/observed";
import { authTerminalCommand, parseProfileDefinition } from "../lib/profileDefinition";
import { engineKeys, useEngineQuery } from "../lib/queries";
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

interface RowActions {
  onLaunchCommands: () => void;
  onEdit: () => void;
  onRemove: () => void;
  onAddLaunch: () => void;
  onAdopt: () => void;
  onManageSkills: () => void;
}

function AccountRow({
  account,
  group,
  auth,
  actions,
}: {
  account: Account;
  group: AgentAccounts;
  auth: OfficialAuthStatus | "pending" | null;
  actions: RowActions;
}) {
  const id = account.elementId;
  const loginCommand = authTerminalCommand(group.kind, group.bin, "login", account.isMain ? undefined : account.configDir);
  const loggedOut = account.type === "official" && auth !== "pending" && auth?.state === "logged-out";
  // 登录了哪个账号：Claude 给出邮箱与档位；Codex 官方命令只说登录方式
  const identity = auth && auth !== "pending" && auth.state === "logged-in"
    ? [auth.account?.email, auth.account?.plan].filter(Boolean).join(" · ")
      || (auth.method === "chatgpt" ? "ChatGPT 登录" : auth.method === "api_key" ? "API Key 登录" : "已登录")
    : null;
  const typeText =
    account.type === "api"
      ? `API · ${account.apiHost ?? "未填地址"}`
      : auth === "pending"
        ? "官方 · 检查中"
        : identity
          ? `官方 · ${identity}`
          : auth?.state === "logged-out"
            ? "官方 · 未登录"
            : "官方";
  const typeTitle = auth && auth !== "pending" && auth.account?.org ? `组织：${auth.account.org}` : undefined;
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
          <span data-testid={`account-type-${id}`} title={typeTitle} className={cn("truncate", loggedOut && "text-warning")}>
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
          ...(!account.isMain ? [{ label: "编辑", testId: `account-menu-edit-${id}`, onSelect: actions.onEdit }] : []),
          { label: "启动命令", testId: `account-menu-commands-${id}`, onSelect: actions.onLaunchCommands },
          ...(!account.isMain && !account.command
            ? [{ label: "写入启动命令", testId: `account-menu-launch-${id}`, onSelect: actions.onAddLaunch }]
            : []),
          ...(account.drifted && account.profileBase
            ? [{ label: "采用目录里的内容", testId: `account-menu-adopt-${id}`, onSelect: actions.onAdopt }]
            : []),
          ...(account.type === "official"
            ? [{ label: "复制登录命令", testId: `account-menu-login-${id}`, onSelect: () => void copyText(loginCommand, "已复制登录命令") }]
            : []),
          { label: "技能", testId: `account-menu-skills-${id}`, onSelect: actions.onManageSkills },
          { label: "复制目录路径", testId: `account-menu-dir-${id}`, onSelect: () => void copyText(account.configDir, "已复制目录路径") },
          ...(!account.isMain ? [{ label: "移除", testId: `account-menu-remove-${id}`, onSelect: actions.onRemove }] : []),
        ]}
      />
    </li>
  );
}

/** 账号页：每个助手的全部账号并列，一行一个；添加与编辑在右侧抽屉里。 */
export default function AccountsPage({ onManageSkills }: { onManageSkills?: (accountId: string) => void }) {
  const agentsQuery = useEngineQuery<StatusData>("agents:status");
  const profilesQuery = useEngineQuery<StatusData>("profiles:status");
  const secretsQuery = useEngineQuery<StatusData>("secrets:status");
  const ops = useAccountOps();
  const aliasSource = ops.aliasSource;
  const [selected, setSelected] = useState<string | null>(null);
  const [drawer, setDrawer] = useState<DrawerMode | null>(null);
  const [removing, setRemoving] = useState<{ id: string; name: string; definition: Record<string, unknown> } | null>(null);
  const [removeBusy, setRemoveBusy] = useState(false);
  const [confirmQuota, setConfirmQuota] = useState(false);
  const [commandsOf, setCommandsOf] = useState<Account | null>(null);
  const [quotaBusy, setQuotaBusy] = useState(false);

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

  // 实时额度：只看 Claude 账号（Codex 没有状态栏，额度直接读它自己的会话记录）
  const claudeProfileIds = group?.kind === "claude"
    ? group.accounts.filter((account) => !account.isMain).map((account) => account.elementId.replace(/^profiles:/, ""))
    : [];
  const sharedQueries = useQueries({
    queries: claudeProfileIds.map((id) => ({
      queryKey: engineKeys.request("profiles:shared-status", ["--only", `profiles:${id}`]),
      queryFn: () => runEngine<SharedStatus>("profiles:shared-status", ["--only", `profiles:${id}`]),
    })),
  });

  if (agentsQuery.isPending) return <PageLoading />;
  if (agentsQuery.isError) return <PageError title="账号读取失败" error={agentsQuery.error} />;
  if (!group)
    return (
      <>
        <h1 className="sr-only">账号</h1>
        <p className="text-sm text-muted-foreground">没有识别到 Claude 或 Codex。</p>
      </>
    );

  const agentElements = agentsQuery.data.ok ? agentsQuery.data.data.elements : [];
  /** 实例的完整登记：agents:status 里同名元素的 spec。 */
  const definitionOf = (account: Account): Record<string, unknown> | null => {
    const spec = agentElements.find((item) => item.id === `agents:${account.elementId.replace(/^profiles:/, "")}`)?.spec;
    return spec && typeof spec === "object" && !Array.isArray(spec) ? (spec as Record<string, unknown>) : null;
  };
  const reservedNames = [
    ...groups.map((item) => item.bin.split("/").pop() ?? item.bin),
    "claude",
    "codex",
    ...agentElements.map((item) => item.id.replace(/^agents:/, "")),
  ];
  const withDefinition = (account: Account, run: (definition: Record<string, unknown>) => void) => {
    const definition = definitionOf(account);
    if (definition) run(definition);
    else toast.error("读不到这个账号的登记，刷新后重试");
  };
  const actionsOf = (account: Account): RowActions => {
    const profileId = account.elementId.replace(/^profiles:/, "");
    return {
      onLaunchCommands: () => setCommandsOf(account),
      onEdit: () => withDefinition(account, (definition) => setDrawer({ type: "edit", group, account, definition })),
      onRemove: () => withDefinition(account, (definition) => setRemoving({ id: profileId, name: account.name, definition })),
      onAddLaunch: () => withDefinition(account, (definition) => void ops.addLaunch(profileId, definition, group.bin)),
      onAdopt: () => account.profileBase && void ops.adopt(account.elementId, account.profileBase),
      onManageSkills: () => onManageSkills?.(account.isMain ? group.agentId : profileId),
    };
  };

  const mainElement = agentElements.find((item) => item.id === `agents:${group.agentId}`);
  const mainStatusline = mainElement
    ? (currentConfigOf(mainElement)?.config?.runtime as { statusline?: string } | undefined)?.statusline
    : undefined;
  const quotaNotLive = group.kind === "claude" && Boolean(mainElement) && (
    mainStatusline !== "builtin"
    || sharedQueries.some((query) => query.data?.ok && query.data.data.items.some((item) => item.item === "statusline" && item.state !== "same"))
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
          onClick={() => setDrawer({ type: "create", group })}
          className="ml-auto inline-flex items-center gap-1 rounded-md bg-primary px-3 py-1.5 text-sm text-primary-foreground hover:bg-primary/90"
        >
          <Plus className="h-4 w-4" aria-hidden /> 添加账号
        </button>
      </div>
      {quotaNotLive && (
        <div data-testid="accounts-quota-notice" className="flex items-center gap-2 rounded-md border border-info/30 bg-info/5 px-3 py-2 text-xs">
          <Activity className="h-3.5 w-3.5 text-info" aria-hidden />
          <span>额度还不是实时的：只在 Claude 自己查询时才更新。</span>
          <button
            type="button"
            data-testid="accounts-enable-live-quota"
            onClick={() => setConfirmQuota(true)}
            className="ml-auto rounded border border-primary/30 bg-primary/10 px-2 py-0.5 text-primary hover:bg-primary/15"
          >
            开启实时额度
          </button>
        </div>
      )}
      <ul data-testid="accounts-list" className="divide-y rounded-lg border bg-card">
        {group.accounts.map((account) => (
          <AccountRow
            key={account.elementId}
            account={account}
            group={group}
            auth={authOf(account)}
            actions={actionsOf(account)}
          />
        ))}
      </ul>
      {drawer && (
        <AccountDrawer
          mode={drawer}
          ops={ops}
          secretSource={secretSourceOf(secretsQuery.data?.ok ? secretsQuery.data.data : null)}
          reservedNames={reservedNames}
          onClose={() => setDrawer(null)}
        />
      )}
      {commandsOf && (
        <LaunchCommandsDrawer
          account={commandsOf}
          group={group}
          aliasSource={aliasSource}
          defaultValue={(() => {
            if (commandsOf.isMain) return group.bin;
            const definition = definitionOf(commandsOf);
            const id = commandsOf.elementId.replace(/^profiles:/, "");
            return definition ? ops.launchValue(parseProfileDefinition(id, definition), group.bin) : group.bin;
          })()}
          onClose={() => setCommandsOf(null)}
        />
      )}
      {confirmQuota && mainElement && (
        <ConfirmModal
          title="为全部 Claude 账号开启实时额度"
          description={`把状态栏换成 Agent Manager 的用量状态栏（模型、目录、上下文、额度），它每次刷新时把额度记在本工具的数据目录里，账号页据此显示实时额度。${mainStatusline === "custom" ? "主账号现在的自定义状态栏会被替换。" : ""}先写主账号，再同步给其余 Claude 账号；被外部改过的文件不会覆盖。`}
          risk="write-local"
          confirmLabel="开启"
          executing={quotaBusy}
          onConfirm={async () => {
            setQuotaBusy(true);
            try {
              await ops.enableLiveQuota(mainElement, claudeProfileIds);
              setConfirmQuota(false);
            } finally {
              setQuotaBusy(false);
            }
          }}
          onClose={() => setConfirmQuota(false)}
        />
      )}
      {removing && (
        <ConfirmModal
          title={`移除账号 ${removing.name}`}
          description="删掉它的启动命令并停止管理；目录、登录和会话都保留。以后不用这个目录了，可以先在终端退出登录再自己删掉。"
          risk="write-local"
          confirmLabel="移除"
          executing={removeBusy}
          onConfirm={async () => {
            setRemoveBusy(true);
            try {
              if (await ops.remove(removing.id, removing.definition)) setRemoving(null);
            } finally {
              setRemoveBusy(false);
            }
          }}
          onClose={() => setRemoving(null)}
        />
      )}
    </div>
  );
}
