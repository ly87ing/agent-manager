import { MCP_CELL_COPY, SKILL_CELL_COPY } from "../lib/ui-copy";
import { cn } from "../lib/utils";

/**
 * 矩阵单元格徽章：MCP 服务 × 助手（条目状态）与 技能 × 助手（启用状态）
 * 共用渲染。用户文案集中在 lib/ui-copy.ts；title 保留工程原值对照。
 */

const MCP_CELL_STYLE: Record<string, string> = {
  enabled: "border-success/30 bg-success/10 text-success",
  disabled: "border bg-muted text-muted-foreground",
  saved: "border bg-muted text-muted-foreground",
  invalid: "border-rose/30 bg-rose/10 text-rose",
};

const SKILL_CELL_STYLE: Record<string, string> = {
  linked: "border-success/30 bg-success/10 text-success",
  "missing-link": "border bg-muted text-muted-foreground",
  "dead-link": "border-destructive/30 bg-destructive/10 text-destructive",
  drifted: "border-warning/30 bg-warning/10 text-warning",
  "extra-link": "border-info/30 bg-info/10 text-info",
  disabled: "border bg-muted text-muted-foreground",
};

const FALLBACK_STYLE = "border bg-muted text-muted-foreground";

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
