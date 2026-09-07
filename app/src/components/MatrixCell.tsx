import { MCP_CELL_COPY, SKILL_CELL_COPY } from "../lib/ui-copy";
import { cn } from "../lib/utils";

/**
 * 矩阵单元格徽章：MCP 服务 × 助手（配置状态）与 技能 × 助手（启用状态）
 * 共用渲染。用户文案集中在 lib/ui-copy.ts；title 保留工程原值对照。
 */

const MCP_CELL_STYLE: Record<string, string> = {
  "managed-ok": "border-emerald-200 bg-emerald-50 text-emerald-700",
  drifted: "border-amber-200 bg-amber-50 text-amber-700",
  missing: "border-slate-200 bg-slate-50 text-slate-500",
  absent: "border-slate-200 bg-slate-50 text-slate-500",
  "agent-disabled": "border-slate-200 bg-slate-50 text-slate-400",
  "not-materialized": "border-zinc-200 bg-zinc-100 text-zinc-500",
  leftover: "border-orange-200 bg-orange-50 text-orange-700",
  "probe-error": "border-zinc-300 bg-zinc-100 text-zinc-500",
};

const SKILL_CELL_STYLE: Record<string, string> = {
  linked: "border-emerald-200 bg-emerald-50 text-emerald-700",
  "missing-link": "border-slate-200 bg-slate-50 text-slate-500",
  "dead-link": "border-red-200 bg-red-50 text-red-700",
  drifted: "border-amber-200 bg-amber-50 text-amber-700",
  "extra-link": "border-sky-200 bg-sky-50 text-sky-700",
  disabled: "border-zinc-200 bg-zinc-50 text-zinc-400",
};

const FALLBACK_STYLE = "border-slate-200 bg-slate-50 text-slate-500";

export function MatrixCell({
  state,
  kind,
  title,
}: {
  state: string;
  kind: "mcp" | "skills";
  title?: string;
}) {
  const label = (kind === "mcp" ? MCP_CELL_COPY : SKILL_CELL_COPY)[state] ?? state;
  const style = (kind === "mcp" ? MCP_CELL_STYLE : SKILL_CELL_STYLE)[state] ?? FALLBACK_STYLE;
  return (
    <span
      data-cell-state={state}
      title={title ?? state}
      className={cn(
        "inline-flex items-center justify-center rounded border px-1.5 py-0.5 text-xs whitespace-nowrap",
        style,
      )}
    >
      {label}
    </span>
  );
}
