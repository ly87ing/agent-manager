import { useState } from "react";
import { Check, ChevronDown, ChevronRight, Download, RefreshCw, Sparkles, Trash2, X } from "lucide-react";
import { ConfirmModal } from "../components/ConfirmModal";
import { DriftExplanationPanel } from "../components/DriftExplanationPanel";
import { DefinitionFormDialog } from "../components/DefinitionFormDialog";
import { useDefinitionOps } from "../lib/useDefinitionOps";
import { ErrorPanel } from "../components/ErrorPanel";
import { FreshnessBadge } from "../components/FreshnessBadge";
import { PageError, PageHeader, PageLoading } from "../components/PageState";
import { StateBadge } from "../components/StateBadge";
import type { ElementAction, EngineElement, PolicyData, StatusData } from "../lib/contract";
import { definitionOrigin } from "../lib/definitionOverlay";
import type { AgentsObserved, AgentsPolicy, AgentsSpec } from "../lib/observed";
import { useEngineQuery, useOfflineMode } from "../lib/queries";
import { useDirectApply } from "../lib/useDirectApply";
import { cn } from "../lib/utils";

/** agents 动作按钮文案/样式（离线可用性由 actions[].offline_ok 驱动，引擎是事实源）。 */
const ACTION_META: Record<string, { label: string; icon: typeof Download; className: string }> = {
  install: { label: "安装", icon: Download, className: "border-indigo-200 text-indigo-600 hover:bg-indigo-50" },
  upgrade: { label: "升级", icon: RefreshCw, className: "border-amber-200 text-amber-700 hover:bg-amber-50" },
  uninstall: { label: "卸载", icon: Trash2, className: "border-red-200 text-red-600 hover:bg-red-50" },
};

/** provider 分类徽标（observed.classified 四桶 → 只读徽标）。 */
const PROVIDER_BUCKETS = ["managed", "deprecated", "system_only", "unsupported"] as const;
type ProviderBucket = (typeof PROVIDER_BUCKETS)[number];

const PROVIDER_CLASS_META: Record<ProviderBucket, { label: string; className: string }> = {
  managed: { label: "由此工具管理", className: "border-emerald-200 bg-emerald-50 text-emerald-700" },
  deprecated: { label: "已弃用", className: "border-amber-200 bg-amber-50 text-amber-700" },
  system_only: { label: "仅系统检测到", className: "border-sky-200 bg-sky-50 text-sky-700" },
  unsupported: { label: "不可接管", className: "border-zinc-200 bg-zinc-100 text-zinc-500" },
};

function PolicyChip({ label, value }: { label: string; value: boolean | undefined }) {
  return (
    <span className="inline-flex items-center gap-1 rounded border border-slate-200 bg-white px-1.5 py-0.5 text-xs text-slate-600">
      {value ? (
        <Check className="h-3 w-3 text-emerald-600" aria-hidden />
      ) : (
        <X className="h-3 w-3 text-slate-300" aria-hidden />
      )}
      {label}
    </span>
  );
}

