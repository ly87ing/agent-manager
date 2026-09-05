import { cn } from "../lib/utils";

/**
 * 矩阵单元格徽章：MCP 注入格（server × agent）与 Skills 启用格（skill × agent）
 * 共用渲染，词汇表不同（引擎 _mcp_cell_state / _skill_cell_state）。
 */

const MCP_CELL_META: Record<string, { label: string; className: string }> = {
  "managed-ok": { label: "已注入", className: "border-emerald-200 bg-emerald-50 text-emerald-700" },
  drifted: { label: "漂移", className: "border-amber-200 bg-amber-50 text-amber-700" },
  missing: { label: "未注入", className: "border-slate-200 bg-slate-50 text-slate-500" },
  absent: { label: "未注入", className: "border-slate-200 bg-slate-50 text-slate-500" },
  "agent-disabled": { label: "未注入", className: "border-slate-200 bg-slate-50 text-slate-400" },
  "not-materialized": { label: "未物化", className: "border-zinc-200 bg-zinc-100 text-zinc-500" },
  leftover: { label: "遗留", className: "border-orange-200 bg-orange-50 text-orange-700" },
  "probe-error": { label: "探测异常", className: "border-zinc-300 bg-zinc-100 text-zinc-500" },
};

const SKILL_CELL_META: Record<string, { label: string; className: string }> = {
  linked: { label: "已链接", className: "border-emerald-200 bg-emerald-50 text-emerald-700" },
  "missing-link": { label: "未链接", className: "border-slate-200 bg-slate-50 text-slate-500" },
  "dead-link": { label: "死链", className: "border-red-200 bg-red-50 text-red-700" },
  drifted: { label: "漂移", className: "border-amber-200 bg-amber-50 text-amber-700" },
  "extra-link": { label: "额外链接", className: "border-sky-200 bg-sky-50 text-sky-700" },
  disabled: { label: "已停用", className: "border-zinc-200 bg-zinc-50 text-zinc-400" },
};

const FALLBACK = { className: "border-slate-200 bg-slate-50 text-slate-500" };

export function MatrixCell({
  state,
  kind,
  title,
}: {
  state: string;
  kind: "mcp" | "skills";
  title?: string;
}) {
  const meta = (kind === "mcp" ? MCP_CELL_META : SKILL_CELL_META)[state] ?? { label: state, ...FALLBACK };
  return (
    <span
      data-cell-state={state}
      title={title ?? state}
      className={cn(
        "inline-flex items-center justify-center rounded border px-1.5 py-0.5 text-xs whitespace-nowrap",
        meta.className,
      )}
    >
      {meta.label}
    </span>
  );
}
