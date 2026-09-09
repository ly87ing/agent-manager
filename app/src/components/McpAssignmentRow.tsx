import type { EngineElement } from "../lib/contract";
import type { McpAgentCell } from "../lib/observed";
import { cn } from "../lib/utils";

/** 服务 × 助手 单元格扩展：合同已定 assigned/enabled/version/available_version/actions/blocked_reason
 * （observed.ts 冻结，本地扩展镜像引擎 payload）。 */
export type McpAgentCellView = McpAgentCell & {
  assigned?: boolean;
  enabled?: boolean;
  version?: string | null;
  available_version?: string | null;
  actions?: string[];
  blocked_reason?: string | null;
};

function shortDigest(value: string | null | undefined): string {
  if (!value) return "?";
  return value.length > 12 ? value.slice(0, 7) : value;
}

/** 单元格状态文案与样式；异常态优先于 assigned/enabled，enabled=true 不能掩盖异常。 */
const CELL_VIEW: Record<string, { label: string; className: string }> = {
  drifted: { label: "不一致", className: "border-amber-200 bg-amber-50 text-amber-700" },
  leftover: { label: "残留", className: "border-purple-200 bg-purple-50 text-purple-700" },
  "probe-error": { label: "检查出错", className: "border-rose-200 bg-rose-50 text-rose-700" },
  "managed-ok": { label: "已配置", className: "border-emerald-200 bg-emerald-50 text-emerald-700" },
  "not-materialized": { label: "尚未生成", className: "border-slate-200 bg-slate-50 text-slate-600" },
  missing: { label: "未配置", className: "border-slate-200 bg-slate-50 text-slate-500" },
  absent: { label: "未配置", className: "border-slate-200 bg-slate-50 text-slate-500" },
  "agent-disabled": { label: "未配置", className: "border-slate-200 bg-slate-50 text-slate-500" },
  disabled: { label: "已停用", className: "border-zinc-200 bg-zinc-100 text-zinc-600" },
};

export function McpCellStatusBadge({ agent, cell }: { agent: string; cell: McpAgentCellView | null }) {
  let label = "未配置";
  let state = "未配置";
  let className = "border-slate-200 bg-slate-50 text-slate-400";

  if (cell) {
    state = cell.state;
    const view = CELL_VIEW[cell.state];
    if (view) {
      label = view.label;
      className = view.className;
    } else if (cell.assigned && cell.enabled === false) {
      label = "已停用";
      className = "border-zinc-200 bg-zinc-100 text-zinc-600";
    } else if (cell.assigned && cell.enabled) {
      label = "已配置";
      className = "border-emerald-200 bg-emerald-50 text-emerald-700";
    } else {
      label = "未配置";
      className = "border-slate-200 bg-slate-50 text-slate-500";
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

const MCP_ACTIONS: { action: string; label: string; className: string }[] = [
  { action: "add", label: "添加", className: "border-indigo-200 bg-indigo-50 text-indigo-700 hover:bg-indigo-100" },
  { action: "update", label: "更新", className: "border-indigo-200 bg-indigo-50 text-indigo-700 hover:bg-indigo-100" },
  { action: "enable", label: "启用", className: "border-emerald-200 bg-emerald-50 text-emerald-700 hover:bg-emerald-100" },
  { action: "disable", label: "禁用", className: "border-amber-200 bg-amber-50 text-amber-700 hover:bg-amber-100" },
  { action: "remove", label: "从此助手删除", className: "border-red-200 bg-white text-red-600 hover:bg-red-50" },
];

/**
 * 「助手中的服务」一行（服务 × 助手）。操作按钮严格按引擎报告的 cell.actions
 * 子集渲染（add/update/enable/disable/remove），更新与启用不混；cell 缺失时
 * 不伪造操作，明确显示未配置。版本显示短摘要，完整值在 title。
 */
export function McpAssignmentRow({
  element,
  agent,
  cell,
  onOpenDetail,
  onAction,
}: {
  element: EngineElement;
  agent: string;
  cell: McpAgentCellView | null;
  onOpenDetail: (id: string) => void;
  onAction: (element: EngineElement, agent: string, action: string) => void;
}) {
  const actions = cell?.actions ?? [];

  return (
    <tr className="border-b border-slate-100 last:border-0 hover:bg-slate-50/50">
      <td className="py-2.5 pr-4">
        <button
          type="button"
          onClick={() => onOpenDetail(element.id)}
          className="font-mono text-xs font-medium break-all text-indigo-600 hover:text-indigo-800"
          title="查看服务定义详情"
        >
          {element.id}
        </button>
      </td>
      <td className="py-2.5 pr-4 font-mono text-xs text-slate-700">{agent}</td>
      <td className="py-2.5 pr-4 font-mono text-xs whitespace-nowrap text-slate-500" title={cell?.available_version ?? undefined}>
        {cell?.available_version ? shortDigest(cell.available_version) : "未获取"}
      </td>
      <td className="py-2.5 pr-4 font-mono text-xs whitespace-nowrap text-slate-600" title={cell?.version ?? undefined}>
        {cell ? (cell.version ? shortDigest(cell.version) : cell.assigned ? "已配置" : "未配置") : "—"}
      </td>
      <td className="py-2.5 pr-4">
        <div className="flex flex-col gap-1">
          <McpCellStatusBadge agent={agent} cell={cell} />
          {cell?.blocked_reason && (
            <span className="text-xs text-amber-600" title={cell.blocked_reason}>
              {cell.blocked_reason}
            </span>
          )}
        </div>
      </td>
      <td className="py-2.5">
        <div className="flex flex-wrap items-center gap-1.5">
          {MCP_ACTIONS.filter(({ action }) => actions.includes(action)).map(({ action, label, className }) => (
            <button
              key={action}
              type="button"
              data-testid={`action-${action}-${element.id}-${agent}`}
              onClick={() => onAction(element, agent, action)}
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
