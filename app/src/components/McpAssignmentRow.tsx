import { Check, X } from "lucide-react";
import type { EngineElement } from "../lib/contract";
import type { McpAgentCell } from "../lib/observed";
import { MCP_CELL_COPY } from "../lib/ui-copy";
import { cn } from "../lib/utils";

/** 矩阵与徽章用的状态键：快照单独记作 saved（「已停用（已保存）」）。 */
export function mcpCellKey(cell: McpAgentCell): string {
  return cell.source === "saved" && cell.state === "disabled" ? "saved" : cell.state;
}

const BADGE_STYLE: Record<string, string> = {
  enabled: "border-success/30 bg-success/10 text-success",
  disabled: "border bg-muted text-muted-foreground",
  saved: "border bg-muted text-muted-foreground",
  invalid: "border-rose/30 bg-rose/10 text-rose",
};

export function McpCellStatusBadge({ agent, cell }: { agent: string; cell: McpAgentCell | null }) {
  const key = cell ? mcpCellKey(cell) : "absent";
  const label = cell ? (MCP_CELL_COPY[key] ?? cell.state) : "未配置";
  return (
    <span
      title={`${agent}: ${key}`}
      className={cn(
        "inline-flex items-center rounded border px-2 py-0.5 text-xs font-medium",
        BADGE_STYLE[key] ?? "border bg-muted text-muted-foreground",
      )}
    >
      {label}
    </span>
  );
}

/** 条目一句话摘要：stdio 为命令行，远程为地址。 */
export function mcpEntrySummary(cell: McpAgentCell): string {
  const entry = cell.entry ?? {};
  if (cell.transport === "remote") return entry.url ?? "";
  return [entry.command ?? "", ...(entry.args ?? [])].join(" ").trim();
}

const ACTIONS: { action: string; label: string; className: string }[] = [
  { action: "edit", label: "编辑", className: "border bg-card text-muted-foreground hover:bg-accent" },
  { action: "enable", label: "启用", className: "border-success/30 bg-success/10 text-success hover:bg-success/15" },
  { action: "disable", label: "停用", className: "border-warning/30 bg-warning/10 text-warning hover:bg-warning/15" },
  { action: "copy", label: "复制到…", className: "border-primary/30 bg-primary/10 text-primary hover:bg-primary/15" },
  { action: "remove", label: "删除", className: "border-destructive/30 bg-card text-destructive hover:bg-destructive/10" },
];

/** 逐条动作：按钮严格按引擎报告的 cell.actions 渲染。 */
export function McpCellActions({
  element,
  agent,
  cell,
  disabled,
  onAction,
}: {
  element: EngineElement;
  agent: string;
  cell: McpAgentCell;
  disabled?: boolean;
  onAction: (element: EngineElement, agent: string, action: string, cell: McpAgentCell) => void;
}) {
  const actions = cell.actions ?? [];
  return (
    <div className="flex flex-wrap items-center gap-1.5">
      {ACTIONS.filter(({ action }) => actions.includes(action)).map(({ action, label, className }) => (
        <button
          key={action}
          type="button"
          data-testid={`action-${action}-${element.id}-${agent}`}
          onClick={() => onAction(element, agent, action, cell)}
          disabled={disabled}
          className={cn("rounded border px-2 py-0.5 text-xs disabled:opacity-50", className)}
        >
          {label}
        </button>
      ))}
      {actions.length === 0 && (
        <span className="text-xs text-muted-foreground">{cell.blocked_reason ?? "这里没有可做的"}</span>
      )}
    </div>
  );
}

/** 变量引用的解析状态（只显示变量名与是否已配置，永不显示取值）。 */
function VarList({ cell }: { cell: McpAgentCell }) {
  const entries = Object.entries(cell.vars ?? {});
  if (entries.length === 0) return null;
  return (
    <ul className="flex flex-wrap gap-x-3 gap-y-0.5">
      {entries.map(([key, resolution]) => (
        <li key={key} className="flex items-center gap-1 text-[11px]" data-testid="mcp-var-resolution">
          {resolution.resolved ? (
            <Check className="h-3 w-3 text-success" aria-label="已配置" />
          ) : (
            <X className="h-3 w-3 text-destructive" aria-label="未配置" />
          )}
          <span className="font-mono text-muted-foreground">{key}</span>
          <code className="font-mono text-foreground">{resolution.placeholder}</code>
        </li>
      ))}
    </ul>
  );
}

/**
 * 服务 × 助手 一行：该助手配置里这一条的状态、摘要、保留字段与可做的动作。
 * 取值一律脱敏（引擎已处理）；未建模字段只列名字，写入时原样保留。
 */
export function McpEntryRow({
  element,
  agent,
  label,
  cell,
  disabled,
  onAction,
}: {
  element: EngineElement;
  agent: string;
  /** 显示用的账号名；缺省显示 agent id。 */
  label?: string;
  cell: McpAgentCell;
  disabled?: boolean;
  onAction: (element: EngineElement, agent: string, action: string, cell: McpAgentCell) => void;
}) {
  const extraKeys = Object.keys(cell.extra ?? {}).filter((key) => key !== "type");
  return (
    <li data-testid={`mcp-entry-${element.id}-${agent}`} className="space-y-1 py-2 first:pt-0 last:pb-0">
      <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
        <span className="min-w-24 text-xs font-medium text-foreground" title={agent}>{label ?? agent}</span>
        <McpCellStatusBadge agent={agent} cell={cell} />
        {cell.transport === "remote" && (
          <span className="rounded border border-info/30 bg-info/10 px-1.5 py-0.5 text-[10px] text-info">远程 HTTP</span>
        )}
        <code className="min-w-0 flex-1 truncate font-mono text-xs text-muted-foreground" title={mcpEntrySummary(cell)}>
          {mcpEntrySummary(cell)}
        </code>
        <McpCellActions element={element} agent={agent} cell={cell} disabled={disabled} onAction={onAction} />
      </div>
      <VarList cell={cell} />
      {extraKeys.length > 0 && (
        <p className="text-[11px] text-muted-foreground">另有字段原样保留：{extraKeys.join("、")}</p>
      )}
      {cell.command_available === false && (
        <p data-testid={`command-missing-${element.id}-${agent}`} className="text-[11px] text-warning">
          本机找不到它要执行的命令，助手启动这个服务时会失败。
        </p>
      )}
      {cell.notice && <p className="text-[11px] text-muted-foreground">{cell.notice}</p>}
      {cell.blocked_reason && (cell.actions?.length ?? 0) > 0 && (
        <p className="text-[11px] text-warning">{cell.blocked_reason}</p>
      )}
      {cell.target && <p className="break-all font-mono text-[10px] text-muted-foreground/80">{cell.target}</p>}
    </li>
  );
}
