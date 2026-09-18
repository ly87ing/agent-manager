import { Inbox, Pencil, RefreshCw, Trash2 } from "lucide-react";
import type { EngineElement } from "../lib/contract";
import type { SkillCollectionObserved, SkillCollectionSpec, SkillSpec } from "../lib/observed";
import { FreshnessBadge } from "./FreshnessBadge";
import { StateBadge } from "./StateBadge";
import { cn } from "../lib/utils";

function shortCommit(value: string | null | undefined): string {
  if (!value) return "?";
  return value.length > 12 ? value.slice(0, 7) : value;
}

/**
 * 单个技能集合卡片：定义/状态展示 + 集合级操作。
 * 操作按钮严格按 collection.observed.actions 的实际子集渲染
 * （update|enable|disable|remove），不猜 state；受阻原因原样展示。
 */
export function SkillCollectionCard({
  collection,
  onDrilldown,
  onEdit,
  onUpdate,
  onToggle,
  onRemove,
}: {
  collection: EngineElement;
  /** 点击集合名/已收录计数：切到全部技能并按此集合筛选。 */
  onDrilldown: (name: string) => void;
  onEdit: (collection: EngineElement) => void;
  onUpdate: (collection: EngineElement) => void;
  onToggle: (collection: EngineElement, action: "enable" | "disable") => void;
  onRemove: (collection: EngineElement) => void;
}) {
  const spec = collection.spec as SkillCollectionSpec;
  const observed = collection.observed as SkillCollectionObserved;
  const name = collection.id.replace(/^skill:/, "");
  const actions = observed.actions ?? [];
  const blockedReason = observed.blocked_reason;
  const isEnabled = (collection.policy as { enabled?: boolean }).enabled !== false;
  const inboxCount = observed.discovery_inbox?.length ?? 0;

  return (
    <section
      data-testid={`collection-card-${name}`}
      className="rounded-lg border bg-card p-5 shadow-sm transition-shadow hover:shadow-md"
    >
      <div className="flex items-start justify-between gap-3">
        <div>
          <button
            type="button"
            onClick={() => onDrilldown(name)}
            className="text-left text-base font-semibold text-primary hover:text-primary"
            title="点击查看此集合包含的全部技能"
          >
            {name}
          </button>
          <p className="mt-0.5 font-mono text-xs break-all text-muted-foreground">{spec.source_path ?? spec.url ?? "（未填来源）"}</p>
        </div>
        <div className="flex items-center gap-2">
          <StateBadge state={collection.state} />
        </div>
      </div>

      <div className="mt-3 flex flex-wrap gap-1.5 text-xs">
        <span
          className={cn(
            "rounded border px-1.5 py-0.5",
            observed.repo_cached
              ? "border-success/30 bg-success/10 text-success"
              : "border-warning/30 bg-warning/10 text-warning",
          )}
          title={observed.repo_cache}
        >
          {spec.source_path
            ? observed.repo_cached
              ? "本机目录可用"
              : "本机目录不存在"
            : observed.repo_cached
              ? "已下载到本地"
              : "尚未下载"}
        </span>
        {inboxCount > 0 && (
          <span
            data-testid={`inbox-count-${name}`}
            className="rounded border border-info/30 bg-info/10 px-1.5 py-0.5 font-medium text-info"
          >
            新发现 {inboxCount}
          </span>
        )}
      </div>

      <p className="mt-2 text-xs text-muted-foreground">新技能需单独添加到助手；更新资源库不会改变助手已选版本。</p>

      {observed.discovery_error && (
        <div
          data-testid={`discovery-error-${name}`}
          className="mt-3 rounded-md border border-warning/30 bg-warning/10 px-3 py-2 text-xs text-warning"
        >
          {observed.discovery_error.message ?? "技能发现失败"}
        </div>
      )}

      {blockedReason && (
        <div className="mt-3 rounded-md border border-warning/30 bg-warning/10 px-3 py-2 text-xs text-warning">
          受阻原因：{blockedReason}
        </div>
      )}

      <div className="mt-3 flex items-center justify-between border-t pt-3 text-xs text-muted-foreground">
        <button type="button" onClick={() => onDrilldown(name)} className="text-left text-muted-foreground hover:underline">
          已收录 {observed.discovered?.length ?? 0} 个技能
        </button>
        <div className="flex items-center gap-2">
          <span>
            最新提交 <code className="font-mono">{shortCommit(collection.freshness.upstream)}</code>
          </span>
          <FreshnessBadge freshness={collection.freshness} className="items-end" />
        </div>
      </div>

      <div className="mt-4 flex flex-wrap items-center gap-2">
        <button
          type="button"
          data-testid={`edit-collection-${name}`}
          onClick={() => onEdit(collection)}
          className="inline-flex items-center gap-1 rounded border bg-card px-2.5 py-1 text-xs text-muted-foreground hover:bg-accent"
        >
          <Pencil className="h-3.5 w-3.5" aria-hidden /> 编辑定义
        </button>

        {actions.includes("update") && (
          <button
            type="button"
            data-testid={`update-collection-${name}`}
            onClick={() => onUpdate(collection)}
            className="inline-flex items-center gap-1 rounded border border-primary/30 bg-primary/10 px-2.5 py-1 text-xs text-primary hover:bg-primary/15"
          >
            <RefreshCw className="h-3.5 w-3.5" aria-hidden /> 更新资源库
          </button>
        )}

        {actions.includes("enable") && !isEnabled && (
          <button
            type="button"
            data-testid={`enable-collection-${name}`}
            onClick={() => onToggle(collection, "enable")}
            className="inline-flex items-center gap-1 rounded border border-success/30 bg-success/10 px-2.5 py-1 text-xs text-success hover:bg-success/15"
          >
            启用
          </button>
        )}

        {actions.includes("disable") && isEnabled && (
          <button
            type="button"
            data-testid={`disable-collection-${name}`}
            onClick={() => onToggle(collection, "disable")}
            className="inline-flex items-center gap-1 rounded border border-warning/30 bg-warning/10 px-2.5 py-1 text-xs text-warning hover:bg-warning/15"
          >
            停用
          </button>
        )}

        {actions.includes("remove") && (
          <button
            type="button"
            data-testid={`remove-collection-${name}`}
            onClick={() => onRemove(collection)}
            className="inline-flex items-center gap-1 rounded border border-destructive/30 bg-card px-2.5 py-1 text-xs text-destructive hover:bg-destructive/10"
          >
            <Trash2 className="h-3.5 w-3.5" aria-hidden /> 删除集合
          </button>
        )}

        {actions.length === 0 && (
          <span className="text-xs text-muted-foreground">这里没有可做的：状态正常或该集合已停用</span>
        )}
      </div>
    </section>
  );
}

