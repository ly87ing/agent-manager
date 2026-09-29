import type { EngineElement } from "../lib/contract";
import type { McpAgentCell, McpObserved, McpSpec } from "../lib/observed";
import { Pencil, Share2, Trash2 } from "lucide-react";
import type { AccountGroup } from "../lib/accounts";
import { AccountCells, AccountSwitch, RowActions, type CellTone } from "./AccountGrid";
import { mcpBrief, mcpEntrySummary } from "./McpAssignmentRow";
import { missingVariables } from "../lib/mcpVariables";
import { cn } from "../lib/utils";

function savedSummary(spec: McpSpec): string {
  const def = spec.saved?.definition ?? {};
  if (typeof def.url === "string") return def.url;
  return [def.command ?? "", ...(def.args ?? [])].join(" ").trim();
}

function mcpChip(cell: McpAgentCell | undefined, canAddHere: boolean): { tone: CellTone; checked: boolean; hint: string; primary: string | null } {
  if (!cell) {
    return canAddHere
      ? { tone: "off", checked: false, hint: "这个账号还没有，点击从其他账号复制过来", primary: "add-here" }
      : { tone: "blocked", checked: false, hint: "这个账号还没有这个服务", primary: null };
  }
  const has = (action: string) => cell.actions.includes(action);
  if (cell.state === "invalid") {
    return { tone: "warn", checked: false, hint: cell.blocked_reason ?? "条目结构无法识别，只能删除或在配置文件里修正", primary: null };
  }
  if (cell.state === "enabled") {
    const missing = cell.command_available === false;
    return {
      tone: missing ? "warn" : "on",
      checked: true,
      hint: missing ? "已启用，但本机找不到它要执行的命令；点击停用" : "已启用，点击停用",
      primary: has("disable") ? "disable" : null,
    };
  }
  return {
    tone: "off",
    checked: false,
    hint: cell.source === "saved" ? "已停用（内容保存在本工具里），点击启用" : "已停用，点击启用",
    primary: has("enable") ? "enable" : null,
  };
}

/** 给还没有这个服务的账号复制时用哪一份：优先同格式（同一助手）的，其次任何能复制的。 */
export function copySource(cells: Record<string, McpAgentCell>, formatOf: (agent: string) => string | undefined, to: string): string | null {
  const candidates = Object.entries(cells).filter(([, cell]) => cell.source === "file" && cell.actions.includes("copy"));
  const same = candidates.find(([agent]) => formatOf(agent) === formatOf(to));
  return (same ?? candidates[0])?.[0] ?? null;
}

/**
 * 一个服务一行：左边是名字和一句话说明，右边在与表头对齐的列里每个账号一个开关
 * （点一下启用 / 停用；没有这个服务的账号点一下从其他账号复制过来）。
 * 编辑、删除各一个，鼠标移到行上才显示。另附「已保存、还没写进任何账号」的定义。取值一律由引擎脱敏。
 */
