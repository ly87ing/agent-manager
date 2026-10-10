import { useQueries, useQueryClient } from "@tanstack/react-query";
import { Activity, Copy, LogIn, Plus, RefreshCw } from "lucide-react";
import { useEffect, useState } from "react";
import { toast } from "sonner";
import { AccountDrawer, type DrawerMode } from "../components/AccountDrawer";
import { ActionMenu, type MenuAction } from "../components/AccountGrid";
import { AccountReadiness } from "../components/AccountReadiness";
import { DiscoverPanel } from "../components/DiscoverPanel";
import { ConfirmModal } from "../components/ConfirmModal";
import { GettingStarted, type StartStep } from "../components/GettingStarted";
import { LaunchCommandsDrawer } from "../components/LaunchCommandsDrawer";
import { PageError, PageLoading } from "../components/PageState";
import { QuotaCells } from "../components/QuotaCells";
import { SearchInput, matchesKeyword } from "../components/SearchInput";
import { buildAccounts, type Account, type AgentAccounts } from "../lib/accounts";
import { sortByQuota, type QuotaSort } from "../lib/agentUsage";
import type { ProfileObserved, StatusData } from "../lib/contract";
import { runEngine } from "../lib/engine";
import { secretSourceOf } from "../lib/secretSource";
import { currentConfigOf } from "../lib/agentCurrent";
import type { SharedStatus } from "../lib/profileShared";
import { useAccountOps } from "../lib/useAccountOps";
import type { OfficialAuthStatus } from "../lib/observed";
import { authShellLabel, defaultAuthShell, type AuthShell } from "../lib/profileDefinition";
import { accountCommand } from "../lib/accountCommand";
import { engineKeys, useEngineQuery, useRefreshUpstream } from "../lib/queries";
import { isCollection } from "../lib/skillCollections";
import { cn } from "../lib/utils";
import { Select } from "../components/Select";

/** 表头与每行同宽的列：额度、操作。 */
const COL_QUOTA = "hidden w-80 shrink-0 md:block";
const COL_ACTIONS = "flex w-28 shrink-0 items-center justify-end gap-1";

const QUOTA_SORTS: { value: QuotaSort; label: string; title: string }[] = [
  { value: "default", label: "默认", title: "按账号登记顺序" },
  { value: "reset", label: "先重置", title: "最吃紧的窗口最先重置的排前面" },
  { value: "left", label: "剩余多", title: "最吃紧的窗口剩得最多的排前面" },
];

async function copyText(text: string, message: string) {
  try {
    await navigator.clipboard.writeText(text);
    toast.success(message);
  } catch {
    toast.error("复制失败，请检查剪贴板权限");
  }
}

async function copyAccountCommand(element: string, operation: "login" | "logout" | "launch", shell: AuthShell) {
  try {
    const command = await accountCommand(element, operation, shell);
    await copyText(command.terminal_command, `已复制${operation === "login" ? "登录" : operation === "logout" ? "退出" : "启动"}命令，到 ${authShellLabel(shell)} 里运行`);
  } catch (error) {
    toast.error(`账号检查未通过：${String(error)}`);
  }
}

/** 账号行尾「⋯」：与技能、MCP 等页同一个菜单（渲染在页面最外层，不被表格裁掉）。 */
function RowMenu({ items }: { items: MenuAction[] }) {
  return <ActionMenu label="更多操作" items={items} />;
}

interface RowActions {
  onLaunchCommands: () => void;
  onEdit: () => void;
  onRemove: () => void;
  onAddLaunch: () => void;
  onAdopt: () => void;
  onManageSkills: () => void;
  /** 本机文件说明不了登录状态时，用官方命令确认一次（官方命令自己会写配置目录，所以只在点击时调用）。 */
  onProbeAuth: () => void;
  onReadiness: () => void;
}

