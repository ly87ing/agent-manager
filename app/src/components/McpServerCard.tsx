import type { EngineElement } from "../lib/contract";
import type { McpAgentCell, McpObserved, McpSpec } from "../lib/observed";
import { AccountChip, ActionMenu, type ChipTone } from "./AccountChip";
import { mcpEntrySummary } from "./McpAssignmentRow";
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

function mcpChip(cell: McpAgentCell | undefined, canAddHere: boolean): { tone: ChipTone; hint: string; primary: string | null } {
  if (!cell) {
    return canAddHere
      ? { tone: "off", hint: "这个账号还没有，点击从其他账号复制过来", primary: "add-here" }
      : { tone: "blocked", hint: "这个账号还没有这个服务", primary: null };
  }
  const has = (action: string) => cell.actions.includes(action);
  if (cell.state === "invalid") {
    return { tone: "warn", hint: cell.blocked_reason ?? "条目结构无法识别，只能删除或在配置文件里修正", primary: null };
  }
  if (cell.state === "enabled") {
    const missing = cell.command_available === false;
    return {
      tone: missing ? "warn" : "on",
      hint: missing ? "已启用，但本机找不到它要执行的命令；点击停用" : "已启用，点击停用",
      primary: has("disable") ? "disable" : null,
    };
  }
  return {
    tone: "off",
    hint: cell.source === "saved" ? "已停用（内容保存在本工具里），点击启用" : "已停用，点击启用",
    primary: has("enable") ? "enable" : null,
  };
}

const MENU_LABEL: Record<string, string> = { edit: "编辑", copy: "复制到…", remove: "删除" };

/**
 * 一个服务一行：每个账号一个开关，点一下启用 / 停用，没有这个服务的账号点一下从其他账号复制过来；
 * 编辑、复制到、删除按账号放在「⋯」里。另附「已保存、还没写进任何账号」的定义。取值一律由引擎脱敏。
 */
export function McpServerCard({
  element,
  agents,
  busy,
  exclusive = false,
  nameOf = (agent) => agent,
  onAction,
  onAddHere,
  onAddFromSaved,
  onForget,
  onConfigureSecrets,
}: {
  element: EngineElement;
  /** 可写的账号（status.targets 顺序）。 */
  agents: string[];
  busy: boolean;
  /** 按账号筛选时只显示 agents 里的账号。 */
  exclusive?: boolean;
  nameOf?: (agent: string) => string;
  onAction: (element: EngineElement, agent: string, action: string, cell: McpAgentCell) => void;
  /** 这个账号还没有：从 from 账号的条目复制过来。 */
  onAddHere: (element: EngineElement, from: string, to: string) => void;
  onAddFromSaved: (element: EngineElement) => void;
  onForget: (element: EngineElement) => void;
  onConfigureSecrets?: (variable: string) => void;
}) {
  const spec = element.spec as unknown as McpSpec;
  const name = element.id.replace(/^mcp:/, "");
  const cells = (element.observed as McpObserved).agents ?? {};
  const shown = exclusive ? agents : [...agents, ...Object.keys(cells).filter((agent) => !agents.includes(agent))];
  const missing = missingVariables(element);
  const source = Object.entries(cells).find(([, cell]) => cell.source === "file" && cell.actions.includes("copy"));
  const summaries = new Set(Object.values(cells).map(mcpEntrySummary));
  const first = source?.[1] ?? Object.values(cells)[0];
  const menu = shown.flatMap((agent) => {
    const cell = cells[agent];
    if (!cell) return [];
    return ["edit", "copy", "remove"]
      .filter((action) => cell.actions.includes(action))
      .map((action) => ({
        label: `${MENU_LABEL[action]} · ${nameOf(agent)}`,
        testId: `action-${action}-${element.id}-${agent}`,
        danger: action === "remove",
        onSelect: () => onAction(element, agent, action, cell),
      }));
  });

  return (
    <li data-testid={`mcp-card-${element.id}`}>
      <div className="flex flex-wrap items-center gap-2 px-4 py-2">
        <span className="font-mono text-sm">{name}</span>
        {first && (
          <code className="min-w-0 max-w-72 truncate font-mono text-xs text-muted-foreground" title={mcpEntrySummary(first)}>
            {mcpEntrySummary(first)}
          </code>
        )}
        {summaries.size > 1 && <span className="rounded bg-info/10 px-1.5 py-0.5 text-[11px] text-info">各账号内容不同</span>}
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
        <div className="ml-auto flex flex-wrap items-center justify-end gap-1.5">
          {shown.map((agent) => {
            const cell = cells[agent];
            const writable = agents.includes(agent);
            const chip = writable ? mcpChip(cell, Boolean(source)) : { tone: "blocked" as const, hint: cell?.blocked_reason ?? "这个账号的配置文件现在读写不了", primary: null };
            const detail = cell ? `\n${mcpEntrySummary(cell)}` : "";
            return (
              <AccountChip
                key={agent}
                label={nameOf(agent)}
                tone={chip.tone}
                title={`${nameOf(agent)}：${chip.hint}${detail}`}
                testId={chip.primary ? `action-${chip.primary}-${element.id}-${agent}` : `chip-${element.id}-${agent}`}
                disabled={busy}
                onClick={
                  chip.primary === "add-here" && source
                    ? () => onAddHere(element, source[0], agent)
                    : chip.primary && cell
                      ? () => onAction(element, agent, chip.primary as string, cell)
                      : undefined
                }
              />
            );
          })}
          <ActionMenu label={`${name} 的更多操作`} items={menu} />
        </div>
      </div>

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
