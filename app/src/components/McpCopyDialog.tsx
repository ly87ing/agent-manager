import { useState } from "react";
import type { McpTarget } from "../lib/contract";
import type { McpAgentCell } from "../lib/observed";
import { useDialogFocus } from "../lib/dialogFocus";

/**
 * 把一条 MCP 条目复制到另一个助手：引擎按目标客户端的格式转换字段。
 * 目标已有同名条目时默认不覆盖；勾选替换后整条换掉（目标条目的其他字段不保留），
 * 并以打开对话框时目标条目的摘要做写入基准。
 */
export function McpCopyDialog({
  name,
  from,
  targets,
  cells,
  busy,
  onSubmit,
  onClose,
}: {
  name: string;
  from: string;
  targets: McpTarget[];
  /** 该服务在各助手里的现有条目（判断目标是否同名冲突）。 */
  cells: Record<string, McpAgentCell>;
  busy: boolean;
  onSubmit: (input: { to: string; replaceBase: string | null }) => void;
  onClose: () => void;
}) {
  const choices = targets.filter((target) => target.agent !== from && target.error === null && target.agent_enabled);
  const [to, setTo] = useState(choices[0]?.agent ?? "");
  const [replace, setReplace] = useState(false);
  const { dialogRef, onDialogKeyDown } = useDialogFocus<HTMLDivElement>({ onEscape: onClose });
  const existing = to ? cells[to] : undefined;
  const canSubmit = Boolean(to) && (!existing || replace) && !busy;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-slate-900/40 p-4" onClick={(e) => e.target === e.currentTarget && onClose()}>
      <div
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-label={`复制 ${name}`}
        tabIndex={-1}
        onKeyDown={onDialogKeyDown}
        className="w-full max-w-md rounded-xl border bg-card p-5 shadow-xl"
      >
        <h3 className="text-base font-semibold text-foreground">
          复制 {name}（来自 {from}）
        </h3>
        <p className="mt-1 text-xs text-muted-foreground">
          按目标助手的配置格式写入；本表单不认识的字段不会带过去，完成后会列出来。
        </p>
        <label className="mt-4 block text-xs font-medium text-muted-foreground">复制到</label>
        <select
          value={to}
          onChange={(e) => {
            setTo(e.target.value);
            setReplace(false);
          }}
          data-testid="mcp-copy-target"
          className="mt-1 w-full rounded border border-input bg-card px-2 py-1.5 font-mono text-xs"
        >
          {choices.map((target) => (
            <option key={target.agent} value={target.agent}>
              {target.agent}
            </option>
          ))}
        </select>
        {choices.length === 0 && <p className="mt-2 text-xs text-muted-foreground">没有其他可写入的助手配置。</p>}
        {existing && (
          <label className="mt-3 flex items-start gap-2 rounded-md border border-warning/30 bg-warning/10 px-3 py-2 text-xs text-warning">
            <input
              type="checkbox"
              checked={replace}
              onChange={(e) => setReplace(e.target.checked)}
              data-testid="mcp-copy-replace"
              className="mt-0.5"
            />
            <span>{to} 里已有同名条目 {name}。勾选后整条替换，它原有的其他字段不会保留。</span>
          </label>
        )}
        <div className="mt-5 flex justify-end gap-2">
          <button type="button" onClick={onClose} className="rounded-md border px-4 py-2 text-sm text-muted-foreground hover:bg-accent">
            取消
          </button>
          <button
            type="button"
            data-testid="mcp-copy-submit"
            disabled={!canSubmit}
            onClick={() => onSubmit({ to, replaceBase: existing ? existing.base : null })}
            className="rounded-md bg-primary px-4 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90 disabled:cursor-not-allowed disabled:opacity-50"
          >
            {existing ? "替换并复制" : "复制"}
          </button>
        </div>
      </div>
    </div>
  );
}