/** 新发现技能列表（集合面板底部）。只读展示；添加分配在「全部技能」按助手操作。 */
export function SkillDiscoveryInbox({ skills }: { skills: EngineElement[] }) {
  return (
    <section data-testid="discovery-inbox" className="group rounded-lg border bg-card p-5 shadow-sm">
      <h3 className="mb-3 flex items-center gap-2 text-sm font-semibold text-foreground">
        <Inbox className="h-4 w-4 text-info transition-transform group-hover:scale-105" aria-hidden />
        新发现的技能
        <span data-testid="inbox-total" className="rounded-full bg-info/15 px-2 py-0.5 text-xs font-medium text-info">
          {skills.length}
        </span>
      </h3>
      {skills.length === 0 ? (
        <p className="text-sm text-muted-foreground">集合里没有新发现的技能。</p>
      ) : (
        <div className="overflow-x-auto">
          <table className="w-full text-left text-sm">
            <thead>
              <tr className="border-b text-xs text-muted-foreground">
                <th className="py-2 pr-4 font-medium">名称</th>
                <th className="py-2 pr-4 font-medium">来源</th>
                <th className="py-2 font-medium">说明</th>
              </tr>
            </thead>
            <tbody>
              {skills.map((skill) => {
                const spec = skill.spec as SkillSpec;
                return (
                  <tr key={skill.id} className="border-b last:border-0">
                    <td className="py-2 pr-4 font-mono text-xs text-foreground">{spec.name ?? skill.id}</td>
                    <td className="py-2 pr-4 font-mono text-xs break-all text-muted-foreground">{spec.source_path ?? spec.collection}</td>
                    <td className="py-2 text-xs text-muted-foreground">集合里新出现的技能；请在「全部技能」中按目标助手添加分配，启用后才会分发到助手。</td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}
    </section>
  );
}
