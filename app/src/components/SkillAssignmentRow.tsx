import type { EngineElement } from "../lib/contract";
import type { SkillAgentCell } from "../lib/observed";
import { cn } from "../lib/utils";

function shortCommit(value: string | null | undefined): string {
  if (!value) return "?";
  return value.length > 12 ? value.slice(0, 7) : value;
}

/**
 * 技能 × 助手 分配状态徽标。异常态（冲突/共用/不一致/不支持）优先于
 * assigned/enabled 标志显示，enabled=true 不能掩盖异常；cell 缺失（引擎未报告
 * 此助手路径）显示「未配置」，不伪造可执行状态。
 */
export function SkillStatusBadge({ agent, cell }: { agent: string; cell: SkillAgentCell | null }) {
  let label = "未配置";
  let state = "未配置";
  let className = "border-slate-200 bg-slate-50 text-slate-400";

  if (cell) {
    state = cell.state;
    label = "未分配";
    className = "border-slate-200 bg-slate-50 text-slate-500";

    if (state === "conflict") {
      label = "冲突";
      className = "border-rose-200 bg-rose-50 text-rose-700";
    } else if (state === "shared") {
      label = "共用（异常）";
      className = "border-amber-200 bg-amber-50 text-amber-700";
    } else if (state === "drifted") {
      label = "不一致";
      className = "border-amber-200 bg-amber-50 text-amber-700";
    } else if (state === "unsupported") {
      label = "不支持";
      className = "border-slate-300 bg-slate-100 text-slate-600";
    } else if (state === "dead-link") {
      label = "失效链接";
      className = "border-rose-200 bg-rose-50 text-rose-700";
    } else if (state === "missing-link") {
      label = "未生效";
      className = "border-amber-200 bg-amber-50 text-amber-700";
    } else if (state === "not-materialized") {
      label = "尚未生成";
      className = "border-slate-200 bg-slate-50 text-slate-600";
    } else if (state === "extra-link") {
      label = "计划外链接";
      className = "border-purple-200 bg-purple-50 text-purple-700";
    } else if (state === "disabled") {
      label = "已停用";
      className = "border-zinc-200 bg-zinc-100 text-zinc-600";
    } else if (state === "linked") {
      label = "已启用";
      className = "border-emerald-200 bg-emerald-50 text-emerald-700";
    } else if (cell.assigned && cell.enabled === false) {
      label = "已停用";
      className = "border-zinc-200 bg-zinc-100 text-zinc-600";
    } else if (cell.assigned && cell.enabled) {
      label = "已分配";
      className = "border-emerald-200 bg-emerald-50 text-emerald-700";
    }
  }

  return (
    <span
      title={`${agent}: ${state}`}
      className={cn("inline-flex items-center rounded border px-2 py-0.5 text-xs font-medium", className)}
    >
      {label}
    </span>
  );
}

const SKILL_ACTIONS: { action: string; label: string; className: string }[] = [
  { action: "add", label: "添加", className: "border-indigo-200 bg-indigo-50 text-indigo-700 hover:bg-indigo-100" },
  { action: "update", label: "更新", className: "border-indigo-200 bg-indigo-50 text-indigo-700 hover:bg-indigo-100" },
  { action: "enable", label: "启用", className: "border-emerald-200 bg-emerald-50 text-emerald-700 hover:bg-emerald-100" },
  { action: "disable", label: "禁用", className: "border-amber-200 bg-amber-50 text-amber-700 hover:bg-amber-100" },
  { action: "remove", label: "删除分配", className: "border-red-200 bg-white text-red-600 hover:bg-red-50" },
];

/**
 * 全部技能列表中的一行（技能 × 助手）。操作按钮严格按引擎报告的
 * cell.actions 子集渲染（add/update/enable/disable/remove），更新与启用不混；
 * cell 缺失时不伪造操作，明确显示未配置。
 */
export function SkillAssignmentRow({
  skill,
  skillName,
  collection,
  agent,
  cell,
  onFilterCollection,
  onFilterAgent,
  onAction,
}: {
  skill: EngineElement;
  skillName: string;
  collection: string;
  agent: string;
  cell: SkillAgentCell | null;
  onFilterCollection: (collection: string) => void;
  onFilterAgent: (agent: string) => void;
  onAction: (skill: EngineElement, agent: string, action: string) => void;
}) {
  const actions = cell?.actions ?? [];

  return (
    <tr className="border-b border-slate-100 last:border-0 hover:bg-slate-50/50">
      <td className="py-2.5 pr-4">
        <span className="font-mono text-xs font-medium text-slate-700">{skillName}</span>
      </td>
      <td className="py-2.5 pr-4">
        <button
          type="button"
          onClick={() => onFilterCollection(collection)}
          className="font-mono text-xs text-slate-600 hover:text-indigo-600 hover:underline"
          title={`筛选仅查看集合 ${collection}`}
        >
          {collection}
        </button>
      </td>
      <td className="py-2.5 pr-4 font-mono text-xs text-slate-700">
        <button
          type="button"
          onClick={() => onFilterAgent(agent)}
          className="hover:text-indigo-600 hover:underline"
          title={`筛选仅查看助手 ${agent}`}
        >
          {agent}
        </button>
      </td>
      <td className="py-2.5 pr-4 font-mono text-xs whitespace-nowrap text-slate-500" title={cell?.available_version ?? undefined}>
        {cell?.available_version ? shortCommit(cell.available_version) : "未获取"}
      </td>
      <td className="py-2.5 pr-4 font-mono text-xs whitespace-nowrap text-slate-600" title={cell?.version ?? undefined}>
        {cell ? (cell.version ? shortCommit(cell.version) : cell.link_path ? "已链接" : cell.assigned ? "已分配" : "未分配") : "—"}
      </td>
      <td className="py-2.5 pr-4">
        <div className="flex flex-col gap-1">
          <SkillStatusBadge agent={agent} cell={cell} />
          {cell?.blocked_reason && (
            <span className="text-xs text-amber-600" title={cell.blocked_reason}>
              {cell.blocked_reason}
            </span>
          )}
        </div>
      </td>
      <td className="py-2.5">
        <div className="flex flex-wrap items-center gap-1.5">
          {SKILL_ACTIONS.filter(({ action }) => actions.includes(action)).map(({ action, label, className }) => (
            <button
              key={action}
              type="button"
              data-testid={`action-${action}-${skill.id}-${agent}`}
              onClick={() => onAction(skill, agent, action)}
              className={cn("rounded border px-2 py-0.5 text-xs", className)}
            >
              {label}
            </button>
          ))}
          {cell && actions.length === 0 && <span className="text-xs text-slate-400">当前引擎未提供操作</span>}
          {!cell && <span className="text-xs text-slate-400">未配置</span>}
        </div>
      </td>
    </tr>
  );
}
