import { useState } from "react";
import { Check, ChevronDown, ChevronRight, Download, RefreshCw, Trash2, X } from "lucide-react";
import { DriftExplanationPanel } from "../components/DriftExplanationPanel";
import { ErrorPanel } from "../components/ErrorPanel";
import { FreshnessBadge } from "../components/FreshnessBadge";
import { PageError, PageHeader, PageLoading } from "../components/PageState";
import { StateBadge } from "../components/StateBadge";
import type { ElementAction, EngineElement, StatusData } from "../lib/contract";
import type { AgentsObserved, AgentsPolicy, AgentsSpec } from "../lib/observed";
import { useEngineQuery, useOfflineMode } from "../lib/queries";
import { useExecuteApply, usePlanPreview } from "../lib/usePlanPreview";
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

function AgentCard({ element }: { element: EngineElement }) {
  const [expanded, setExpanded] = useState(false);
  const offline = useOfflineMode();
  const preview = usePlanPreview();
  const executeApply = useExecuteApply();
  const spec = element.spec as AgentsSpec;
  const policy = element.policy as AgentsPolicy;
  const observed = element.observed as AgentsObserved;
  const classified = observed.classified ?? {};
  const installed = observed.installed ?? false;
  const version = installed ? (element.freshness.current ?? "已安装（还没检查过版本）") : "未安装";
  const name = spec.display_name ?? element.id;

  /** 动作 → plan 预览（过滤到本元素）→ agents:apply（域级收敛；含卸载时引擎要求 --allow-uninstall）。 */
  const openAction = (action: ElementAction) => {
    const isDestructive = action.id === "uninstall" || action.id === "install";
    preview.open({
      title: `${ACTION_META[action.id]?.label ?? action.id} ${name}`,
      subtitle: `预览只显示这个助手的相关操作；实际执行会同步全部助手配置（执行前自动备份）。`,
      planCommands: ["agents:plan"],
      filterElement: element.id,
      requireConfirmText: isDestructive ? element.id : undefined,
      confirmLabel: `${ACTION_META[action.id]?.label ?? action.id}`,
      execute: executeApply({
        command: "agents:apply",
        label: `${ACTION_META[action.id]?.label ?? action.id} ${name}`,
        extraArgs: isDestructive ? ["--allow-uninstall"] : [],
      }),
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
        {spec.preferred_provider && (
          <span
            className={cn(
              "rounded border px-1.5 py-0.5",
              observed.preferred_active
                ? "border-indigo-200 bg-indigo-50 text-indigo-700"
                : "border-slate-200 bg-slate-50 text-slate-500",
            )}
            title={observed.preferred_active ? "推荐渠道生效中" : "当前安装不走推荐渠道"}
          >
            推荐渠道 {spec.preferred_provider}
            {observed.preferred_active ? "·生效中" : "·未生效"}
          </span>
        )}
      </div>

      <div className="mt-3 flex flex-wrap gap-1.5 border-t border-slate-100 pt-3">
        <PolicyChip label="enabled" value={policy.enabled} />
        <PolicyChip label="auto_install" value={policy.auto_install} />
        <PolicyChip label="auto_update" value={policy.auto_update} />
      </div>

      {element.state === "unmanaged-detected" && (
        <div className="mt-3 rounded-md border border-amber-200 bg-amber-50 px-3 py-2 text-xs text-amber-800">
          <p>
            检测到用 {(observed.classified?.unsupported ?? [])[0] ?? "其它渠道"} 安装的旧版，建议迁移到官方
            {spec.preferred_provider ? `${spec.preferred_provider} ` : ""}渠道，之后就能用本工具统一升级和卸载。
          </p>
          <button
            type="button"
            data-testid={`migrate-${element.id}`}
            disabled={offline}
            onClick={() =>
              openAction({ id: "install", risk: "install-remove", needs_lock: true, offline_ok: false })
            }
            className={cn(
              "mt-2 inline-flex items-center gap-1 rounded border px-2 py-1",
              offline ? "cursor-not-allowed border-slate-200 text-slate-400" : "border-amber-300 text-amber-800 hover:bg-amber-100",
            )}
          >
            迁移到官方渠道
          </button>
          {offline && <span className="ml-2 text-xs text-amber-600">离线模式下不可用（需要联网）</span>}
        </div>
      )}

      <div className="mt-3 flex flex-wrap items-center gap-2 border-t border-slate-100 pt-3">
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
              title={disabled ? "离线模式下不可用（需要联网）" : `${meta.label}（会先预览将要执行的操作）`}
              onClick={() => openAction(action)}
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
      {expanded && <div className="mt-2"><DriftExplanationPanel element={element} planCommand="agents:plan" /></div>}
      {preview.modal}
    </section>
  );
}

export default function AgentsPage() {
  const query = useEngineQuery<StatusData>("agents:status");
  if (query.isPending) return <PageLoading />;
  if (query.isError) return <PageError title="agents:status 调用失败" error={query.error} />;

  const elements = query.data.data.elements;
  return (
    <div className="space-y-4">
      <PageHeader title="助手" description="各 AI 助手的安装渠道、版本与官方最新版对比。安装/升级/卸载都会先预览再执行。" />
      {query.data.errors.length > 0 && <ErrorPanel title="agents:status 错误" errors={query.data.errors} />}
      <div className="grid grid-cols-1 gap-4 xl:grid-cols-2">
        {elements.map((element) => (
          <AgentCard key={element.id} element={element} />
        ))}
      </div>
    </div>
  );
}