function AccountRow({
  account,
  auth,
  actions,
  now,
  authShell,
}: {
  account: Account;
  group: AgentAccounts;
  auth: OfficialAuthStatus | "pending" | null;
  actions: RowActions;
  now: Date;
  authShell: AuthShell;
}) {
  const id = account.elementId;
  const blocked = Boolean(account.blockedReason || account.mainHasThirdParty || auth !== "pending" && auth && (auth.mixed || auth.isolation_blocked));
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
            : auth?.state === "unknown"
              ? "官方 · 未确认"
              : "官方";
  const typeTitle = auth && auth !== "pending" && auth.account?.org ? `组织：${auth.account.org}` : undefined;
  const badges: { text: string; tone: "warn" | "muted" }[] = [];
  if (!account.isMain && !account.command) badges.push({ text: "没有启动命令", tone: "warn" });
  if (account.drifted) badges.push({ text: "目录被外部改过", tone: "warn" });
  if (account.mainHasThirdParty) badges.push({ text: "主账号含第三方接入", tone: "warn" });
  if (account.blockedReason) badges.push({ text: "登记读不了", tone: "warn" });
  if (auth !== "pending" && auth?.mixed) badges.push({ text: "登录与 API 混用，已阻止操作", tone: "warn" });
  else if (auth !== "pending" && auth?.isolation_blocked) badges.push({ text: "账号检查未通过", tone: "warn" });

  return (
    <li data-testid={`account-row-${id}`} className="flex items-center gap-4 px-4 py-2.5">
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
          <span className="max-w-full truncate font-medium">{account.name}</span>
          {account.isMain && (
            <span className="rounded bg-muted px-1.5 py-0.5 text-[11px] text-muted-foreground">主账号</span>
          )}
          {badges.map((badge) => (
            <span
              key={badge.text}
              className={cn(
                "whitespace-nowrap rounded px-1.5 py-0.5 text-[11px]",
                badge.tone === "warn" ? "bg-warning/15 text-warning" : "bg-muted text-muted-foreground",
              )}
            >
              {badge.text}
            </span>
          ))}
        </div>
        <div className="mt-0.5 flex min-w-0 items-center gap-1.5 text-xs text-muted-foreground">
          {account.command && <code className="max-w-[45%] shrink-0 truncate font-mono">{account.command}</code>}
          {account.command && <span aria-hidden>·</span>}
          <span data-testid={`account-type-${id}`} title={typeTitle} className={cn("truncate", loggedOut && "text-warning")}>
            {typeText}
          </span>
          {account.type === "official" && auth !== "pending" && auth?.state === "unknown" && (
            <button
              type="button"
              data-testid={`account-probe-${id}`}
              title={`${auth.reason ?? "本机文件说明不了登录状态"}。点一下运行官方命令确认（官方命令可能在配置目录里写自己的记录）`}
              onClick={actions.onProbeAuth}
              className="shrink-0 text-primary hover:underline"
            >
              确认
            </button>
          )}
          {account.model && <span className="truncate">· {account.model}</span>}
        </div>
      </div>
      <div className={COL_QUOTA}>
        <QuotaCells usage={account.usage} testId={`account-quota-${id}`} now={now} />
      </div>
      <div className={COL_ACTIONS}>
        {loggedOut ? (
          <button
            type="button"
            data-testid={`account-login-${id}`}
            disabled={blocked}
            onClick={() => void copyAccountCommand(id, "login", authShell)}
            className="inline-flex w-20 items-center justify-center gap-1 rounded-md bg-primary px-2.5 py-1.5 text-xs text-primary-foreground hover:bg-primary/90"
          >
            <LogIn className="h-3.5 w-3.5" aria-hidden /> 登录
          </button>
        ) : (
          <button
            type="button"
            data-testid={`account-copy-${id}`}
            disabled={blocked || auth === "pending"}
            onClick={() => void copyAccountCommand(id, "launch", authShell)}
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
              ? [{ label: "复制登录命令", testId: `account-menu-login-${id}`, disabled: blocked, onSelect: () => void copyAccountCommand(id, "login", authShell) },
                { label: "复制退出命令", testId: `account-menu-logout-${id}`, disabled: blocked, onSelect: () => void copyAccountCommand(id, "logout", authShell) }]
              : []),
            { label: "准备情况", testId: `account-menu-readiness-${id}`, onSelect: actions.onReadiness },
            { label: "技能", testId: `account-menu-skills-${id}`, onSelect: actions.onManageSkills },
            { label: "复制目录路径", testId: `account-menu-dir-${id}`, onSelect: () => void copyText(account.configDir, "已复制目录路径") },
            ...(!account.isMain ? [{ label: "移除", testId: `account-menu-remove-${id}`, onSelect: actions.onRemove }] : []),
          ]}
        />
      </div>
    </li>
  );
}

