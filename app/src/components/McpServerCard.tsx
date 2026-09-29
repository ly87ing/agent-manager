import type { EngineElement } from "../lib/contract";
import type { McpAgentCell, McpObserved, McpSpec } from "../lib/observed";
import { ChevronRight } from "lucide-react";
import { McpEntryRow, mcpEntrySummary } from "./McpAssignmentRow";
import { cn } from "../lib/utils";

function variableName(placeholder: string): string {
  return /^\$\{([^}]+)\}$/.exec(placeholder)?.[1] ?? placeholder;
}

/** 条目里还没取到值的变量名（去重）。 */
export function missingVariables(element: EngineElement): string[] {
  const cells = Object.values((element.observed as McpObserved).agents ?? {});
  const names = cells.flatMap((cell) =>
    Object.values(cell.vars ?? {})
      .filter((resolution) => resolution.resolved === false)
      .map((resolution) => variableName(resolution.placeholder)),
  );
  return [...new Set(names)];
}

function savedSummary(spec: McpSpec): string {
  const def = spec.saved?.definition ?? {};
  if (typeof def.url === "string") return def.url;
  return [def.command ?? "", ...(def.args ?? [])].join(" ").trim();
}

/**
 * 一个服务一行：汇总在哪些账号上启用，点开是各账号配置里的条目（内容可以各不相同，各自编辑、启停），
 * 另附「已保存、还没写进任何账号」的定义。一切取值由引擎脱敏。
 */
export function McpServerCard({
  element,
  agents,
  busy,
  open,
  onToggle,
  exclusive = false,
  nameOf = (agent) => agent,
  onAction,
  onAddFromSaved,
  onForget,
  onConfigureSecrets,
}: {
  element: EngineElement;
  /** 可写的账号（status.targets 顺序）。 */
  agents: string[];
  busy: boolean;
  open: boolean;
  onToggle: () => void;
  /** 只看 agents 里的账号（按账号筛选时）。 */
  exclusive?: boolean;
  nameOf?: (agent: string) => string;
  onAction: (element: EngineElement, agent: string, action: string, cell: McpAgentCell) => void;
  onAddFromSaved: (element: EngineElement) => void;
  onForget: (element: EngineElement) => void;
  onConfigureSecrets?: (variable: string) => void;
}) {
  const spec = element.spec as unknown as McpSpec;
  const name = element.id.replace(/^mcp:/, "");
  const cells = (element.observed as McpObserved).agents ?? {};
  const ordered = [
    ...agents.filter((agent) => cells[agent]),
    ...(exclusive ? [] : Object.keys(cells).filter((agent) => !agents.includes(agent))),
  ];
  const missing = missingVariables(element);
  const enabled = ordered.filter((agent) => cells[agent].state === "enabled");
  const off = agents.filter((agent) => cells[agent]?.state !== "enabled").map(nameOf);
  const summary = ordered.length === 0
    ? "还没写进任何账号"
    : enabled.length === 0
      ? "已停用"
      : off.length === 0
        ? "全部账号"
        : off.length <= 2
          ? `全部账号（${off.join("、")} 除外）`
          : `${enabled.length} 个账号`;
  const first = ordered[0] ? cells[ordered[0]] : null;

  return (
    <li data-testid={`mcp-card-${element.id}`}>
      <div className="flex items-center gap-2 px-4 py-2">
        <button
          type="button"
          aria-expanded={open}
          aria-label={`${name} 在各账号上的条目`}
          onClick={onToggle}
          className="rounded p-0.5 text-muted-foreground hover:bg-accent"
        >
          <ChevronRight className={cn("h-4 w-4 transition-transform", open && "rotate-90")} aria-hidden />
        </button>
        <span className="font-mono text-sm">{name}</span>
        {first && (
          <code className="min-w-0 truncate font-mono text-xs text-muted-foreground" title={mcpEntrySummary(first)}>
            {mcpEntrySummary(first)}
          </code>
        )}
        {missing.length > 0 && onConfigureSecrets && (
          <button
            type="button"
            data-testid={`configure-secrets-${element.id}`}
            onClick={() => onConfigureSecrets(missing[0])}
            className="shrink-0 rounded bg-warning/15 px-1.5 py-0.5 text-[11px] text-warning hover:bg-warning/25"
          >
            {missing.length} 个变量没有取值 · 填写
          </button>
        )}
        <span data-testid={`mcp-summary-${element.id}`} className="ml-auto shrink-0 text-xs text-muted-foreground">{summary}</span>
      </div>

      {open && ordered.length > 0 && (
        <ul className="divide-y divide-border/70 bg-muted/20 px-11 py-2">
          {ordered.map((agent) => (
            <McpEntryRow
              key={agent}
              element={element}
              agent={agent}
              label={nameOf(agent)}
              cell={cells[agent]}
              disabled={busy}
              onAction={onAction}
            />
          ))}
        </ul>
      )}

      {spec.saved && (
        <div
          data-testid={`mcp-saved-${element.id}`}
          className={cn("mx-4 mb-2 rounded-md border px-3 py-2", spec.saved.valid ? "border-dashed bg-muted/40" : "border-warning/30 bg-warning/5")}
        >
          <div className="flex flex-wrap items-center justify-between gap-2">
            <div className="min-w-0 text-xs">
              <span className="font-medium text-foreground">已保存的定义（还没写进任何账号）</span>
              <code className="ml-2 font-mono text-muted-foreground">{savedSummary(spec)}</code>
              {!spec.saved.valid && <span className="ml-2 text-warning">内容不完整，只能删除</span>}
            </div>
            <div className="flex items-center gap-1.5">
              {spec.saved.valid && (
                <button
                  type="button"
                  data-testid={`add-from-saved-${element.id}`}
                  onClick={() => onAddFromSaved(element)}
                  disabled={busy}
                  className="rounded border border-primary/30 bg-primary/10 px-2 py-0.5 text-xs text-primary hover:bg-primary/15 disabled:opacity-50"
                >
                  添加到账号
                </button>
              )}
              <button
                type="button"
                data-testid={`forget-${element.id}`}
                onClick={() => onForget(element)}
                disabled={busy}
                className="rounded border border-destructive/30 bg-card px-2 py-0.5 text-xs text-destructive hover:bg-destructive/10 disabled:opacity-50"
              >
                删除保存项
              </button>
            </div>
          </div>
        </div>
      )}
    </li>
  );
}
