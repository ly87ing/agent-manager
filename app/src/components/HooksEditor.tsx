import { useState } from "react";
import { Plus, Trash2 } from "lucide-react";
import { HOOK_EVENTS, buildHooks, flattenHooks, hookRowError, hookSummary, hookType, type HookRow } from "../lib/hooksDefinition";

type HookMap = Record<string, unknown>;

/**
 * Hooks 结构化编辑：按「事件 + 匹配器 + 命令」逐行填，不用照着 JSON 结构手写。
 *
 * 编辑的是目标文件里的实际 hooks：保存把行回填成落盘结构，不认识的字段（timeout 等）
 * 原样带回；command 以外的类型（http、prompt、agent…）只读展示、整条原样写回。需要改
 * 那些字段或类型时切到原文编辑。保存由调用方写回实际文件（带写入基准比对）。
 */
export function HooksEditor({
  initialHooks,
  busy,
  onSave,
  onCancel,
}: {
  initialHooks: HookMap;
  busy: boolean;
  onSave: (hooks: HookMap) => void;
  onCancel: () => void;
}) {
  const [rows, setRows] = useState<HookRow[]>(() => flattenHooks(initialHooks));
  const [raw, setRaw] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const parseRaw = (text: string): HookMap | null => {
    try {
      const value: unknown = JSON.parse(text);
      if (typeof value === "object" && value !== null && !Array.isArray(value)) return value as HookMap;
    } catch {
      // 落到下面的统一提示
    }
    setError("原文必须是 JSON 对象（事件名 → 条目数组）");
    return null;
  };

  const toggleRaw = () => {
    setError(null);
    if (raw === null) {
      setRaw(JSON.stringify(buildHooks(rows), null, 2));
      return;
    }
    const parsed = parseRaw(raw);
    if (!parsed) return;
    setRows(flattenHooks(parsed));
    setRaw(null);
  };

  const save = () => {
    setError(null);
    if (raw !== null) {
      const parsed = parseRaw(raw);
      if (parsed) onSave(parsed);
      return;
    }
    for (const row of rows) {
      const rowError = hookRowError(row);
      if (rowError) {
        setError(rowError);
        return;
      }
    }
    onSave(buildHooks(rows));
  };

  const update = (index: number, patch: Partial<HookRow>) =>
    setRows((prev) => prev.map((item, i) => (i === index ? { ...item, ...patch } : item)));

  return (
    <div className="space-y-3">
      <datalist id="hook-event-options">
        {HOOK_EVENTS.map((event) => (
          <option key={event} value={event} />
        ))}
      </datalist>
      <div className="flex items-center justify-between gap-2">
        <p className="text-xs text-muted-foreground">按事件填写要执行的命令；匹配器留空表示该事件下都执行。</p>
        <div className="flex shrink-0 gap-2">
          {raw === null && (
            <button
              type="button"
              data-testid="hooks-add"
              disabled={busy}
              onClick={() =>
                setRows((prev) => [
                  ...prev,
                  { id: `new-${Date.now()}-${prev.length}`, event: "", matcher: "", command: "", extra: {}, entryExtra: {} },
                ])
              }
              className="inline-flex items-center gap-1 rounded border border-primary/30 bg-primary/10 px-2.5 py-1 text-xs text-primary hover:bg-primary/15"
            >
              <Plus className="h-3.5 w-3.5" aria-hidden /> 添加
            </button>
          )}
          <button
            type="button"
            data-testid="hooks-toggle-raw"
            disabled={busy}
            onClick={toggleRaw}
            className="rounded border px-2.5 py-1 text-xs text-muted-foreground hover:bg-accent"
          >
            {raw === null ? "编辑原文" : "按行编辑"}
          </button>
        </div>
      </div>

      {raw !== null ? (
        <textarea
          aria-label="Hooks 原文"
          value={raw}
          onChange={(event) => setRaw(event.target.value)}
          rows={16}
          disabled={busy}
          spellCheck={false}
          autoCorrect="off"
          autoCapitalize="off"
          className="w-full rounded border p-2 font-mono text-sm"
        />
      ) : rows.length === 0 ? (
        <p data-testid="hooks-empty" className="text-sm text-muted-foreground">
          还没有配置任何命令。保存为空会移除这个文件里的 hooks。
        </p>
      ) : (
        <ul className="space-y-2">
          {rows.map((row, index) =>
            row.raw ? (
              <li key={row.id} data-testid={`hooks-row-${index}`} className="rounded-md border border-dashed px-3 py-2">
                <div className="flex flex-wrap items-center gap-1.5 text-xs">
                  <span className="font-mono">{row.event}</span>
                  <span className="text-muted-foreground">{row.matcher ? `匹配 ${row.matcher}` : "全部匹配"}</span>
                  <span data-testid={`hooks-type-${index}`} className="rounded-full border px-2 py-0.5 text-[11px]">
                    {hookType(row) || "未标类型"}
                  </span>
                </div>
                <p className="mt-1 font-mono text-xs break-all text-muted-foreground">{hookSummary(row)}</p>
                <p className="mt-1 text-[11px] text-muted-foreground">这类 hook 在这里只读，保存时原样保留；要改请用原文编辑。</p>
              </li>
            ) : (
              <li key={row.id} data-testid={`hooks-row-${index}`} className="rounded-md border px-3 py-2">
                <div className="flex flex-wrap items-center gap-1.5">
                  <input
                    value={row.event}
                    list="hook-event-options"
                    disabled={busy}
                    onChange={(event) => update(index, { event: event.target.value })}
                    data-testid={`hooks-event-${index}`}
                    placeholder="事件，如 PreToolUse"
                    className="w-44 rounded border border-input px-2 py-1.5 font-mono text-xs"
                  />
                  <input
                    value={row.matcher}
                    disabled={busy}
                    onChange={(event) => update(index, { matcher: event.target.value })}
                    data-testid={`hooks-matcher-${index}`}
                    placeholder="匹配器（可留空）"
                    className="w-40 rounded border border-input px-2 py-1.5 font-mono text-xs"
                  />
                  <button
                    type="button"
                    data-testid={`hooks-remove-${index}`}
                    disabled={busy}
                    onClick={() => setRows((prev) => prev.filter((_, i) => i !== index))}
                    aria-label="移除这条命令"
                    className="ml-auto shrink-0 rounded border px-2 py-1.5 text-xs text-destructive hover:bg-destructive/10"
                  >
                    <Trash2 className="h-3.5 w-3.5" aria-hidden />
                  </button>
                </div>
                <input
                  value={row.command}
                  disabled={busy}
                  onChange={(event) => update(index, { command: event.target.value })}
                  data-testid={`hooks-command-${index}`}
                  placeholder="要执行的命令"
                  className="mt-1.5 w-full rounded border border-input px-2 py-1.5 font-mono text-xs"
                />
                {Object.keys(row.extra).length > 0 && (
                  <p className="mt-1 text-[11px] text-muted-foreground">保留原有字段：{Object.keys(row.extra).join("、")}</p>
                )}
              </li>
            ),
          )}
        </ul>
      )}

      {error && (
        <p role="alert" className="text-sm text-destructive">
          {error}
        </p>
      )}
      <div className="flex justify-end gap-2">
        <button type="button" disabled={busy} onClick={onCancel} className="rounded border px-4 py-2 text-sm">
          取消
        </button>
        <button
          type="button"
          data-testid="hooks-save"
          disabled={busy}
          onClick={save}
          className="rounded bg-primary px-4 py-2 text-sm text-primary-foreground disabled:opacity-60"
        >
          保存到文件
        </button>
      </div>
    </div>
  );
}