/** 账号页：每个助手的全部账号并列，一行一个；添加与编辑在右侧抽屉里。 */
export default function AccountsPage({
  onManageSkills,
  onOpenSources,
  onOpenSecrets,
}: {
  onManageSkills?: (accountId: string) => void;
  onOpenSources?: () => void;
  onOpenSecrets?: () => void;
}) {
  const agentsQuery = useEngineQuery<StatusData>("agents:status");
  const refreshUpstream = useRefreshUpstream();
  const skillsQuery = useEngineQuery<StatusData>("skills:status");
  const profilesQuery = useEngineQuery<StatusData>("profiles:status");
  const secretsQuery = useEngineQuery<StatusData>("secrets:status");
  const ops = useAccountOps();
  const queryClient = useQueryClient();
  /** 正在看准备情况的账号；新建成功后自动打开。 */
  const [readiness, setReadiness] = useState<{ id: string; isNew: boolean } | null>(null);
  const aliasSource = ops.aliasSource;
  const [selected, setSelected] = useState<string | null>(null);
  const [drawer, setDrawer] = useState<DrawerMode | null>(null);
  const [removing, setRemoving] = useState<{ id: string; name: string; definition: Record<string, unknown> } | null>(null);
  const [removeBusy, setRemoveBusy] = useState(false);
  const [confirmQuota, setConfirmQuota] = useState(false);
  const [commandsOf, setCommandsOf] = useState<Account | null>(null);
  const [quotaBusy, setQuotaBusy] = useState(false);
  const [keyword, setKeyword] = useState("");
  const [authShell, setAuthShell] = useState<AuthShell>(() => defaultAuthShell());
  const [quotaSort, setQuotaSort] = useState<QuotaSort>("default");
  const [refreshing, setRefreshing] = useState(false);
  const [now, setNow] = useState(() => new Date());
  useEffect(() => {
    // 只更新展示时钟，不运行官方命令或扫描文件。
    const timer = window.setInterval(() => setNow(new Date()), 60_000);
    return () => window.clearInterval(timer);
  }, []);

  const groups =
    agentsQuery.data?.ok
      ? buildAccounts(
          agentsQuery.data.data.elements,
          profilesQuery.data?.ok ? profilesQuery.data.data.elements : [],
          aliasSource.source?.aliases ?? {},
        )
      : [];
  const group = groups.find((item) => item.agentId === selected) ?? groups[0];
  const officialIds = (group?.accounts ?? []).map((a) => a.elementId);
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
        <h1 className="text-lg font-semibold tracking-tight">账号</h1>
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
      onEdit: () => withDefinition(account, (definition) => {
        const observed = profilesQuery.data?.ok
          ? profilesQuery.data.data.elements.find((element) => element.id === account.elementId)?.observed
          : undefined;
        setDrawer({ type: "edit", group, account, definition, observed: observed as ProfileObserved | undefined });
      }),
      onRemove: () => withDefinition(account, (definition) => setRemoving({ id: profileId, name: account.name, definition })),
      onAddLaunch: () => withDefinition(account, (definition) => void ops.addLaunch(profileId, definition, group.bin)),
      onAdopt: () => account.profileBase && void ops.adopt(account.elementId, account.profileBase),
      onManageSkills: () => onManageSkills?.(account.isMain ? group.agentId : profileId),
      onReadiness: () => setReadiness({ id: account.elementId, isNew: false }),
      onProbeAuth: () =>
        void runEngine<OfficialAuthStatus>("agents:auth-status", ["--only", account.elementId, "--probe"])
          .then((envelope) => {
            if (envelope.ok) queryClient.setQueryData(engineKeys.request("agents:auth-status", ["--only", account.elementId]), envelope);
            else toast.error(envelope.errors[0]?.message ?? "确认失败");
          })
          .catch((reason) => toast.error(String(reason))),
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
    if (query.data?.ok) return query.data.data;
    return { element: account.elementId, state: "unknown", logged_in: null, method: null, third_party: account.type === "api", mixed: false, isolation_blocked: true, reason: query.data?.errors[0]?.message ?? "无法读取账号配置，请刷新后检查" };
  };

  const visibleAccounts = sortByQuota(group.accounts, quotaSort, now).filter((account) => {
    const auth = authOf(account);
    const identity = auth && auth !== "pending" ? auth.account : null;
    return matchesKeyword(keyword, [account.name, account.command, account.configDir, account.apiHost, account.model,
      identity?.email, identity?.org, identity?.plan, account.type === "api" ? "API" : "官方"]);
  });

  const refreshAccounts = async () => {
    setRefreshing(true);
    try {
      const results = await Promise.all([
        agentsQuery.refetch(), profilesQuery.refetch(), aliasSource.query.refetch(),
        ...authQueries.map((query) => query.refetch()),
      ]);
      setNow(new Date());
      if (results.some((result) => result.isError || !result.data?.ok)) toast.error("部分账号状态读取失败，请重试");
      else toast.success("已从本机重新读取账号状态");
    } finally {
      setRefreshing(false);
    }
  };

  const mainAccount = group.accounts.find((account) => account.isMain);
  const mainAuth = mainAccount ? authOf(mainAccount) : null;
  const startSteps: StartStep[] = [
    {
      id: "login",
      label: `登录 ${group.label} 主账号`,
      hint: "主账号只用官方登录",
      done: mainAuth !== "pending" && mainAuth?.state === "logged-in",
      action:
        !mainAccount || mainAuth === "pending"
          ? undefined
          : mainAuth?.state === "unknown"
            ? { label: "确认", onClick: actionsOf(mainAccount).onProbeAuth }
            : { label: "复制登录命令", onClick: () => void copyAccountCommand(mainAccount.elementId, "login", authShell) },
    },
    {
      id: "source",
      label: "添加技能来源（可选）",
      hint: "推荐 agent-skills；不用也行，各账号里已有的技能照常用",
      optional: true,
      done: !skillsQuery.data?.ok || (skillsQuery.data.data.elements ?? []).some(isCollection),
      action: onOpenSources ? { label: "去添加", onClick: onOpenSources } : undefined,
    },
  ];

  return (
    <div className="space-y-3">
      <h1 className="text-lg font-semibold tracking-tight">账号</h1>
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs">
        <span>登录命令使用</span>
        <Select label="登录命令使用的终端" testId="accounts-auth-shell" value={authShell}
          onChange={(value) => setAuthShell(value as AuthShell)}
          options={[{ value: "powershell", label: "PowerShell" }, { value: "posix", label: "Bash / Zsh / Git Bash" }]}
          className="rounded-md border bg-background px-2 py-1 text-xs" />
        <span className="text-muted-foreground">
          {authShell === "powershell" ? "在 cmd 中请先输入 powershell，再粘贴登录命令。" : "请在 Bash、Zsh 或 Git Bash 中运行。"}
        </span>
      </div>
      <GettingStarted steps={startSteps} />
      <div className="flex flex-wrap items-center gap-3">
        <div role="tablist" className="inline-flex rounded-lg bg-muted p-1">
          {groups.map((item) => (
            <button
              key={item.agentId}
              type="button"
              role="tab"
              aria-selected={item.agentId === group.agentId}
              data-testid={`accounts-tab-${item.agentId}`}
              onClick={() => { setSelected(item.agentId); setKeyword(""); }}
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
        {(() => {
          // 智能体本身只检测：装了哪个版本、有没有新版本；安装与升级用它自己的方式
          const agent = agentElements.find((element) => element.id === `agents:${group.agentId}`);
          const version = (agent?.observed as { version?: string | null } | undefined)?.version ?? agent?.freshness.current ?? null;
          if (!agent || !version) return null;
          const behind = agent.freshness.state === "behind" && agent.freshness.upstream;
          return (
            <span data-testid={`agent-version-${group.agentId}`} className="flex items-center gap-2 text-xs text-muted-foreground">
              <span>
                {group.label} <span className="font-mono text-foreground">{version}</span>
              </span>
              {behind && (
                <span className="rounded bg-info/10 px-1.5 py-0.5 text-[11px] text-info" title="用它自己的更新方式升级（本工具不管理智能体的安装）">
                  有新版本 {agent.freshness.upstream}
                </span>
              )}
              <button
                type="button"
                data-testid={`agent-check-${group.agentId}`}
                disabled={refreshUpstream.isPending}
                onClick={() => refreshUpstream.mutate(agent.id)}
                className="rounded px-1.5 py-0.5 hover:bg-accent hover:text-foreground disabled:opacity-40"
              >
                检查新版本
              </button>
            </span>
          );
        })()}
        <button
          type="button"
          data-testid="accounts-add"
          onClick={() => setDrawer({ type: "create", group })}
          className="ml-auto inline-flex items-center gap-1 rounded-md bg-primary px-3 py-1.5 text-sm text-primary-foreground hover:bg-primary/90"
        >
          <Plus className="h-4 w-4" aria-hidden /> 添加账号
        </button>
      </div>
      <div className="flex flex-wrap items-center gap-2">
        <SearchInput value={keyword} onChange={setKeyword} placeholder="搜索账号、命令、地址或模型" testId="accounts-search" />
        {keyword.trim() && <span className="text-xs text-muted-foreground">{visibleAccounts.length} / {group.accounts.length} 个账号</span>}
        <div role="group" aria-label="按额度排序" className="inline-flex items-center rounded-lg bg-muted p-0.5 text-xs">
          {QUOTA_SORTS.map((option) => (
            <button
              key={option.value}
              type="button"
              aria-pressed={quotaSort === option.value}
              data-testid={`accounts-sort-${option.value}`}
              title={option.title}
              onClick={() => setQuotaSort(option.value)}
              className={cn(
                "rounded-md px-2.5 py-1",
                quotaSort === option.value ? "bg-background font-medium shadow-sm" : "text-muted-foreground hover:text-foreground",
              )}
            >
              {option.label}
            </button>
          ))}
        </div>
        <button type="button" data-testid="accounts-refresh" disabled={refreshing}
          onClick={() => void refreshAccounts()}
          title="重新读取本机配置、登录记录与额度缓存；不联网、不运行登录命令"
          className="ml-auto inline-flex items-center gap-1 rounded-md border px-2.5 py-1.5 text-xs hover:bg-accent disabled:opacity-40">
          <RefreshCw className={cn("h-3.5 w-3.5", refreshing && "animate-spin")} aria-hidden />
          {refreshing ? "读取中…" : "刷新状态"}
        </button>
      </div>
      {quotaNotLive && (
        <div data-testid="accounts-quota-notice" className="flex items-center gap-2 rounded-md border border-info/30 bg-info/5 px-3 py-2 text-xs">
          <Activity className="h-3.5 w-3.5 text-info" aria-hidden />
          <span>{mainStatusline === "builtin-outdated" ? "状态栏脚本是旧版本：本版本新增的显示还没生效。" : "额度还不是实时的：只在 Claude 自己查询时才更新。"}</span>
          <button
            type="button"
            data-testid="accounts-enable-live-quota"
            onClick={() => setConfirmQuota(true)}
            className="ml-auto rounded border border-primary/30 bg-primary/10 px-2 py-0.5 text-primary hover:bg-primary/15"
          >
            {mainStatusline === "builtin-outdated" ? "更新状态栏" : "开启实时额度"}
          </button>
        </div>
      )}
      {(() => {
        const account = readiness && group.accounts.find((item) => item.elementId === readiness.id);
        if (!account) return null;
        const actions = actionsOf(account);
        return (
          <AccountReadiness
            account={account}
            group={group}
            auth={authOf(account)}
            isNew={readiness!.isNew}
            loginShellLabel={authShellLabel(authShell)}
            onCopyLogin={() => void copyAccountCommand(account.elementId, "login", authShell)}
            onAddLaunch={actions.onAddLaunch}
            onProbeAuth={actions.onProbeAuth}
            onClose={() => setReadiness(null)}
          />
        );
      })()}
      <section aria-label="账号列表" className="overflow-clip rounded-lg border bg-card">
        <div className="flex items-center gap-4 border-b bg-muted px-4 py-2 text-xs text-muted-foreground">
          <span className="min-w-0 flex-1 font-medium text-foreground">账号、启动命令与登录</span>
          <span className={COL_QUOTA}>额度</span>
          <span className={COL_ACTIONS} />
        </div>
        <ul data-testid="accounts-list" className="divide-y">
          {visibleAccounts.map((account) => (
            <AccountRow
              key={account.elementId}
              account={account}
              group={group}
              auth={authOf(account)}
              actions={actionsOf(account)}
              now={now}
              authShell={authShell}
            />
          ))}
        </ul>
        {visibleAccounts.length === 0 && <p className="px-4 py-6 text-center text-sm text-muted-foreground">没有匹配的账号。<button type="button" onClick={() => setKeyword("")} className="ml-1 text-primary hover:underline">清除搜索</button></p>}
      </section>
      {drawer && (
        <AccountDrawer
          mode={drawer}
          authShell={authShell}
          ops={ops}
          secretSource={secretSourceOf(secretsQuery.data?.ok ? secretsQuery.data.data : null)}
          reservedNames={reservedNames}
          onCreated={(profileId) => setReadiness({ id: `profiles:${profileId}`, isNew: true })}
          onOpenSecrets={onOpenSecrets}
          onClose={() => setDrawer(null)}
        />
      )}
      {commandsOf && (
        <LaunchCommandsDrawer
          account={commandsOf}
          group={group}
          aliasSource={aliasSource}
          onClose={() => setCommandsOf(null)}
        />
      )}
      {confirmQuota && mainElement && (
        <ConfirmModal
          title="为全部 Claude 账号开启实时额度"
          description={`把状态栏换成 Agent Manager 的用量状态栏（账号、模型、目录、上下文、提示缓存、额度），它每次刷新时把额度记在本工具的数据目录里，账号页据此显示实时额度。${mainStatusline === "custom" ? "主账号现在的自定义状态栏会被替换。" : ""}先写主账号，再同步给其余 Claude 账号；被外部改过的文件不会覆盖。`}
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
      <DiscoverPanel scope="agents" />
    </div>
  );
}
