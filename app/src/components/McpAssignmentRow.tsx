import type { EngineElement } from "../lib/contract";
import { MCP_UNMANAGED_CONFLICT_MARKER } from "../lib/mcpImport";
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
  drifted: { label: "不一致", className: "border-warning/30 bg-warning/10 text-warning" },
  // 「残留」对用户没有意义：说清是什么处境——以前配过、现在没在管
  leftover: { label: "以前配过", className: "border-violet/30 bg-violet/10 text-violet" },
  "probe-error": { label: "检查出错", className: "border-rose/30 bg-rose/10 text-rose" },
  "managed-ok": { label: "已配置", className: "border-success/30 bg-success/10 text-success" },
  "not-materialized": { label: "尚未生成", className: "border bg-muted text-muted-foreground" },
  missing: { label: "未配置", className: "border bg-muted text-muted-foreground" },
  absent: { label: "未配置", className: "border bg-muted text-muted-foreground" },
  "agent-disabled": { label: "未配置", className: "border bg-muted text-muted-foreground" },
  disabled: { label: "已停用", className: "border bg-muted text-muted-foreground" },
};

export function McpCellStatusBadge({ agent, cell }: { agent: string; cell: McpAgentCellView | null }) {
  let label = "未配置";
  let state = "未配置";
  let className = "border bg-muted text-muted-foreground";

  if (cell) {
    state = cell.state;
    const view = CELL_VIEW[cell.state];
    if (view) {
      label = view.label;
      className = view.className;
    } else if (cell.assigned && cell.enabled === false) {
      label = "已停用";
      className = "border bg-muted text-muted-foreground";
    } else if (cell.assigned && cell.enabled) {
      label = "已配置";
      className = "border-success/30 bg-success/10 text-success";
    } else {
      label = "未配置";
      className = "border bg-muted text-muted-foreground";
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
  { action: "add", label: "添加", className: "border-primary/30 bg-primary/10 text-primary hover:bg-primary/15" },
  { action: "update", label: "更新", className: "border-primary/30 bg-primary/10 text-primary hover:bg-primary/15" },
  { action: "enable", label: "启用", className: "border-success/30 bg-success/10 text-success hover:bg-success/15" },
  { action: "disable", label: "禁用", className: "border-warning/30 bg-warning/10 text-warning hover:bg-warning/15" },
  { action: "remove", label: "从此助手删除", className: "border-destructive/30 bg-card text-destructive hover:bg-destructive/10" },
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
  onTakeover,
}: {
  element: EngineElement;
  agent: string;
  cell: McpAgentCellView | null;
  onOpenDetail: (id: string) => void;
  onAction: (element: EngineElement, agent: string, action: string) => void;
  /** 显式接管入口：仅在 cell 因「非受管同名条目」fail-closed 且无动作时渲染。 */
  onTakeover?: (element: EngineElement, agent: string, action: string) => void;
}) {
  const actions = cell?.actions ?? [];
  const takeoverBlocked =
    cell !== null && actions.length === 0 && (cell.blocked_reason ?? "").includes(MCP_UNMANAGED_CONFLICT_MARKER);

  return (
    <tr className="border-b last:border-0 hover:bg-muted/50">
      <td className="py-2.5 pr-4">
        <button
          type="button"
          onClick={() => onOpenDetail(element.id)}
          className="font-mono text-xs font-medium break-all text-primary hover:text-primary"
          title="查看服务定义详情"
        >
          {element.id}
        </button>
      </td>
      <td className="py-2.5 pr-4 font-mono text-xs text-foreground">{agent}</td>
      <td className="py-2.5 pr-4 font-mono text-xs whitespace-nowrap text-muted-foreground" title={cell?.available_version ?? undefined}>
        {cell?.available_version ? shortDigest(cell.available_version) : "未获取"}
      </td>
      <td className="py-2.5 pr-4 font-mono text-xs whitespace-nowrap text-muted-foreground" title={cell?.version ?? undefined}>
        {cell ? (cell.version ? shortDigest(cell.version) : cell.assigned ? "已配置" : "未配置") : "—"}
      </td>
      <td className="py-2.5 pr-4">
        <div className="flex flex-col gap-1">
          <McpCellStatusBadge agent={agent} cell={cell} />
          {cell?.blocked_reason && (
            <span className="text-xs text-warning" title={cell.blocked_reason}>
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
          {takeoverBlocked && onTakeover && (
            <button
              type="button"
              data-testid={`takeover-${element.id}-${agent}`}
              title="以本工具的标准配置替换目标文件中的同名外部条目（其余条目与字段保持不变）"
              onClick={() => onTakeover(element, agent, cell.assigned ? "update" : "add")}
              className="rounded border border-warning/40 bg-warning/10 px-2 py-0.5 text-xs font-medium text-warning hover:bg-warning/15"
            >
              接管原条目
            </button>
          )}
          {/* 没有可执行动作时说清「为什么不能操作」，而不是报告引擎的内部情况——
              用户不关心引擎提供不提供，他要知道的是接下来该做什么。 */}
          {cell && actions.length === 0 && !takeoverBlocked && (
            <span className="text-xs text-muted-foreground">
              {cell.blocked_reason ?? "这里没有可做的：状态正常或该项已停用"}
            </span>
          )}
          {!cell && <span className="text-xs text-muted-foreground">未配置</span>}
        </div>
      </td>
    </tr>
  );
}