function AgentCard({
  element,
  customDefinition,
  onEdit,
  onRemove,
  onRequestUninstall,
  onRequestAction,
  onManageSkills,
}: {
  element: EngineElement;
  /** true = 定义来自本地数据库（用户新增或改过内置定义）：安装/升级先弹确认。 */
  customDefinition: boolean;
  onEdit: () => void;
  onRemove: () => void;
  onRequestUninstall: () => void;
  onRequestAction: (action: ElementAction) => void;
  onManageSkills?: (agentId: string) => void;
}) {
  const [expanded, setExpanded] = useState(false);
  const offline = useOfflineMode();
  const directApply = useDirectApply();
  const spec = element.spec as AgentsSpec;
  const policy = element.policy as AgentsPolicy;
  const observed = element.observed as AgentsObserved;
  const classified = observed.classified ?? {};
  const installed = observed.installed ?? false;
  const version = installed ? (element.freshness.current ?? "已安装（还没检查过版本）") : "未安装";
  const name = spec.display_name ?? element.id;

  /** 显式动作点击直接 agents:apply（带精确 --only/--action）；卸载先经独立 ConfirmModal，
   * 自定义定义的安装/升级也先经 ConfirmModal（命令内容由用户定义提供）。 */
  const runAction = (action: ElementAction) => {
    if (action.id === "uninstall") {
      onRequestUninstall();
      return;
    }
    if (customDefinition) {
      onRequestAction(action);
      return;
    }
    const label = ACTION_META[action.id]?.label ?? action.id;
    void directApply.apply({
      command: "agents:apply",
      label: `${label} ${name}`,
      args: ["--only", element.id, "--action", action.id],
    });
  };

  return (
    <section className="rounded-lg border border-slate-200 bg-white p-5 shadow-sm">
      <div className="flex items-start justify-between gap-3">
        <div>
          <h3 className="text-base font-semibold text-slate-800">
            {name}
            <span className="ml-2 font-mono text-xs font-normal text-slate-400">{spec.bin}</span>
          </h3>
          <p className="mt-0.5 text-sm text-slate-500">当前版本:{version}</p>
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
            className="rounded border border-slate-200 bg-slate-50 px-1.5 py-0.5 text-slate-600"
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
                ? "border-indigo-200 bg-indigo-50 text-indigo-700"
                : "border-slate-200 bg-slate-50 text-slate-500",
            )}
            title={observed.preferred_active ? "当前安装与默认安装方式一致" : "当前安装与默认安装方式不一致"}
          >
            默认安装方式 {spec.preferred_provider}
            {observed.preferred_active ? "·生效中" : "·未生效"}
          </span>
        )}
      </div>

      <div className="mt-3 flex flex-wrap items-center gap-1.5 border-t border-slate-100 pt-3">
        <PolicyChip label="自动安装" value={policy.auto_install} />
        <PolicyChip label="自动更新" value={policy.auto_update} />
        <span className="ml-auto text-xs text-slate-500">
          {policy.enabled === false ? "已停用" : "已启用"}
        </span>
      </div>

      {element.state === "unmanaged-detected" && (
        <div className="mt-3 rounded-md border border-amber-200 bg-amber-50 px-3 py-2 text-xs text-amber-800">
          <p>
            检测到用 {(observed.classified?.unsupported ?? [])[0] ?? "其它渠道"} 安装的版本，该来源不在本工具管理范围内。
            当前不支持自动迁移安装方式；如需由本工具管理，请先自行卸载现有安装，再使用「安装」。
          </p>
        </div>
      )}

      <div className="mt-3 flex flex-wrap items-center gap-2 border-t border-slate-100 pt-3">
        <button type="button" onClick={onEdit} className="rounded border px-2.5 py-1 text-xs">编辑命令</button>
        <button type="button" onClick={onRemove} className="rounded border border-red-200 px-2.5 py-1 text-xs text-red-600">移除管理</button>
        <button
          type="button"
          data-testid={`manage-skills-${element.id}`}
          onClick={() => onManageSkills?.(element.id.replace(/^agents:/, ""))}
          className="inline-flex items-center gap-1 rounded border border-indigo-200 bg-indigo-50 px-2.5 py-1 text-xs text-indigo-700 hover:bg-indigo-100"
        >
          <Sparkles className="h-3 w-3" aria-hidden /> 管理技能
        </button>
        {element.actions.map((action) => {
          const meta = ACTION_META[action.id];
          if (!meta) return null;
          const Icon = meta.icon;
          const disabled = offline && !action.offline_ok;
          return (
            <button
              key={action.id}
              type="button"
              data-testid={`action-${action.id}-${element.id}`}
              disabled={disabled}
              title={
                disabled
                  ? "离线模式下不可用（需要联网）"
                  : customDefinition
                    ? `${meta.label}：先弹确认再执行，命令以你的权限运行，结果不能自动撤销`
                    : `${meta.label}：直接执行，命令以你的权限运行，结果不能自动撤销`
              }
              onClick={() => runAction(action)}
              className={cn(
                "inline-flex items-center gap-1 rounded border px-2.5 py-1 text-xs",
                disabled ? "cursor-not-allowed border-slate-200 text-slate-300" : meta.className,
              )}
            >
              <Icon className="h-3 w-3" aria-hidden />
              {meta.label}
            </button>
          );
        })}
        <button
          type="button"
          onClick={() => setExpanded((value) => !value)}
          className="ml-auto inline-flex items-center gap-1 text-xs text-indigo-600 hover:text-indigo-800"
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
}: {
  onManageSkills?: (agentId: string) => void;
} = {}) {
  const query = useEngineQuery<StatusData>("agents:status");
  const policyQuery = useEngineQuery<PolicyData>("policy:get");
  const definitionOps = useDefinitionOps({ section: "agents", domain: "agents" });
  const directApply = useDirectApply();
  const [form, setForm] = useState<EngineElement | "new" | null>(null);
  const [pendingUninstall, setPendingUninstall] = useState<EngineElement | null>(null);
  const [pendingAction, setPendingAction] = useState<{ element: EngineElement; action: ElementAction } | null>(null);
  if (query.isPending) return <PageLoading />;
  if (query.isError) return <PageError title="agents:status 调用失败" error={query.error} />;

  const elements = query.data.data.elements;
  const layers = policyQuery.data?.data.layers ?? null;
  /** 内置判定：policy 层已加载且本地数据库无该定义覆盖；未加载到时按自定义处理（先确认再执行）。 */
  const isCustomDefinition = (element: EngineElement) =>
    layers === null || definitionOrigin(layers, "agents", element.id.replace(/^agents:/, "")) === "sqlite";
  const uninstallElement = pendingUninstall;
  const uninstallName = uninstallElement ? (uninstallElement.spec as AgentsSpec).display_name ?? uninstallElement.id : "";
  const pendingActionName = pendingAction
    ? (pendingAction.element.spec as AgentsSpec).display_name ?? pendingAction.element.id
    : "";
  const pendingActionLabel = pendingAction ? (ACTION_META[pendingAction.action.id]?.label ?? pendingAction.action.id) : "";
  return (
    <div className="space-y-4">
      <PageHeader title="助手" description="各 AI 助手的安装渠道、版本与官方最新版对比。内置助手安装/升级点击后直接执行；自定义定义的安装/升级与卸载需先确认。" />
      {query.data.errors.length > 0 && <ErrorPanel title="agents:status 错误" errors={query.data.errors} />}
      <div className="flex justify-end"><button type="button" onClick={() => setForm("new")} className="rounded-md bg-indigo-600 px-4 py-2 text-sm text-white">添加助手</button></div>
      <div className="grid grid-cols-1 gap-4 xl:grid-cols-2">
        {elements.map((element) => (
          <AgentCard
            key={element.id}
            element={element}
            customDefinition={isCustomDefinition(element)}
            onEdit={() => setForm(element)}
            onRemove={() => definitionOps.requestRemove(element.id.replace(/^agents:/, ""), policyQuery.data?.data.layers ?? null)}
            onRequestUninstall={() => setPendingUninstall(element)}
            onRequestAction={(action) => setPendingAction({ element, action })}
            onManageSkills={onManageSkills}
          />
        ))}
      </div>
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
            void directApply.apply({
              command: "agents:apply",
              label: `卸载 ${uninstallName}`,
              args: ["--only", uninstallElement.id, "--action", "uninstall", "--allow-uninstall"],
            });
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
            void directApply.apply({
              command: "agents:apply",
              label: `${pendingActionLabel} ${pendingActionName}`,
              args: ["--only", pendingAction.element.id, "--action", pendingAction.action.id],
            });
            setPendingAction(null);
          }}
          onClose={() => setPendingAction(null)}
        />
      )}
    </div>
  );
}
