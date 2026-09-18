import { FolderTree, Loader2, PlayCircle, Sparkles, Terminal, Trash2 } from "lucide-react";
import type { EngineElement } from "../lib/contract";
import type { ProfileObserved, ProfileSpec } from "../lib/contract";
import { StateBadge } from "./StateBadge";
import { cn } from "../lib/utils";

/** 实例状态 → 一句话说明用户现在处于什么位置。 */
const STATE_NOTE: Record<string, string> = {
  "not-materialized": "还没写入这套配置，点「写入配置」后就能用这个名字启动。",
  "managed-ok": "配置已写入，终端里用它的名字即可启动这套实例。",
  drifted: "配置文件被本工具以外改过，写入会覆盖它——确认后再接管。",
  disabled: "已停用，不再维护这套配置。",
  unsupported: "定义有问题，先修好再写入。",
};

export function ProfileCard({
  element,
  busy,
  onApply,
  onTakeover,
  onEdit,
  onRemove,
  onManageSkills,
  onSyncLaunch,
}: {
  element: EngineElement;
  busy: boolean;
  onApply: () => void;
  onTakeover: () => void;
  onEdit: () => void;
  onRemove: () => void;
  onManageSkills: () => void;
  onSyncLaunch: () => void;
}) {
  const spec = element.spec as unknown as ProfileSpec;
  const observed = element.observed as unknown as ProfileObserved;
  const id = element.id.replace(/^profiles:/, "");
  const blocked = observed.blocked_reason;
  const missingVars = (observed.env_refs ?? []).filter((ref) => ref.resolved === false);

  return (
    <section
      data-testid={`profile-card-${id}`}
      className="rounded-lg border bg-card p-5 shadow-sm transition-shadow hover:shadow-md"
    >
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <h3 className="text-base font-semibold text-foreground">
            {spec.display_name}
            <span className="ml-2 font-mono text-xs font-normal text-muted-foreground">{id}</span>
          </h3>
          <p className="mt-0.5 text-xs text-muted-foreground">
            {spec.base_agent} 的独立实例{spec.base_url ? ` · ${spec.base_url}` : ""}
          </p>
        </div>
        <StateBadge state={element.state} />
      </div>

      <p className="mt-2 text-xs text-muted-foreground">{blocked ?? STATE_NOTE[element.state] ?? ""}</p>

      <div className="mt-3 space-y-1.5 border-t pt-3 text-[11px] text-muted-foreground">
        <p className="flex items-start gap-1.5 break-all">
          <FolderTree className="mt-0.5 h-3 w-3 shrink-0" aria-hidden />
          <span className="font-mono">{observed.config_dir}</span>
          {!observed.config_dir_exists && <span className="shrink-0 text-warning">（尚未创建）</span>}
        </p>
        <p className="flex items-start gap-1.5 break-all">
          <Terminal className="mt-0.5 h-3 w-3 shrink-0" aria-hidden />
          <span data-testid={`profile-launch-${id}`} className="font-mono">
            alias {id}=&#39;{observed.launch_value}&#39;
          </span>
        </p>
      </div>

      {missingVars.length > 0 && (
        <p data-testid={`profile-missing-vars-${id}`} className="mt-2 rounded-md border border-warning/30 bg-warning/10 px-3 py-2 text-xs text-warning">
          这些环境变量当前取不到值，写入会被拒绝：{missingVars.map((ref) => ref.variable ?? ref.name).join("、")}
        </p>
      )}

      <div className="mt-3 flex flex-wrap items-center gap-2 border-t pt-3">
        <button
          type="button"
          data-testid={`profile-apply-${id}`}
          disabled={busy || element.actions.length === 0}
          onClick={element.state === "drifted" ? onTakeover : onApply}
          className={cn(
            "inline-flex items-center gap-1 rounded border px-2.5 py-1 text-xs",
            element.state === "drifted"
              ? "border-warning/40 bg-warning/10 text-warning hover:bg-warning/15"
              : "border-primary/30 bg-primary/10 text-primary hover:bg-primary/15",
            (busy || element.actions.length === 0) && "cursor-not-allowed opacity-50",
          )}
        >
          {busy ? <Loader2 className="h-3 w-3 animate-spin" aria-hidden /> : <PlayCircle className="h-3 w-3" aria-hidden />}
          {busy ? "执行中…" : element.state === "drifted" ? "接管并写入" : "写入配置"}
        </button>
        <button
          type="button"
          data-testid={`profile-sync-launch-${id}`}
          disabled={busy}
          onClick={onSyncLaunch}
          className="rounded border px-2.5 py-1 text-xs text-muted-foreground hover:bg-accent disabled:opacity-50"
        >
          同步启动命令
        </button>
        <button type="button" data-testid={`profile-edit-${id}`} onClick={onEdit} className="rounded border px-2.5 py-1 text-xs">
          编辑
        </button>
        <button
          type="button"
          data-testid={`profile-skills-${id}`}
          onClick={onManageSkills}
          className="inline-flex items-center gap-1 rounded border border-primary/30 bg-primary/10 px-2.5 py-1 text-xs text-primary hover:bg-primary/15"
        >
          <Sparkles className="h-3 w-3" aria-hidden /> 管理技能
        </button>
        <button
          type="button"
          data-testid={`profile-remove-${id}`}
          onClick={onRemove}
          className="ml-auto inline-flex items-center gap-1 rounded border border-destructive/30 px-2.5 py-1 text-xs text-destructive hover:bg-destructive/10"
        >
          <Trash2 className="h-3 w-3" aria-hidden /> 移除实例
        </button>
      </div>
    </section>
  );
}
