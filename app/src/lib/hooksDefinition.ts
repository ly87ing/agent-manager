/**
 * Hooks 结构 ↔ 扁平行互转（纯函数）。
 *
 * 落盘结构是三层嵌套：`hooks[事件] = [{ matcher?, hooks: [{type, ...}] }]`。
 * 界面按「事件 + 匹配器 + 命令」一行行编辑 command 类型的 hook，这里负责两边转换，并把
 * 不认识的字段（timeout 等）原样带回去——本工具不理解它们，但也不能在编辑时悄悄丢掉。
 * 其他类型（http、mcp_tool、prompt、agent 以及将来的类型）只读：整条对象原样带回。
 */

export interface HookRow {
  /** 列表稳定 key，仅界面用，不落盘。 */
  id: string;
  event: string;
  /** 空串表示不限匹配器。 */
  matcher: string;
  command: string;
  /** 该 hook 对象里除 type/command 外的字段。 */
  extra: Record<string, unknown>;
  /** 该 entry 对象里除 matcher/hooks 外的字段。 */
  entryExtra: Record<string, unknown>;
  /** 非 command 类型的 hook：整条原对象，只读、写回时原样保留。 */
  raw?: Record<string, unknown>;
}

/** Claude Code 的事件名（作为候选，不限制只能填这些）。 */
export const HOOK_EVENTS = [
  "PreToolUse",
  "PostToolUse",
  "UserPromptSubmit",
  "Notification",
  "Stop",
  "SubagentStop",
  "SessionStart",
  "SessionEnd",
  "PreCompact",
];

type HookMap = Record<string, unknown>;

/** 非 command 类型 hook 的类型名（没有 type 字段时为空串）。 */
export function hookType(row: HookRow): string {
  if (!row.raw) return "command";
  return typeof row.raw.type === "string" ? row.raw.type : "";
}

/** 非 command 类型 hook 的一句话摘要：除 type 外的字段，截断到可读长度。 */
export function hookSummary(row: HookRow): string {
  if (!row.raw) return row.command;
  const { type: _type, ...rest } = row.raw;
  const text = JSON.stringify(rest);
  return text.length > 160 ? `${text.slice(0, 157)}…` : text;
}

export function flattenHooks(hooks: HookMap | undefined | null): HookRow[] {
  const rows: HookRow[] = [];
  if (!hooks || typeof hooks !== "object") return rows;
  for (const [event, entries] of Object.entries(hooks)) {
    if (!Array.isArray(entries)) continue;
    entries.forEach((entry, entryIndex) => {
      if (typeof entry !== "object" || entry === null) return;
      const { matcher, hooks: hookList, ...entryExtra } = entry as Record<string, unknown>;
      if (!Array.isArray(hookList)) return;
      hookList.forEach((hook, hookIndex) => {
        if (typeof hook !== "object" || hook === null) return;
        const base = {
          id: `${event}-${entryIndex}-${hookIndex}`,
          event,
          matcher: typeof matcher === "string" ? matcher : "",
          entryExtra,
        };
        const { type, command, ...extra } = hook as Record<string, unknown>;
        if (type !== "command") {
          rows.push({ ...base, command: "", extra: {}, raw: hook as Record<string, unknown> });
          return;
        }
        rows.push({ ...base, command: typeof command === "string" ? command : "", extra });
      });
    });
  }
  return rows;
}

/**
 * 扁平行回填为落盘结构：同事件同匹配器的 hook 合并进一个 entry（与落盘结构一致），
 * 事件与 entry 的出现顺序按行顺序，保证保存后再读出来还是同一份。
 */
export function buildHooks(rows: HookRow[]): HookMap {
  const out: Record<string, Record<string, unknown>[]> = {};
  for (const row of rows) {
    const event = row.event.trim();
    const command = row.command.trim();
    if (!event || (!row.raw && !command)) continue;
    const matcher = row.matcher.trim();
    const list = (out[event] ??= []);
    const existing = list.find((entry) => (typeof entry.matcher === "string" ? entry.matcher : "") === matcher);
    const hook = row.raw ?? { type: "command", command, ...row.extra };
    if (existing) {
      (existing.hooks as Record<string, unknown>[]).push(hook);
    } else {
      list.push({
        ...row.entryExtra,
        ...(matcher ? { matcher } : {}),
        hooks: [hook],
      });
    }
  }
  return out;
}

/** 保存前校验：command 类型的事件名与命令必填，其余交给引擎的结构校验兜底。 */
export function hookRowError(row: HookRow): string | null {
  if (row.raw) return null;
  if (!row.event.trim()) return "事件名不能为空";
  if (!row.command.trim()) return "命令不能为空";
  return null;
}
