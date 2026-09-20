import { useEffect, useState } from "react";
import { toast } from "sonner";
import { Download, Loader2, MoreHorizontal, RefreshCw, SearchX, Sparkles, Terminal, Trash2 } from "lucide-react";
import { AgentUsageBar } from "../components/AgentUsageBar";
import { AliasEditorDialog } from "../components/AliasEditorDialog";
import { ProfileCard } from "../components/ProfileCards";
import { ProfileFormDialog } from "../components/ProfileFormDialog";
import { ConfirmModal } from "../components/ConfirmModal";
import { DefinitionFormDialog } from "../components/DefinitionFormDialog";
import {
  ConnectionFormDialog,
  type ConnectionSubmission,
} from "../components/ConnectionFormDialog";
import {
  ConnectionWorkbench,
  connectionKindOf,
} from "../components/ConnectionWorkbench";
import { useDefinitionOps } from "../lib/useDefinitionOps";
import { EmptyState } from "../components/EmptyState";
import { FirstRunGuide } from "../components/FirstRunGuide";
import { ErrorPanel } from "../components/ErrorPanel";
import { FreshnessBadge } from "../components/FreshnessBadge";
import { PageError, PageHeader, PageLoading } from "../components/PageState";
import { SearchInput, matchesKeyword } from "../components/SearchInput";
import { StateBadge } from "../components/StateBadge";
import type { ElementAction, EngineElement, PolicyData, StatusData } from "../lib/contract";
import { parseAliasValue, renderAliasPreview } from "../lib/aliasCommand";
import { ALIAS_EFFECT_HINT, ALIAS_EFFECT_LABEL, aliasEffectOf, aliasElementsOf, type AliasEffect } from "../lib/aliasEffect";
import {
  buildProfileDefinition,
  defaultConfigDir,
  fixLaunchCommandTilde,
  launchCommandsForProfile,
  parseProfileDefinition,
  profileKindOf,
  profileLaunchValue,
  type ProfileFormInput,
} from "../lib/profileDefinition";
import { runEngine } from "../lib/engine";
import { toastWithUndo } from "../lib/undo";
import { definitionOrigin } from "../lib/definitionOverlay";
import {
  activeConnectionOf,
  connectionsOf,
  hasAgentSettings,
  removeAgentConnection,
  upsertAgentConnection,
  type ConnectionEntry,
} from "../lib/agentDefinitionForm";
import type { AgentsObserved, AgentsPolicy, AgentsSpec } from "../lib/observed";
import { useEngineQuery, useRefreshUpstream } from "../lib/queries";
import { COPY, versionRangeHint } from "../lib/ui-copy";
import { useAliasSource } from "../lib/useAliasSource";
import { useDirectApply } from "../lib/useDirectApply";
import { cn } from "../lib/utils";

/** agents 动作按钮文案/样式（离线可用性由 actions[].offline_ok 驱动，引擎是事实源）。 */
const ACTION_META: Record<string, { label: string; icon: typeof Download; className: string }> = {
  install: { label: "安装", icon: Download, className: "border-primary/30 text-primary hover:bg-primary/10" },
  upgrade: { label: "升级", icon: RefreshCw, className: "border-warning/30 text-warning hover:bg-warning/10" },
  uninstall: { label: "卸载", icon: Trash2, className: "border-destructive/30 text-destructive hover:bg-destructive/10" },
};

interface CardMenuItem {
  label: string;
  testId?: string;
  danger?: boolean;
  disabled?: boolean;
  onSelect: () => void;
}

