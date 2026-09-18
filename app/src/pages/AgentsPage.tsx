import { useState } from "react";
import { toast } from "sonner";
import { Check, ChevronDown, ChevronRight, Download, Loader2, RefreshCw, SearchX, Sparkles, Terminal, Trash2, X } from "lucide-react";
import { AliasEditorDialog } from "../components/AliasEditorDialog";
import { ProfileCard } from "../components/ProfileCards";
import { ProfileFormDialog } from "../components/ProfileFormDialog";
import { ConfirmModal } from "../components/ConfirmModal";
import { DriftExplanationPanel } from "../components/DriftExplanationPanel";
import { DefinitionFormDialog } from "../components/DefinitionFormDialog";
import { useDefinitionOps } from "../lib/useDefinitionOps";
import { EmptyState } from "../components/EmptyState";
import { FirstRunGuide } from "../components/FirstRunGuide";
import { ErrorPanel } from "../components/ErrorPanel";
import { FreshnessBadge } from "../components/FreshnessBadge";
import { PageError, PageHeader, PageLoading } from "../components/PageState";
import { SearchInput, matchesKeyword } from "../components/SearchInput";
import { StateBadge } from "../components/StateBadge";
import type { ElementAction, EngineElement, PolicyData, StatusData } from "../lib/contract";
import { renderAliasPreview } from "../lib/aliasCommand";
import { ALIAS_EFFECT_HINT, ALIAS_EFFECT_LABEL, aliasEffectOf, aliasElementsOf, type AliasEffect } from "../lib/aliasEffect";
import {
  buildProfileDefinition,
  parseProfileDefinition,
  profileLaunchValue,
  type ProfileFormInput,
} from "../lib/profileDefinition";
import { runEngine } from "../lib/engine";
import { toastWithUndo } from "../lib/undo";
import { definitionOrigin } from "../lib/definitionOverlay";
import type { AgentsObserved, AgentsPolicy, AgentsSpec } from "../lib/observed";
import { useEngineQuery, useOfflineMode } from "../lib/queries";
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

/** provider 分类徽标（observed.classified 四桶 → 只读徽标）。 */
const PROVIDER_BUCKETS = ["managed", "deprecated", "system_only", "unsupported"] as const;
type ProviderBucket = (typeof PROVIDER_BUCKETS)[number];

const PROVIDER_CLASS_META: Record<ProviderBucket, { label: string; className: string }> = {
  managed: { label: "由此工具管理", className: "border-success/30 bg-success/10 text-success" },
  deprecated: { label: "已弃用", className: "border-warning/30 bg-warning/10 text-warning" },
  system_only: { label: "仅系统检测到", className: "border-info/30 bg-info/10 text-info" },
  unsupported: { label: "不可接管", className: "border bg-muted text-muted-foreground" },
};

