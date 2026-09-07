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
        "managed-ok": "border-emerald-200 bg-emerald-50 text-emerald-700",
        drifted: "border-amber-200 bg-amber-50 text-amber-700",
        missing: "border-red-200 bg-red-50 text-red-700",
        "not-materialized": "border-slate-200 bg-slate-50 text-slate-600",
        "unmanaged-detected": "border-sky-200 bg-sky-50 text-sky-700",
        unsupported: "border-zinc-200 bg-zinc-100 text-zinc-500",
        disabled: "border-zinc-200 bg-zinc-50 text-zinc-400",
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
