import type { EngineElement } from "../lib/contract";
import type { McpAgentCell, McpObserved, McpSpec } from "../lib/observed";
import { FreshnessBadge } from "./FreshnessBadge";
import { McpEntryRow } from "./McpAssignmentRow";
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
 * 一个服务名：各助手配置里的同名条目逐行列出（内容可以各不相同，各自编辑、各自启停），
 * 另附「已保存、还没写进任何助手」的定义。一切取值由引擎脱敏。
 */
export function McpServerCard({
  element,
  agents,
  busy,
  onAction,
  onAddFromSaved,
  onForget,
  onConfigureSecrets,
}: {
  element: EngineElement;
  /** 列顺序：status.targets 的助手顺序。 */
  agents: string[];
  busy: boolean;
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
    ...Object.keys(cells).filter((agent) => !agents.includes(agent)),
  ];
  const missing = missingVariables(element);

  return (
    <section
      data-testid={`mcp-card-${element.id}`}
      className="rounded-lg border bg-card p-5 shadow-sm transition-shadow hover:shadow-md"
    >
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <h3 className="font-mono text-xs font-semibold break-all text-primary">{name}</h3>
          <p className="mt-1 text-xs text-muted-foreground">
            {ordered.length > 0 ? `${ordered.length} 个助手配置了这个服务` : "还没有写进任何助手"}
          </p>
        </div>
        <FreshnessBadge freshness={element.freshness} className="items-end" />
      </div>

      {missing.length > 0 && onConfigureSecrets && (
        <div className="mt-3 flex flex-wrap items-center gap-2 text-xs">
          <span className="rounded border border-warning/30 bg-warning/10 px-1.5 py-0.5 text-warning">
            {missing.length} 个变量还没有取值
          </span>
          <button
            type="button"
            data-testid={`configure-secrets-${element.id}`}
            onClick={() => onConfigureSecrets(missing[0])}
            className="rounded border border-primary/30 bg-primary/10 px-1.5 py-0.5 font-medium text-primary hover:bg-primary/15"
          >
            填写缺失变量{missing.length > 1 ? `（${missing.length}）` : ""}
          </button>
        </div>
      )}

      {ordered.length > 0 && (
        <ul className="mt-3 divide-y divide-border/70">
          {ordered.map((agent) => (
            <McpEntryRow
              key={agent}
              element={element}
              agent={agent}
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
          className={cn(
            "mt-3 rounded-md border px-3 py-2",
            spec.saved.valid ? "border-dashed bg-muted/40" : "border-warning/30 bg-warning/5",
          )}
        >
          <div className="flex flex-wrap items-center justify-between gap-2">
            <div className="min-w-0 text-xs">
              <span className="font-medium text-foreground">已保存的定义（还没写进任何助手）</span>
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
                  添加到助手
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
    </section>
  );
}