/** 卡片上的「更多」：停用、移除管理、卸载这类低频动作收在这里，主行只留常用的。 */
function CardMenu({ items, testId }: { items: CardMenuItem[]; testId?: string }) {
  const [open, setOpen] = useState(false);
  useEffect(() => {
    if (!open) return;
    // 点别处就收起：菜单自身的点击在容器上已阻止冒泡
    const close = () => setOpen(false);
    window.addEventListener("click", close);
    return () => window.removeEventListener("click", close);
  }, [open]);
  return (
    <div className="relative" onClick={(event) => event.stopPropagation()}>
      <button
        type="button"
        data-testid={testId}
        aria-label="更多操作"
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen((value) => !value)}
        className="inline-flex items-center rounded border px-2.5 py-1 text-xs text-muted-foreground hover:bg-accent"
      >
        <MoreHorizontal className="h-3.5 w-3.5" aria-hidden />
      </button>
      {open && (
        <div role="menu" className="absolute right-0 z-10 mt-1 min-w-36 rounded-md border bg-card p-1 shadow-md">
          {items.map((item) => (
            <button
              key={item.label}
              type="button"
              role="menuitem"
              data-testid={item.testId}
              disabled={item.disabled}
              onClick={() => {
                setOpen(false);
                item.onSelect();
              }}
              className={cn(
                "block w-full rounded px-2 py-1.5 text-left text-xs hover:bg-accent disabled:opacity-50",
                item.danger ? "text-destructive" : "text-foreground",
              )}
            >
              {item.label}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

function AgentCard({
  element,
  customDefinition,
  runningActionId,
  onEdit,
  onRemove,
  onRequestUninstall,
  onRequestAction,
  onManageSkills,
  aliasValue,
  aliasPreview,
  onEditAlias,
  onSyncAliases,
  aliasSyncing,
  aliasEffect,
  policyBusy,
  onToggleEnabled,
}: {
  element: EngineElement;
  /** true = 定义来自本地数据库（用户新增或改过内置定义）：安装/升级先弹确认。 */
  customDefinition: boolean;
  /** 页面级确认路径（卸载/自定义定义安装升级）在途的动作 id；确认弹窗关闭后卡片按钮保持执行态。 */
  runningActionId: string | null;
  onEdit: () => void;
  onRemove: () => void;
  onRequestUninstall: () => void;
  onRequestAction: (action: ElementAction) => void;
  onManageSkills?: (agentId: string) => void;
  /** 该助手当前的启动命令取值（终端快捷命令来源里与其可执行名同名的一条）；没有则为 null。 */
  aliasValue: string | null;
  /** 同步后实际写入终端配置的那一行（已按全局设置渲染）。 */
  aliasPreview: string | null;
  onEditAlias: () => void;
  onSyncAliases: () => void;
  aliasSyncing: boolean;
  /** 该命令在终端配置里的落地情况（两个目标都跟上才算已写入）。 */
  aliasEffect: AliasEffect;
  /** 偏好写入在途：期间禁用菜单里的启停项，避免连点写出交错结果。 */
  policyBusy: boolean;
  onToggleEnabled: (next: boolean) => void;
}) {
  /** 本卡直执路径在途的动作 id（点击即置位，apply 结束回落）。 */
  const [localActionId, setLocalActionId] = useState<string | null>(null);
  const directApply = useDirectApply();
  const spec = element.spec as AgentsSpec;
  const policy = element.policy as AgentsPolicy;
  const observed = element.observed as AgentsObserved;
  const installed = observed.installed ?? false;
  const version = installed ? (element.freshness.current ?? "已安装（还没检查过版本）") : "未安装";
  const name = spec.display_name ?? element.id;
  const busyActionId = localActionId ?? runningActionId;

  /** 显式动作点击直接 agents:apply（带精确 --only/--action）；卸载先经独立 ConfirmModal，
   * 自定义定义的安装/升级也先经 ConfirmModal（命令内容由用户定义提供）。
   * 升级 label 携带版本对照（当前 → 最新），点击后按钮立即进入执行态直到 apply 结束。 */
  const runAction = (action: ElementAction) => {
    if (busyActionId) return;
    if (action.id === "uninstall") {
      onRequestUninstall();
      return;
    }
    if (customDefinition) {
      onRequestAction(action);
      return;
    }
    const label = ACTION_META[action.id]?.label ?? action.id;
    const applyLabel =
      action.id === "upgrade" ? `${label} ${name}（${versionRangeHint(element.freshness)}）` : `${label} ${name}`;
    setLocalActionId(action.id);
    void directApply
      .apply({
        command: "agents:apply",
        label: applyLabel,
        args: ["--only", element.id, "--action", action.id],
      })
      .finally(() => setLocalActionId(null));
  };

  return (
    <section className="rounded-lg border bg-card p-5 shadow-sm transition-shadow hover:shadow-md">
      <div className="flex items-start justify-between gap-3">
        <div>
          <h3 className="text-base font-semibold text-foreground">
            {name}
            <span className="ml-2 font-mono text-xs font-normal text-muted-foreground">{spec.bin}</span>
          </h3>
          <p className="mt-0.5 text-sm text-muted-foreground">当前版本:{version}</p>
        </div>
        <div className="flex flex-col items-end gap-1.5">
          {/* 「正常」对装好的助手是废话；已停用下面另有整句说明，不在这里重复 */}
          {element.state !== "managed-ok" && element.state !== "disabled" && (
            <StateBadge state={element.state} />
          )}
          <FreshnessBadge freshness={element.freshness} className="items-end" />
        </div>
      </div>

      {/* 额度紧跟版本：这两件事是打开这页最常来看的 */}
      {observed.usage && (
        <div className="mt-3 rounded-md border px-3 py-2">
          <AgentUsageBar usage={observed.usage} testId={`agent-usage-${element.id}`} />
        </div>
      )}

      {policy.enabled === false && (
        <p className="mt-3 rounded-md border bg-muted px-3 py-2 text-xs text-muted-foreground">
          已停用：不再为它维护技能、MCP 等资源；已安装的软件不受影响。
        </p>
      )}

      {element.state === "unmanaged-detected" && (
        <div className="mt-3 rounded-md border border-warning/30 bg-warning/10 px-3 py-2 text-xs text-warning">
          <p>
            检测到用 {observed.install_source ?? "其它渠道"} 安装的版本，该来源不在本工具管理范围内。
            当前不支持自动迁移安装方式；如需由本工具管理，请先自行卸载现有安装，再使用「安装」。
          </p>
        </div>
      )}

      <div className="mt-3 border-t pt-3">
        <div className="flex items-center justify-between gap-2">
          <span className="flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
            <Terminal className="h-3.5 w-3.5" aria-hidden /> 启动命令
            {aliasValue && (
              <span
                data-testid={`alias-effect-${element.id}`}
                title={ALIAS_EFFECT_HINT[aliasEffect]}
                className={cn(
                  "rounded border px-1.5 py-0.5 text-[10px] font-normal",
                  aliasEffect === "applied"
                    ? "border-success/30 bg-success/10 text-success"
                    : aliasEffect === "unknown"
                      ? "border bg-muted text-muted-foreground"
                      : "border-warning/30 bg-warning/10 text-warning",
                )}
              >
                {ALIAS_EFFECT_LABEL[aliasEffect]}
              </span>
            )}
          </span>
          <div className="flex items-center gap-2">
            <button
              type="button"
              data-testid={`edit-alias-${element.id}`}
              onClick={onEditAlias}
              className="rounded border px-2.5 py-1 text-xs text-muted-foreground hover:bg-accent"
            >
              {aliasValue ? "修改" : "设置"}
            </button>
            {aliasValue && (
              <button
                type="button"
                data-testid={`sync-alias-${element.id}`}
                disabled={aliasSyncing}
                onClick={onSyncAliases}
                className="rounded border border-primary/30 bg-primary/10 px-2.5 py-1 text-xs text-primary hover:bg-primary/15 disabled:opacity-50"
              >
                {aliasSyncing ? "同步中…" : "同步到终端"}
              </button>
            )}
          </div>
        </div>
        {/* 命令是一整行、按空格才有意义，break-all 会从 LC_ALL 中间断开很难读；
            这里改为横向滚动，读的时候一眼能看出词边界。 */}
        <p
          data-testid={`alias-preview-${element.id}`}
          title={aliasPreview ?? undefined}
          className={cn(
            "mt-1.5 font-mono text-[11px] text-muted-foreground",
            aliasPreview ? "overflow-x-auto whitespace-pre" : "",
          )}
        >
          {aliasPreview ?? `还没设置：终端里直接执行 ${spec.bin}，不带固定时区与参数`}
        </p>
      </div>

      <div className="mt-3 flex flex-wrap items-center gap-2 border-t pt-3">
        <button type="button" onClick={onEdit} className="rounded border px-2.5 py-1 text-xs">管理助手</button>
        <button
          type="button"
          data-testid={`manage-skills-${element.id}`}
          onClick={() => onManageSkills?.(element.id.replace(/^agents:/, ""))}
          className="inline-flex items-center gap-1 rounded border border-primary/30 bg-primary/10 px-2.5 py-1 text-xs text-primary hover:bg-primary/15"
        >
          <Sparkles className="h-3 w-3" aria-hidden /> 管理技能
        </button>
        {element.actions.map((action) => {
          const meta = ACTION_META[action.id];
          // 卸载收在「更多」里：破坏性且低频，不占主行；执行中回到主行，让进度看得见
          if (!meta || (action.id === "uninstall" && busyActionId !== "uninstall")) return null;
          const Icon = meta.icon;
          const busy = busyActionId === action.id;
          /** 任一动作在途时整卡动作禁用（防并发触发），在途按钮自身转 spinner + 执行中文案。 */
          const disabled = busyActionId !== null;
          const execHint = `${meta.label}${action.id === "upgrade" ? `（${versionRangeHint(element.freshness)}）` : ""}：${
            customDefinition ? "先弹确认再执行" : "直接执行"
          }，命令以你的权限运行，结果不能自动撤销`;
          const title = busy ? COPY.executing : execHint;
          return (
            <button
              key={action.id}
              type="button"
              data-testid={`action-${action.id}-${element.id}`}
              disabled={disabled}
              title={title}
              onClick={() => runAction(action)}
              className={cn(
                "inline-flex items-center gap-1 rounded border px-2.5 py-1 text-xs",
                busy ? meta.className : disabled ? "cursor-not-allowed border text-muted-foreground" : meta.className,
              )}
            >
              {busy ? <Loader2 className="h-3 w-3 animate-spin" aria-hidden /> : <Icon className="h-3 w-3" aria-hidden />}
              {busy ? COPY.executing : meta.label}
            </button>
          );
        })}
        <div className="ml-auto">
          <CardMenu
            testId={`agent-menu-${element.id}`}
            items={[
              {
                label: policy.enabled === false ? "启用" : "停用",
                testId: `policy-enabled-${element.id}`,
                disabled: policyBusy,
                onSelect: () => onToggleEnabled(policy.enabled === false),
              },
              ...(element.actions.some((action) => action.id === "uninstall")
                ? [
                    {
                      label: "卸载软件",
                      testId: `menu-uninstall-${element.id}`,
                      danger: true,
                      disabled: busyActionId !== null,
                      onSelect: onRequestUninstall,
                    },
                  ]
                : []),
              { label: "移除管理", testId: `menu-remove-${element.id}`, danger: true, onSelect: onRemove },
            ]}
          />
        </div>
      </div>
    </section>
  );
}

export default function AgentsPage({
  onManageSkills,
  onGoTakeover,
}: {
  onManageSkills?: (agentId: string) => void;
  onGoTakeover?: () => void;
} = {}) {
  const query = useEngineQuery<StatusData>("agents:status");
  const policyQuery = useEngineQuery<PolicyData>("policy:get");
  const definitionOps = useDefinitionOps({ section: "agents", domain: "agents" });
  const directApply = useDirectApply();
  // 启动命令与「终端快捷命令」页共用同一份来源：卡片按助手的可执行名取对应那一条
  const aliasSource = useAliasSource();
  // 与助手动作分开的执行实例：两者互不占用彼此的在途状态与按钮文案
  const aliasApply = useDirectApply();
  // 终端配置里的实际落地情况：卡片据此显示是否已写入
  const aliasStatusQuery = useEngineQuery<StatusData>("aliases:status");
  // 实例（同一助手的多套独立配置）：定义与 agents 共用，状态与写入走 profiles 域
  const profilesQuery = useEngineQuery<StatusData>("profiles:status");
  const profileApply = useDirectApply();
  const [profileForm, setProfileForm] = useState<ProfileFormInput | "new" | null>(null);
  /** 复制实例：表单带着预填内容，但名称与配置目录仍要能改（否则两个实例指同一套配置）。 */
  const [profileDuplicating, setProfileDuplicating] = useState(false);
  const [profileRemoving, setProfileRemoving] = useState<string | null>(null);
  const [policyBusy, setPolicyBusy] = useState(false);
  /** 日常接入工作台当前查看的主助手；null 时跟随列表里的第一个。 */
  const [selectedAgentId, setSelectedAgentId] = useState<string | null>(null);
  const [connectionForm, setConnectionForm] = useState<{
    agent: EngineElement;
    initial?: ConnectionEntry;
  } | null>(null);
  const [connectionSaving, setConnectionSaving] = useState(false);
  const [connectionFormError, setConnectionFormError] = useState<string | null>(null);
  const [connectionRemoving, setConnectionRemoving] = useState<{
    agent: EngineElement;
    connection: ConnectionEntry;
  } | null>(null);
  const [connectionRemovingBusy, setConnectionRemovingBusy] = useState(false);
  /** 接入配置的检查结果：按「助手/配置名」存，卡片与弹窗都读它。 */
  const [checkResults, setCheckResults] = useState<
    Record<string, { ok: boolean; detail: string; latencyMs: number; pending?: boolean }>
  >({});

  const [aliasEditing, setAliasEditing] = useState<
    { bin: string; name: string; value: string | null; presetName?: string } | null
  >(null);
  const [launchRemoving, setLaunchRemoving] = useState<string | null>(null);
  const [form, setForm] = useState<EngineElement | "new" | null>(null);
  const [pendingUninstall, setPendingUninstall] = useState<EngineElement | null>(null);
  const [pendingAction, setPendingAction] = useState<{ element: EngineElement; action: ElementAction } | null>(null);
  /** 运行参数写入前要用户点头的那一步：降安全等级，或目标被外部改过。 */
  const [pendingConfigure, setPendingConfigure] = useState<
    { agentId: string; yolo: boolean; use?: string; reason: "yolo" | "takeover" } | null
  >(null);
  /** 正在写入哪一个助手/接入；非空时禁止并发切换。 */
  const [configuringTarget, setConfiguringTarget] = useState<string | null>(null);
  const configuring = configuringTarget !== null;
  /** 确认弹窗路径（卸载/自定义定义安装升级）在途动作：弹窗确认即关闭，卡片按钮据此保持执行态直到 apply 结束。 */
  const [runningAction, setRunningAction] = useState<{ elementId: string; actionId: string } | null>(null);
  const [search, setSearch] = useState("");
  // 版本对比要先探一次才有数：探测只在点它时联网，status 一律零网络
  const refreshUpstream = useRefreshUpstream();
  if (query.isPending) return <PageLoading />;
  if (query.isError) return <PageError title="agents:status 调用失败" error={query.error} />;

  const elements = query.data.data.elements;
  const filteredElements = elements.filter((element) => {
    const spec = element.spec as AgentsSpec;
    return matchesKeyword(search, [spec.display_name, element.id, spec.bin]);
  });
  const layers = policyQuery.data?.data.layers ?? null;
  /** 本地层为空 = 还没在这里保存过任何东西：首次使用引导据此出现（policyQuery 未就绪时不显示）。 */
  const freshInstall =
    policyQuery.data?.ok === true && Object.keys((layers?.sqlite ?? {}) as Record<string, unknown>).length === 0;
  /** 内置判定：policy 层已加载且本地数据库无该定义覆盖；未加载到时按自定义处理（先确认再执行）。 */
  const isCustomDefinition = (element: EngineElement) =>
    layers === null || definitionOrigin(layers, "agents", element.id.replace(/^agents:/, "")) === "sqlite";
  const profileElements = profilesQuery.data?.ok ? profilesQuery.data.data.elements : [];
  const profileIds = new Set(profileElements.map((element) => element.id.replace(/^profiles:/, "")));
  /** 主助手（非实例）：只有它们参与安装渠道展示与「新建实例」。 */
  const allPrimaryElements = elements.filter((element) => !profileIds.has(element.id.replace(/^agents:/, "")));
  const primaryElements = filteredElements.filter((element) => !profileIds.has(element.id.replace(/^agents:/, "")));
  const effectiveSelectedAgentId =
    selectedAgentId && allPrimaryElements.some((element) => element.id === selectedAgentId)
      ? selectedAgentId
      : allPrimaryElements[0]?.id ?? null;
  /** 可作为实例归属的助手：目前只有 Claude Code 与 Codex 有隔离配置目录的官方支持。 */
  const profileCapableAgents = elements
    .map((element) => ({ id: element.id.replace(/^agents:/, ""), spec: element.spec as AgentsSpec }))
    .filter((item) => !profileIds.has(item.id) && ["claude-code", "codex"].includes(item.id))
    .map((item) => ({ id: item.id, displayName: item.spec.display_name ?? item.id }));
  const baseBinOf = (agentId: string) =>
    (elements.find((element) => element.id === `agents:${agentId}`)?.spec as AgentsSpec | undefined)?.bin ??
    (agentId === "codex" ? "codex" : "claude");

  /**
   * 把运行参数写进助手自己的配置文件。
   *
   * 两处需要用户点头，都由引擎强制、界面只负责问：降安全等级（--allow-yolo）、
   * 目标里我们那几项被外部改过（--allow-takeover）。引擎拒绝时一个字节都不会动。
   */
  const runConfigure = async (
    agentId: string,
    opts: { allowYolo?: boolean; allowTakeover?: boolean; use?: string } = {},
  ): Promise<boolean> => {
    const args = ["--only", `agents:${agentId}`];
    // 切换与写入是一条命令：库里的生效项和目标文件要么一起变，要么都不变
    if (opts.use !== undefined) args.push(...(opts.use ? ["--use", opts.use] : ["--use-default"]));
    if (opts.allowYolo) args.push("--allow-yolo");
    if (opts.allowTakeover) args.push("--allow-takeover");
    const target = opts.use !== undefined ? `${agentId}/${opts.use || "default"}` : `${agentId}/settings`;
    setConfiguringTarget(target);
    try {
      const envelope = await runEngine("agents:configure", args);
      if (envelope.ok) {
        // 以 status 的真实回读更新界面，再宣布成功；不靠本地乐观改状态。
        await query.refetch();
        toast.success(opts.use !== undefined ? `已切换 ${agentId} 的接入配置` : `已写入 ${agentId} 的运行参数`, {
          description: "助手下次启动生效；已经开着的要重启它。",
        });
        return true;
      }
      const message = envelope.errors[0]?.message ?? "未知错误";
      // 外部改过：让用户看清再决定要不要用这里的内容替换
      if (!opts.allowTakeover && message.includes("拒绝覆盖")) {
        setPendingConfigure({ agentId, yolo: opts.allowYolo === true, use: opts.use, reason: "takeover" });
        return false;
      }
      toast.error(`运行参数没写进去:${message}`, {
        description: "定义已经存下了，助手的配置文件没动。",
      });
      return false;
    } catch (err) {
      toast.error(`运行参数没写进去:${String(err)}`);
      return false;
    } finally {
      setConfiguringTarget(null);
    }
  };

  /**
   * 测一下这套接入配置：发一个最小真实请求，确认地址、令牌与模型名都对。
   *
   * 这是全局唯一会主动联网的助手侧动作，只在点它时才发；只读，不碰配置文件，
   * 也不改当前生效的那套。
   */
  const runCheck = async (agentId: string, profile: string): Promise<void> => {
    const key = `${agentId}/${profile}`;
    setCheckResults((prev) => ({ ...prev, [key]: { ok: false, detail: "正在测…", latencyMs: 0, pending: true } }));
    try {
      const envelope = await runEngine<{ ok?: boolean; detail?: string; latency_ms?: number }>(
        "agents:check",
        ["--only", `agents:${agentId}`, "--use", profile],
      );
      const detail = envelope.ok
        ? envelope.data?.detail ?? "测完了"
        : envelope.errors[0]?.message ?? "没测成";
      setCheckResults((prev) => ({
        ...prev,
        [key]: {
          ok: envelope.ok === true && envelope.data?.ok === true,
          detail,
          latencyMs: envelope.data?.latency_ms ?? 0,
        },
      }));
    } catch (err) {
      setCheckResults((prev) => ({
        ...prev,
        [key]: { ok: false, detail: String(err), latencyMs: 0 },
      }));
    }
  };

  /**
   * 聚焦式新增接入：先保存完整助手定义，再按需保存 Claude 凭据，最后才执行启用。
   * 后一步失败时不冒充原子成功；已保存的定义仍会回读到卡片，用户可直接重试。
   */
  const saveConnection = async (element: EngineElement, submission: ConnectionSubmission) => {
    if (connectionSaving) return;
    const agentId = element.id.replace(/^agents:/, "");
    setConnectionSaving(true);
    setConnectionFormError(null);
    try {
      const definition = upsertAgentConnection(
        element.spec as Record<string, unknown>,
        submission.connection,
      );
      const saved = await definitionOps.saveDefinition({ name: agentId, definition, isNew: false });
      if (!saved) {
        setConnectionFormError("接入没有保存，请根据提示修正后重试");
        return;
      }

      if (submission.secretValue) {
        const secret = await runEngine("secrets:set", [
          "--var",
          submission.connection.tokenVar,
          "--value",
          submission.secretValue,
        ]);
        if (!secret.ok) {
          await query.refetch();
          setConnectionFormError(
            `接入已保存，但凭据没有存下：${secret.errors[0]?.message ?? "请修正后重试，或从外部环境提供这个变量"}`,
          );
          return;
        }
      }

      setConnectionForm(null);
      await query.refetch();
      if (submission.activate) {
        await runConfigure(agentId, { use: submission.connection.name });
      }
    } catch (err) {
      setConnectionFormError(`保存接入失败:${String(err)}`);
    } finally {
      setConnectionSaving(false);
    }
  };

  /** 删除非当前接入：只删定义里的目标 profile，不清理可能被其他接入复用的凭据变量。 */
  const removeConnection = async (element: EngineElement, connection: ConnectionEntry) => {
    if (connectionRemovingBusy) return;
    const definition = element.spec as Record<string, unknown>;
    if (activeConnectionOf(definition) === connection.name) {
      toast.error("当前接入不能删除，请先切换到其他接入或助手默认账号");
      return;
    }
    const agentId = element.id.replace(/^agents:/, "");
    setConnectionRemovingBusy(true);
    try {
      const saved = await definitionOps.saveDefinition({
        name: agentId,
        definition: removeAgentConnection(definition, connection.name),
        isNew: false,
      });
      if (!saved) return;
      setConnectionRemoving(null);
      await query.refetch();
      toast.success(`已删除接入 ${connection.label || connection.name}`, {
        description: "凭据变量没有删除；仍在使用时不会受影响，可在密钥页单独处理。",
      });
    } finally {
      setConnectionRemovingBusy(false);
    }
  };

  /** 启停助手：只写本机配置，不触发安装、卸载或同步。 */
  const toggleEnabled = async (agentId: string, next: boolean) => {
    if (policyBusy) return;
    setPolicyBusy(true);
    try {
      const envelope = await runEngine("policy:set", [
        "--layer",
        "local",
        "--key",
        `agents.${agentId}.enabled`,
        "--value",
        JSON.stringify(next),
      ]);
      if (!envelope.ok) {
        toast.error(`保存偏好被拒绝:${envelope.errors[0]?.message ?? "未知错误"}`);
        return;
      }
      toast.success(next ? "已启用" : "已停用", {
        description: next
          ? "重新为它维护技能、MCP 等资源。"
          : "不再为它维护技能、MCP 等资源；已安装的软件不受影响。",
      });
      void query.refetch();
    } catch (err) {
      toast.error(`保存偏好失败:${String(err)}`);
    } finally {
      setPolicyBusy(false);
    }
  };

  /** 保存实例：写定义 + 把启动命令存成同名快捷命令；两步都只写配置，不落地。 */
  const saveProfile = async (input: ProfileFormInput) => {
    const definition = buildProfileDefinition(input, baseBinOf(input.baseAgent));
    let envelope;
    try {
      envelope = await runEngine("policy:set", [
        "--layer",
        "local",
        "--key",
        `agents.definitions.${input.id}`,
        "--value",
        JSON.stringify({ ...definition, _delete: false, _replace: true }),
      ]);
    } catch (err) {
      toast.error(`保存实例失败:${String(err)}`);
      return;
    }
    if (!envelope.ok) {
      toast.error(`保存实例被拒绝:${envelope.errors[0]?.message ?? "未知错误"}`);
      return;
    }
    if (aliasSource.source) {
      await aliasSource.save(
        {
          aliases: {
            ...aliasSource.source.aliases,
            [input.id]: profileLaunchValue(input, baseBinOf(input.baseAgent)),
          },
        },
        `已保存实例 ${input.id} 与它的启动命令`,
      );
    } else {
      toast.success(`已保存实例 ${input.id}`);
    }
    setProfileForm(null);
    setProfileDuplicating(false);
    void profilesQuery.refetch();
  };

  /** 移除实例定义：只停止管理这套配置，配置目录与里面的内容原样保留。 */
  const removeProfile = async (id: string) => {
    const snapshot = (policyQuery.data?.ok
      ? ((policyQuery.data.data.layers?.sqlite as Record<string, any> | null)?.agents?.definitions?.[id] ?? null)
      : null) as Record<string, unknown> | null;
    const removedAlias = aliasSource.source?.aliases[id];
    let envelope;
    try {
      envelope = await runEngine("policy:set", [
        "--layer",
        "local",
        "--key",
        `agents.definitions.${id}`,
        "--value",
        JSON.stringify({ _delete: true }),
      ]);
    } catch (err) {
      toast.error(`移除实例失败:${String(err)}`);
      return;
    }
    if (!envelope.ok) {
      toast.error(`移除实例被拒绝:${envelope.errors[0]?.message ?? "未知错误"}`);
      return;
    }
    if (aliasSource.source && aliasSource.source.aliases[id]) {
      const next = { ...aliasSource.source.aliases };
      delete next[id];
      await aliasSource.save({ aliases: next }, `已移除实例 ${id}`, { silent: true });
    }
    setProfileRemoving(null);
    void profilesQuery.refetch();
    if (snapshot) {
      toastWithUndo(`已移除实例 ${id}`, {
        description: "配置目录原样保留；撤销会把实例定义与启动命令加回来。",
        onUndo: async () => {
          const restored = await runEngine("policy:set", [
            "--layer",
            "local",
            "--key",
            `agents.definitions.${id}`,
            "--value",
            JSON.stringify({ ...snapshot, _delete: false, _replace: true }),
          ]);
          if (!restored.ok) {
            toast.error(`恢复实例被拒绝:${restored.errors[0]?.message ?? "未知错误"}`);
            return;
          }
          if (removedAlias && aliasSource.source) {
            await aliasSource.save(
              { aliases: { ...aliasSource.source.aliases, [id]: removedAlias } },
              `已恢复实例 ${id}`,
              { silent: true },
            );
          }
          toast.success(`已恢复实例 ${id}`);
          void profilesQuery.refetch();
        },
      });
    } else {
      toast.success(`已移除实例 ${id}`);
    }
  };

  /** 助手的启动命令：来源里与其可执行名同名的那一条；来源还没读到时为 null。 */
  const aliasOf = (element: EngineElement): string | null => {
    const bin = (element.spec as AgentsSpec).bin;
    if (!bin || !aliasSource.source) return null;
    return aliasSource.source.aliases[bin] ?? null;
  };
  const aliasPreviewOf = (element: EngineElement): string | null => {
    const value = aliasOf(element);
    const bin = (element.spec as AgentsSpec).bin;
    if (!value || !bin || !aliasSource.source) return null;
    return renderAliasPreview(bin, value, { timezone: aliasSource.source.timezone, lang: aliasSource.source.lang });
  };
  /** 一个实例可以挂多条启动命令：按命令里的配置目录变量反查，别名仍只有别名域一个写者。 */
  const launchCommandsOf = (element: EngineElement) => {
    if (!aliasSource.source) return [];
    const spec = element.spec as unknown as { base_agent?: string; config_dir?: string };
    const observed = element.observed as unknown as { config_dir?: string };
    const dir = observed.config_dir ?? spec.config_dir ?? "";
    if (!dir) return [];
    return launchCommandsForProfile(
      aliasSource.source.aliases,
      profileKindOf(spec.base_agent ?? "claude-code"),
      dir,
      (value) => parseAliasValue(value).otherEnv,
    );
  };

  /** 保存一条启动命令；改了名字即重命名，旧名那条一并去掉。 */
  const saveLaunchAlias = async (name: string, value: string, renamedFrom?: string) => {
    if (!aliasSource.source) return false;
    const aliases = { ...aliasSource.source.aliases, [name]: value };
    if (renamedFrom && renamedFrom !== name) delete aliases[renamedFrom];
    return aliasSource.save(
      { aliases },
      renamedFrom && renamedFrom !== name
        ? `已改名为 ${name}，点「同步启动命令」写入配置`
        : `已保存 ${name}，点「同步启动命令」写入配置`,
    );
  };

  const removeLaunchAlias = async (name: string) => {
    if (!aliasSource.source) return;
    const removed = aliasSource.source.aliases[name];
    const aliases = { ...aliasSource.source.aliases };
    delete aliases[name];
    const ok = await aliasSource.save({ aliases }, `已删除 ${name}`, { silent: true });
    if (!ok) return;
    setLaunchRemoving(null);
    toastWithUndo(`已删除启动命令 ${name}`, {
      description: "还没同步到终端配置；撤销会把它加回来。",
      onUndo: () => void aliasSource.save({ aliases: { ...aliases, [name]: removed } }, `已恢复 ${name}`),
    });
  };

  /** 复制一条：名字自动避让已有条目，内容照搬等着改。 */
  const duplicateLaunchAlias = (name: string, bin: string) => {
    const existing = aliasSource.source?.aliases ?? {};
    let candidate = `${name}-copy`;
    let n = 2;
    while (existing[candidate] !== undefined) {
      candidate = `${name}-copy${n}`;
      n += 1;
    }
    setAliasEditing({ bin, name: "", value: existing[name] ?? null, presetName: candidate });
  };

  const uninstallElement = pendingUninstall;
  const uninstallName = uninstallElement ? (uninstallElement.spec as AgentsSpec).display_name ?? uninstallElement.id : "";
  const pendingActionName = pendingAction
    ? (pendingAction.element.spec as AgentsSpec).display_name ?? pendingAction.element.id
    : "";
  const pendingActionLabel = pendingAction ? (ACTION_META[pendingAction.action.id]?.label ?? pendingAction.action.id) : "";
  return (
    <div className="space-y-4">
      <PageHeader title="助手" description="先选择助手和接入，再处理安装、版本与启动命令。" />
      <FirstRunGuide show={freshInstall} onGoTakeover={onGoTakeover} />
      {query.data.errors.length > 0 && <ErrorPanel title="agents:status 错误" errors={query.data.errors} />}
      <ConnectionWorkbench
        agents={allPrimaryElements}
        selectedAgentId={effectiveSelectedAgentId}
        busyTarget={configuringTarget}
        checkResults={checkResults}
        onSelectAgent={setSelectedAgentId}
        onAdd={(element) => {
          setConnectionFormError(null);
          setConnectionForm({ agent: element });
        }}
        onSwitch={(element, connection) =>
          void runConfigure(element.id.replace(/^agents:/, ""), { use: connection })
        }
        onCheck={(element, connection) =>
          void runCheck(element.id.replace(/^agents:/, ""), connection)
        }
        onEdit={(element, connection) => {
          setConnectionFormError(null);
          setConnectionForm({ agent: element, initial: connection });
        }}
        onRemove={(element, connection) => setConnectionRemoving({ agent: element, connection })}
      />
      <div className="pt-2">
        <h2 className="text-base font-semibold text-foreground">安装与维护</h2>
        <p className="mt-0.5 text-sm text-muted-foreground">
          低频操作集中在这里：版本、启动命令、技能与助手定义。
        </p>
      </div>
      <div className="flex items-center justify-between gap-3">
        <SearchInput value={search} onChange={setSearch} placeholder="搜索助手…" testId="agents-search" />
        <div className="flex shrink-0 items-center gap-2">
          <button
            type="button"
            data-testid="agents-check-updates"
            disabled={refreshUpstream.isPending}
            onClick={() => refreshUpstream.mutate(undefined)}
            title="去各自的发布源查一次最新版本（这一下会联网）"
            className="inline-flex items-center gap-1.5 rounded-md border px-3 py-2 text-sm text-muted-foreground hover:bg-accent disabled:opacity-60"
          >
            <RefreshCw className={cn("h-4 w-4", refreshUpstream.isPending && "animate-spin")} aria-hidden />
            {refreshUpstream.isPending ? "正在查…" : "检查更新"}
          </button>
          <button type="button" onClick={() => setForm("new")} className="rounded-md bg-primary px-4 py-2 text-sm text-primary-foreground">添加助手</button>
        </div>
      </div>
      {search.trim() && filteredElements.length === 0 ? (
        <EmptyState
          icon={SearchX}
          testId="agents-search-empty"
          title="无匹配结果"
          description="没有名称或命令包含该关键词的助手。"
          actions={
            <button
              type="button"
              onClick={() => setSearch("")}
              className="inline-flex items-center gap-1 rounded border px-2 py-1 text-xs text-muted-foreground hover:bg-accent"
            >
              清除搜索关键词
            </button>
          }
        />
      ) : (
        <div className="grid grid-cols-1 gap-4 xl:grid-cols-2">
          {primaryElements.map((element) => (
            <AgentCard
              key={element.id}
              element={element}
              customDefinition={isCustomDefinition(element)}
              onEdit={() => setForm(element)}
              onRemove={() => definitionOps.requestRemove(element.id.replace(/^agents:/, ""), policyQuery.data?.data.layers ?? null)}
              onRequestUninstall={() => setPendingUninstall(element)}
              onRequestAction={(action) => setPendingAction({ element, action })}
              onManageSkills={onManageSkills}
              runningActionId={runningAction?.elementId === element.id ? runningAction.actionId : null}
              aliasValue={aliasOf(element)}
              aliasPreview={aliasPreviewOf(element)}
              aliasSyncing={aliasApply.executing}
              aliasEffect={aliasEffectOf(
                aliasElementsOf(aliasStatusQuery.data),
                (element.spec as AgentsSpec).bin ?? "",
              )}
              onEditAlias={() => {
                const bin = (element.spec as AgentsSpec).bin ?? element.id.replace(/^agents:/, "");
                setAliasEditing({ bin, name: bin, value: aliasOf(element) });
              }}
              onSyncAliases={() =>
                void aliasApply.apply({ command: "aliases:apply", label: "同步终端快捷命令" })
              }
              policyBusy={policyBusy}
              onToggleEnabled={(next) => void toggleEnabled(element.id.replace(/^agents:/, ""), next)}
            />
          ))}
        </div>
      )}
      <section className="space-y-3">
        <div className="flex items-center justify-between gap-3">
          <div>
            <h3 className="text-sm font-semibold text-foreground">实例</h3>
            <p className="mt-0.5 text-xs text-muted-foreground">
              同一个助手的多套独立配置：各自的配置目录、登录与会话，可以同时开着互不干扰，全局配置不受影响。
            </p>
          </div>
          <button
            type="button"
            data-testid="add-profile-button"
            disabled={profileCapableAgents.length === 0}
            onClick={() => setProfileForm("new")}
            className="shrink-0 rounded-md border border-primary/30 bg-primary/10 px-3 py-1.5 text-xs text-primary hover:bg-primary/15 disabled:opacity-50"
          >
            新建实例
          </button>
        </div>
        {profileElements.length === 0 ? (
          <EmptyState
            compact
            testId="profiles-empty"
            title="还没有实例"
            description="需要用第二个账号或另一套接口时，新建一个实例：它有自己的配置目录，和主实例同时开着也不会互相干扰。"
          />
        ) : (
          <div className="grid grid-cols-1 gap-4 xl:grid-cols-2">
            {profileElements.map((element) => {
              const id = element.id.replace(/^profiles:/, "");
              return (
                <ProfileCard
                  key={element.id}
                  element={element}
                  busy={profileApply.executing}
                  onApply={() =>
                    void profileApply.apply({
                      command: "profiles:apply",
                      label: `写入实例配置 ${id}`,
                      args: ["--only", element.id],
                    })
                  }
                  onTakeover={() =>
                    void profileApply.apply({
                      command: "profiles:apply",
                      label: `接管并写入实例配置 ${id}`,
                      args: ["--only", element.id, "--allow-takeover"],
                    })
                  }
                  onEdit={() => {
                    const definition = (policyQuery.data?.ok
                      ? ((policyQuery.data.data.layers?.sqlite as Record<string, any> | null)?.agents?.definitions?.[id] ?? null)
                      : null) as Record<string, unknown> | null;
                    setProfileForm(parseProfileDefinition(id, definition ?? {}));
                  }}
                  onRemove={() => setProfileRemoving(id)}
                  onDuplicate={() => {
                    const definition = (policyQuery.data?.ok
                      ? ((policyQuery.data.data.layers?.sqlite as Record<string, any> | null)?.agents?.definitions?.[id] ?? null)
                      : null) as Record<string, unknown> | null;
                    const base = parseProfileDefinition(id, definition ?? {});
                    const existing = new Set(elements.map((e) => e.id.replace(/^agents:/, "")));
                    let nextId = `${id}-copy`;
                    let n = 2;
                    while (existing.has(nextId)) {
                      nextId = `${id}-copy${n}`;
                      n += 1;
                    }
                    setProfileDuplicating(true);
                    setProfileForm({
                      ...base,
                      id: nextId,
                      displayName: `${base.displayName} 副本`,
                      // 必须换目录：两个实例指同一套配置就不是两个实例了
                      configDir: defaultConfigDir(base.kind, nextId),
                    });
                  }}
                  onManageSkills={() => onManageSkills?.(id)}
                  onSyncLaunch={() =>
                    void aliasApply.apply({ command: "aliases:apply", label: "同步终端快捷命令" })
                  }
                  launchCommands={launchCommandsOf(element)}
                  onAddLaunch={() => {
                    const bin = baseBinOf((element.spec as unknown as { base_agent?: string }).base_agent ?? id);
                    const existing = aliasSource.source?.aliases ?? {};
                    let candidate = `${id}-2`;
                    let n = 3;
                    while (existing[candidate] !== undefined) {
                      candidate = `${id}-${n}`;
                      n += 1;
                    }
                    // 以该实例已有的第一条为底稿：配置目录等前置变量照搬，只改名字和参数
                    const template = launchCommandsOf(element)[0]?.value ?? null;
                    setAliasEditing({ bin, name: "", value: template, presetName: candidate });
                  }}
                  onEditLaunch={(name) => {
                    const bin = baseBinOf((element.spec as unknown as { base_agent?: string }).base_agent ?? id);
                    setAliasEditing({ bin, name, value: aliasSource.source?.aliases[name] ?? null });
                  }}
                  onDuplicateLaunch={(name) =>
                    duplicateLaunchAlias(
                      name,
                      baseBinOf((element.spec as unknown as { base_agent?: string }).base_agent ?? id),
                    )
                  }
                  onRemoveLaunch={(name) => setLaunchRemoving(name)}
                  onFixLaunch={(name) => {
                    const current = aliasSource.source?.aliases[name];
                    if (!current) return;
                    void saveLaunchAlias(name, fixLaunchCommandTilde(current));
                  }}
                />
              );
            })}
          </div>
        )}
      </section>

      {connectionForm && connectionKindOf(connectionForm.agent) && (
        <ConnectionFormDialog
          agentName={(connectionForm.agent.spec as AgentsSpec).display_name ?? connectionForm.agent.id.replace(/^agents:/, "")}
          kind={connectionKindOf(connectionForm.agent)!}
          existingNames={connectionsOf(connectionForm.agent.spec as Record<string, unknown>).map((item) => item.name)}
          initialConnection={connectionForm.initial}
          initialActivate={
            connectionForm.initial
              ? activeConnectionOf(connectionForm.agent.spec as Record<string, unknown>) === connectionForm.initial.name
              : undefined
          }
          busy={connectionSaving}
          error={connectionFormError}
          onSubmit={(submission) => void saveConnection(connectionForm.agent, submission)}
          onClose={() => {
            setConnectionForm(null);
            setConnectionFormError(null);
          }}
        />
      )}
      {connectionRemoving && (
        <ConfirmModal
          title={`删除接入 ${connectionRemoving.connection.label || connectionRemoving.connection.name}`}
          description="只删除本应用保存的这套接入，不会删除凭据变量。当前正在使用的接入必须先切换到其他接入或助手默认账号。"
          risk="write-local"
          riskLabel="删除接入"
          confirmLabel="确认删除"
          executing={connectionRemovingBusy}
          onConfirm={() => void removeConnection(connectionRemoving.agent, connectionRemoving.connection)}
          onClose={() => setConnectionRemoving(null)}
        />
      )}

      {profileForm && (
        <ProfileFormDialog
          baseAgents={profileCapableAgents}
          initial={profileForm === "new" ? undefined : profileForm}
          isEdit={profileForm !== "new" && !profileDuplicating}
          existingIds={elements.map((element) => element.id.replace(/^agents:/, ""))}
          baseBinOf={baseBinOf}
          onSubmit={(input) => void saveProfile(input)}
          onClose={() => {
            setProfileForm(null);
            setProfileDuplicating(false);
          }}
        />
      )}

      {profileRemoving && (
        <ConfirmModal
          title={`移除实例 ${profileRemoving}`}
          description="只停止管理这套配置并删掉它的启动命令；配置目录、登录状态与里面的内容都原样保留，需要的话请自行删除目录。"
          risk="write-local"
          confirmLabel="确认移除"
          executing={false}
          onConfirm={() => void removeProfile(profileRemoving)}
          onClose={() => setProfileRemoving(null)}
        />
      )}

      {aliasEditing && aliasSource.source && (
        <AliasEditorDialog
          initialName={aliasEditing.value === null ? undefined : aliasEditing.name || undefined}
          initialValue={aliasEditing.value ?? undefined}
          defaultCommand={aliasEditing.bin}
          presetName={aliasEditing.presetName}
          globals={{ timezone: aliasSource.source.timezone, lang: aliasSource.source.lang }}
          existingNames={Object.keys(aliasSource.source.aliases)}
          onSubmit={(name, value) => {
            // value===null 表示新建（助手卡片「设置」走这条），此时 name 只是预填的可执行名，不算改名
            const renamedFrom = aliasEditing.value === null ? undefined : aliasEditing.name || undefined;
            void saveLaunchAlias(name, value, renamedFrom).then((ok) => {
              if (ok) setAliasEditing(null);
            });
          }}
          onClose={() => setAliasEditing(null)}
        />
      )}
      {launchRemoving && (
        <ConfirmModal
          title={`删除启动命令 ${launchRemoving}`}
          description="只删这条命令，实例的配置目录与登录状态不受影响。下次同步会把它从终端配置里本工具维护的那一段中去掉。"
          risk="write-local"
          confirmLabel="确认删除"
          executing={false}
          onConfirm={() => void removeLaunchAlias(launchRemoving)}
          onClose={() => setLaunchRemoving(null)}
        />
      )}
      {form && <DefinitionFormDialog existingNames={elements.map((element) => element.id.replace(/^agents:/, ""))} initial={form === "new" ? undefined : { name: form.id.replace(/^agents:/, ""), definition: form.spec }} onClose={() => setForm(null)} onSubmit={(name, definition) => {
        const isNew = form === "new";
        setForm(null);
        void (async () => {
          if (!(await definitionOps.saveDefinition({ name, definition, isNew }))) return;
          // 保存即写入：没配过运行参数就不碰助手的配置文件
          if (!hasAgentSettings(definition)) return;
          const yolo = (definition.settings as { yolo?: unknown } | undefined)?.yolo === true;
          if (yolo) {
            setPendingConfigure({ agentId: name, yolo: true, reason: "yolo" });
            return;
          }
          await runConfigure(name);
        })();
      }} />}
      {definitionOps.modals}

      {pendingConfigure && (
        <ConfirmModal
          title={pendingConfigure.reason === "yolo" ? "跳过权限确认" : "覆盖被改过的配置"}
          description={
            pendingConfigure.reason === "yolo"
              ? "写入后这个助手执行命令、改文件都不再征求你同意，任何入口启动它都一样。确认要这么配吗？"
              : "助手配置文件里本工具写的那几项被改过。继续会用这里的内容替换它们，文件里其余内容不动。"
          }
          risk="write-local"
          confirmLabel={pendingConfigure.reason === "yolo" ? "确认跳过权限确认" : "确认覆盖"}
          executing={configuring}
          onConfirm={() => {
            const pending = pendingConfigure;
            setPendingConfigure(null);
            void runConfigure(pending.agentId, {
              allowYolo: pending.yolo,
              allowTakeover: pending.reason === "takeover",
              use: pending.use,
            });
          }}
          onClose={() => {
            setPendingConfigure(null);
            toast.message("运行参数没写进去", { description: "定义已经存下了，助手的配置文件没动。" });
          }}
        />
      )}

      {uninstallElement && (
        <ConfirmModal
          title={`卸载 ${uninstallName}`}
          description="卸载将删除该助手在本工具管理下的安装与配置；命令以你的权限运行，结果不能自动撤销。"
          risk="install-remove"
          requireText={uninstallElement.id}
          confirmLabel="确认卸载"
          executing={directApply.executing}
          onConfirm={() => {
            setRunningAction({ elementId: uninstallElement.id, actionId: "uninstall" });
            void directApply
              .apply({
                command: "agents:apply",
                label: `卸载 ${uninstallName}`,
                args: ["--only", uninstallElement.id, "--action", "uninstall", "--allow-uninstall"],
              })
              .finally(() => setRunningAction(null));
            setPendingUninstall(null);
          }}
          onClose={() => setPendingUninstall(null)}
        />
      )}

      {pendingAction && (
        <ConfirmModal
          title={`${pendingActionLabel} ${pendingActionName}`}
          description={`该助手来自你的自定义定义：将执行定义中的${pendingActionLabel}命令，可能写入系统目录；命令以你的权限运行，结果不能自动撤销。`}
          risk={pendingAction.action.risk}
          confirmLabel={`确认${pendingActionLabel}`}
          executing={directApply.executing}
          onConfirm={() => {
            const versionHint = pendingAction.action.id === "upgrade" ? `（${versionRangeHint(pendingAction.element.freshness)}）` : "";
            setRunningAction({ elementId: pendingAction.element.id, actionId: pendingAction.action.id });
            void directApply
              .apply({
                command: "agents:apply",
                label: `${pendingActionLabel} ${pendingActionName}${versionHint}`,
                args: ["--only", pendingAction.element.id, "--action", pendingAction.action.id],
              })
              .finally(() => setRunningAction(null));
            setPendingAction(null);
          }}
          onClose={() => setPendingAction(null)}
        />
      )}
    </div>
  );
}