/** 偏好开关：点一下就改本机偏好（只写配置，不触发安装或同步）。 */
function PolicyChip({
  label,
  value,
  testId,
  title,
  disabled,
  onToggle,
}: {
  label: string;
  value: boolean | undefined;
  testId?: string;
  title?: string;
  disabled?: boolean;
  onToggle?: (next: boolean) => void;
}) {
  const active = value !== false;
  const content = (
    <>
      {active ? (
        <Check className="h-3 w-3 text-success" aria-hidden />
      ) : (
        <X className="h-3 w-3 text-muted-foreground" aria-hidden />
      )}
      {label}
    </>
  );
  if (!onToggle) {
    return (
      <span className="inline-flex items-center gap-1 rounded border bg-card px-1.5 py-0.5 text-xs text-muted-foreground">
        {content}
      </span>
    );
  }
  return (
    <button
      type="button"
      data-testid={testId}
      title={title}
      disabled={disabled}
      aria-pressed={active}
      onClick={() => onToggle(!active)}
      className={cn(
        "inline-flex items-center gap-1 rounded border px-1.5 py-0.5 text-xs hover:bg-accent disabled:opacity-50",
        active ? "bg-card text-foreground" : "bg-muted text-muted-foreground",
      )}
    >
      {content}
    </button>
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
  onTogglePolicy,
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
  /** 偏好写入在途：期间禁用开关，避免连点写出交错结果。 */
  policyBusy: boolean;
  onTogglePolicy: (key: "enabled" | "auto_install" | "auto_update", next: boolean) => void;
}) {
  const [expanded, setExpanded] = useState(false);
  /** 本卡直执路径在途的动作 id（点击即置位，apply 结束回落）。 */
  const [localActionId, setLocalActionId] = useState<string | null>(null);
  const offline = useOfflineMode();
  const directApply = useDirectApply();
  const spec = element.spec as AgentsSpec;
  const policy = element.policy as AgentsPolicy;
  const observed = element.observed as AgentsObserved;
  const classified = observed.classified ?? {};
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
          <StateBadge state={element.state} />
          <FreshnessBadge freshness={element.freshness} className="items-end" />
        </div>
      </div>

      <div className="mt-3 flex flex-wrap items-center gap-1.5 text-xs">
        {PROVIDER_BUCKETS.flatMap((bucket) =>
          (classified[bucket] ?? []).map((provider) => (
            <span
              key={`${bucket}-${provider}`}
              className={cn("rounded border px-1.5 py-0.5", PROVIDER_CLASS_META[bucket].className)}
              title={`${provider}（${PROVIDER_CLASS_META[bucket].label}）`}
            >
              {provider}·{PROVIDER_CLASS_META[bucket].label}
            </span>
          )),
        )}
        {(observed.providers ?? []).length > 0 && (
          <span
            className="rounded border bg-muted px-1.5 py-0.5 text-muted-foreground"
            title="系统实际检测到的安装来源"
          >
            当前安装方式 {(observed.providers ?? []).join("、")}
          </span>
        )}
        {spec.preferred_provider && (
          <span
            className={cn(
              "rounded border px-1.5 py-0.5",
              observed.preferred_active
                ? "border-primary/30 bg-primary/10 text-primary"
                : "border bg-muted text-muted-foreground",
            )}
            title={observed.preferred_active ? "当前安装与默认安装方式一致" : "当前安装与默认安装方式不一致"}
          >
            默认安装方式 {spec.preferred_provider}
            {observed.preferred_active ? "·生效中" : "·未生效"}
          </span>
        )}
      </div>

      <div className="mt-3 flex flex-wrap items-center gap-1.5 border-t pt-3">
        <PolicyChip
          label="自动安装"
          value={policy.auto_install}
          testId={`policy-auto-install-${element.id}`}
          title="缺失时是否自动安装（只改偏好，不会立刻执行安装）"
          disabled={policyBusy}
          onToggle={(next) => onTogglePolicy("auto_install", next)}
        />
        <PolicyChip
          label="自动更新"
          value={policy.auto_update}
          testId={`policy-auto-update-${element.id}`}
          title="有新版本时是否自动更新（只改偏好）"
          disabled={policyBusy}
          onToggle={(next) => onTogglePolicy("auto_update", next)}
        />
        <PolicyChip
          label={policy.enabled === false ? "已停用" : "已启用"}
          value={policy.enabled}
          testId={`policy-enabled-${element.id}`}
          title="停用后不再为它维护技能、MCP 等资源；已装的软件不受影响"
          disabled={policyBusy}
          onToggle={(next) => onTogglePolicy("enabled", next)}
        />
      </div>

      {element.state === "unmanaged-detected" && (
        <div className="mt-3 rounded-md border border-warning/30 bg-warning/10 px-3 py-2 text-xs text-warning">
          <p>
            检测到用 {(observed.classified?.unsupported ?? [])[0] ?? "其它渠道"} 安装的版本，该来源不在本工具管理范围内。
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
        <p data-testid={`alias-preview-${element.id}`} className="mt-1.5 break-all font-mono text-[11px] text-muted-foreground">
          {aliasPreview ?? `还没设置：终端里直接执行 ${spec.bin}，不带固定时区与参数`}
        </p>
      </div>

      <div className="mt-3 flex flex-wrap items-center gap-2 border-t pt-3">
        <button type="button" onClick={onEdit} className="rounded border px-2.5 py-1 text-xs">编辑命令</button>
        <button type="button" onClick={onRemove} className="rounded border border-destructive/30 px-2.5 py-1 text-xs text-destructive">移除管理</button>
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
          if (!meta) return null;
          const Icon = meta.icon;
          const busy = busyActionId === action.id;
          const offlineBlocked = offline && !action.offline_ok;
          /** 任一动作在途时整卡动作禁用（防并发触发），在途按钮自身转 spinner + 执行中文案。 */
          const disabled = offlineBlocked || busyActionId !== null;
          const execHint = `${meta.label}${action.id === "upgrade" ? `（${versionRangeHint(element.freshness)}）` : ""}：${
            customDefinition ? "先弹确认再执行" : "直接执行"
          }，命令以你的权限运行，结果不能自动撤销`;
          const title = busy ? COPY.executing : offlineBlocked ? "离线模式下不可用（需要联网）" : execHint;
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
        <button
          type="button"
          onClick={() => setExpanded((value) => !value)}
          className="ml-auto inline-flex items-center gap-1 text-xs text-primary hover:text-primary"
          aria-expanded={expanded}
        >
          {expanded ? <ChevronDown className="h-3.5 w-3.5" aria-hidden /> : <ChevronRight className="h-3.5 w-3.5" aria-hidden />}
          {expanded ? "收起同步详情" : "展开同步详情"}
        </button>
      </div>
      {expanded && <div className="mt-2"><DriftExplanationPanel element={element} /></div>}
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
  const [profileRemoving, setProfileRemoving] = useState<string | null>(null);
  const [policyBusy, setPolicyBusy] = useState(false);
  const [aliasEditing, setAliasEditing] = useState<{ bin: string; name: string; value: string | null } | null>(null);
  const [form, setForm] = useState<EngineElement | "new" | null>(null);
  const [pendingUninstall, setPendingUninstall] = useState<EngineElement | null>(null);
  const [pendingAction, setPendingAction] = useState<{ element: EngineElement; action: ElementAction } | null>(null);
  /** 确认弹窗路径（卸载/自定义定义安装升级）在途动作：弹窗确认即关闭，卡片按钮据此保持执行态直到 apply 结束。 */
  const [runningAction, setRunningAction] = useState<{ elementId: string; actionId: string } | null>(null);
  const [search, setSearch] = useState("");
  if (query.isPending) return <PageLoading />;
  if (query.isError) return <PageError title="agents:status 调用失败" error={query.error} />;

  const elements = query.data.data.elements;
  const filteredElements = elements.filter((element) => {
    const spec = element.spec as AgentsSpec;
    return matchesKeyword(search, [spec.display_name, element.id, spec.bin, ...(element.observed as AgentsObserved).providers ?? []]);
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
  const primaryElements = filteredElements.filter((element) => !profileIds.has(element.id.replace(/^agents:/, "")));
  /** 可作为实例归属的助手：目前只有 Claude Code 与 Codex 有隔离配置目录的官方支持。 */
  const profileCapableAgents = elements
    .map((element) => ({ id: element.id.replace(/^agents:/, ""), spec: element.spec as AgentsSpec }))
    .filter((item) => !profileIds.has(item.id) && ["claude-code", "codex"].includes(item.id))
    .map((item) => ({ id: item.id, displayName: item.spec.display_name ?? item.id }));
  const baseBinOf = (agentId: string) =>
    (elements.find((element) => element.id === `agents:${agentId}`)?.spec as AgentsSpec | undefined)?.bin ??
    (agentId === "codex" ? "codex" : "claude");

  /** 改助手偏好：只写本机配置，不触发安装、卸载或同步。 */
  const togglePolicy = async (agentId: string, key: "enabled" | "auto_install" | "auto_update", next: boolean) => {
    if (policyBusy) return;
    setPolicyBusy(true);
    try {
      const envelope = await runEngine("policy:set", [
        "--layer",
        "local",
        "--key",
        `agents.${agentId}.${key}`,
        "--value",
        JSON.stringify(next),
      ]);
      if (!envelope.ok) {
        toast.error(`保存偏好被拒绝:${envelope.errors[0]?.message ?? "未知错误"}`);
        return;
      }
      toast.success(next ? "已开启" : "已关闭", {
        description:
          key === "enabled"
            ? next
              ? "重新为它维护技能、MCP 等资源。"
              : "不再为它维护技能、MCP 等资源；已安装的软件不受影响。"
            : "只改本机偏好，不会立刻执行安装或更新。",
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
  const uninstallElement = pendingUninstall;
  const uninstallName = uninstallElement ? (uninstallElement.spec as AgentsSpec).display_name ?? uninstallElement.id : "";
  const pendingActionName = pendingAction
    ? (pendingAction.element.spec as AgentsSpec).display_name ?? pendingAction.element.id
    : "";
  const pendingActionLabel = pendingAction ? (ACTION_META[pendingAction.action.id]?.label ?? pendingAction.action.id) : "";
  return (
    <div className="space-y-4">
      <PageHeader title="助手" description="各 AI 助手的安装渠道、版本与官方最新版对比。内置助手安装/升级点击后直接执行；自定义定义的安装/升级与卸载需先确认。" />
      <FirstRunGuide show={freshInstall} onGoTakeover={onGoTakeover} />
      {query.data.errors.length > 0 && <ErrorPanel title="agents:status 错误" errors={query.data.errors} />}
      <div className="flex items-center justify-between gap-3">
        <SearchInput value={search} onChange={setSearch} placeholder="搜索助手…" testId="agents-search" />
        <button type="button" onClick={() => setForm("new")} className="shrink-0 rounded-md bg-primary px-4 py-2 text-sm text-primary-foreground">添加助手</button>
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
              onTogglePolicy={(key, next) => void togglePolicy(element.id.replace(/^agents:/, ""), key, next)}
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
                  onManageSkills={() => onManageSkills?.(id)}
                  onSyncLaunch={() =>
                    void aliasApply.apply({ command: "aliases:apply", label: "同步终端快捷命令" })
                  }
                />
              );
            })}
          </div>
        )}
      </section>

      {profileForm && (
        <ProfileFormDialog
          baseAgents={profileCapableAgents}
          initial={profileForm === "new" ? undefined : profileForm}
          existingIds={elements.map((element) => element.id.replace(/^agents:/, ""))}
          baseBinOf={baseBinOf}
          onSubmit={(input) => void saveProfile(input)}
          onClose={() => setProfileForm(null)}
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
          initialName={aliasEditing.value === null ? undefined : aliasEditing.name}
          initialValue={aliasEditing.value ?? undefined}
          defaultCommand={aliasEditing.bin}
          globals={{ timezone: aliasSource.source.timezone, lang: aliasSource.source.lang }}
          existingNames={Object.keys(aliasSource.source.aliases)}
          onSubmit={(name, value) => {
            const aliases = { ...aliasSource.source!.aliases, [name]: value };
            void aliasSource.save({ aliases }, `已保存 ${name} 的启动命令，点「同步到终端」写入配置`).then((ok) => {
              if (ok) setAliasEditing(null);
            });
          }}
          onClose={() => setAliasEditing(null)}
        />
      )}
      {form && <DefinitionFormDialog existingNames={elements.map((element) => element.id.replace(/^agents:/, ""))} initial={form === "new" ? undefined : { name: form.id.replace(/^agents:/, ""), definition: form.spec }} onClose={() => setForm(null)} onSubmit={(name, definition) => { setForm(null); void definitionOps.saveDefinition({ name, definition, isNew: form === "new" }); }} />}
      {definitionOps.modals}

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
