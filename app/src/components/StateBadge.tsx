import { cva } from "class-variance-authority";
import type { ElementState } from "../lib/contract";
import { ELEMENT_STATE_COPY } from "../lib/ui-copy";
import { cn } from "../lib/utils";

/** 同步状态徽章（收敛轴七态，提案 §3.1；用户文案见 lib/ui-copy.ts，tooltip 保留工程原值对照）。 */
const stateBadgeVariants = cva(
  "inline-flex items-center rounded-full border px-2 py-0.5 text-xs font-medium whitespace-nowrap",
  {
    variants: {
      state: {
        "managed-ok": "border-success/30 bg-success/10 text-success",
        drifted: "border-warning/30 bg-warning/10 text-warning",
        missing: "border-destructive/30 bg-destructive/10 text-destructive",
        "not-materialized": "border bg-muted text-muted-foreground",
        "unmanaged-detected": "border-info/30 bg-info/10 text-info",
        unsupported: "border bg-muted text-muted-foreground",
        disabled: "border bg-muted text-muted-foreground",
      },
    },
  },
);

export function StateBadge({ state, className }: { state: ElementState; className?: string }) {
  const copy = ELEMENT_STATE_COPY[state];
  return (
    <span className={cn(stateBadgeVariants({ state }), className)} title={`${state}（${copy.hint}）`}>
      {copy.label}
    </span>
  );
}