export function McpServerCard({
  element,
  groups,
  writable,
  busy,
  formatOf,
  onAction,
  onAddHere,
  onEdit,
  onShare,
  onRemove,
  onAddFromSaved,
  onForget,
  onConfigureSecrets,
}: {
  element: EngineElement;
  groups: AccountGroup[];
  /** 配置文件读写正常的账号。 */
  writable: string[];
  busy: boolean;
  formatOf: (agent: string) => string | undefined;
  onAction: (element: EngineElement, agent: string, action: string, cell: McpAgentCell) => void;
  /** 这个账号还没有：从 from 账号的条目复制过来。 */
  onAddHere: (element: EngineElement, from: string, to: string) => void;
  onEdit: (element: EngineElement) => void;
  onShare?: (element: EngineElement) => void;
  onRemove: (element: EngineElement) => void;
  onAddFromSaved: (element: EngineElement) => void;
  onForget: (element: EngineElement) => void;
  onConfigureSecrets?: (variable: string) => void;
}) {
  const spec = element.spec as unknown as McpSpec;
  const name = element.id.replace(/^mcp:/, "");
  const cells = (element.observed as McpObserved).agents ?? {};
  const missing = missingVariables(element);
  const summaries = new Set(Object.values(cells).map(mcpEntrySummary));
  // 说明取一份能读出命令或地址的条目；读不出的（结构无法识别）不拿来写说明
  const described = Object.values(cells).filter((cell) => cell.state !== "invalid" && (cell.entry?.command || cell.entry?.url));
  const first = described.find((cell) => cell.source === "file") ?? described[0];
  const invalid = Object.values(cells).some((cell) => cell.state === "invalid");
  const editable = Object.values(cells).some((cell) => cell.actions.includes("edit"));
  const removable = Object.values(cells).some((cell) => cell.actions.includes("remove"));
  const commandMissing = Object.values(cells).some((cell) => cell.state === "enabled" && cell.command_available === false);

  return (
    <li data-testid={`mcp-card-${element.id}`}>
      <div className="group flex items-center gap-3 py-2 pl-4 pr-4 hover:bg-accent/40">
        <div className="min-w-0 flex-1">
          <div className="font-mono text-sm font-medium">{name}</div>
          <div className="flex flex-wrap items-center gap-x-2 gap-y-0.5 text-xs text-muted-foreground">
            {first && <span className="truncate" title={mcpEntrySummary(first)}>{mcpBrief(first)}</span>}
            {summaries.size > 1 && <span className="text-warning" title="保存编辑时会统一成一份">⚠ 各账号内容不同</span>}
            {commandMissing && <span className="text-warning">⚠ 有账号找不到它要执行的命令</span>}
            {invalid && <span className="text-warning" title="只能删除或在配置文件里修正">⚠ 有账号的条目无法识别</span>}
            {missing.length > 0 && onConfigureSecrets && (
              <button
                type="button"
                data-testid={`configure-secrets-${element.id}`}
                onClick={() => onConfigureSecrets(missing[0])}
                className="text-warning underline-offset-2 hover:underline"
              >
                ⚠ {missing.length} 个变量没有取值 · 填写
              </button>
            )}
          </div>
        </div>
        <AccountCells
          groups={groups}
          cell={(column) => {
            const cell = cells[column.id];
            const canWrite = writable.includes(column.id);
            const source = canWrite && !cell ? copySource(cells, formatOf, column.id) : null;
            const chip = canWrite
              ? mcpChip(cell, Boolean(source))
              : { tone: "blocked" as const, checked: cell?.state === "enabled", hint: cell?.blocked_reason ?? "这个账号的配置文件现在读写不了", primary: null };
            const detail = cell ? `\n${mcpEntrySummary(cell)}` : "";
            return (
              <AccountSwitch
                tone={chip.tone}
                checked={chip.checked}
                title={`${column.fullName}：${chip.hint}${detail}`}
                testId={chip.primary ? `action-${chip.primary}-${element.id}-${column.id}` : `chip-${element.id}-${column.id}`}
                disabled={busy}
                onClick={
                  chip.primary === "add-here" && source
                    ? () => onAddHere(element, source, column.id)
                    : chip.primary && cell
                      ? () => onAction(element, column.id, chip.primary as string, cell)
                      : undefined
                }
              />
            );
          }}
        />
        <RowActions>
          {editable && (
            <button
              type="button"
              data-testid={`edit-${element.id}`}
              aria-label={`编辑 ${name}`}
              title="编辑（写到所有有它的账号）"
              onClick={() => onEdit(element)}
              disabled={busy}
              className="rounded p-1.5 text-muted-foreground hover:bg-accent hover:text-foreground"
            >
              <Pencil className="h-3.5 w-3.5" aria-hidden />
            </button>
          )}
          {onShare && first && (
            <button
              type="button"
              data-testid={`share-${element.id}`}
              aria-label={`复制 ${name} 的分享链接`}
              title="复制分享链接（不含明文取值）"
              onClick={() => onShare(element)}
              className="rounded p-1.5 text-muted-foreground hover:bg-accent hover:text-foreground"
            >
              <Share2 className="h-3.5 w-3.5" aria-hidden />
            </button>
          )}
          {removable && (
            <button
              type="button"
              data-testid={`remove-${element.id}`}
              aria-label={`删除 ${name}`}
              title="删除（可选从哪些账号删）"
              onClick={() => onRemove(element)}
              disabled={busy}
              className="rounded p-1.5 text-muted-foreground hover:bg-destructive/10 hover:text-destructive"
            >
              <Trash2 className="h-3.5 w-3.5" aria-hidden />
            </button>
          )}
        </RowActions>
      </div>

      {spec.saved && (
        <div
          data-testid={`mcp-saved-${element.id}`}
          className={cn("mx-4 mb-2 rounded-md border px-3 py-2", spec.saved.valid ? "border-dashed bg-muted/40" : "border-warning/30 bg-warning/5")}
        >
          <div className="flex flex-wrap items-center justify-between gap-2">
            <div className="min-w-0 text-xs">
              <span className="font-medium text-foreground">存着但还没用上的配置（没写进任何账号）</span>
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
