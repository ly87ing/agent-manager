import type { RiskLevel } from "../lib/contract";
import { RISK_COPY } from "../lib/ui-copy";
import { cn } from "../lib/utils";

/** 风险级徽章（ConfirmModal 等破坏性/独立确认弹窗使用）。 */
export function RiskBadge({ risk, label }: { risk: RiskLevel; label?: string }) {
  const copy = RISK_COPY[risk];
  return (
    <span
      title={risk}
      className={cn("inline-flex rounded-full border px-2 py-0.5 text-xs font-medium whitespace-nowrap", copy.className)}
    >
      {label ?? copy.label}
    </span>
  );
}
