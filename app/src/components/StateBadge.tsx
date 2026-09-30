import type { ReactNode } from "react";
import { INBOX_STATES, type InboxState } from "../lib/inbox";
import { cn } from "../lib/utils";

/** 统一状态的配色：待处理页、各资源页的标签同一套。 */
export const STATE_TONE: Record<InboxState, string> = {
  broken: "bg-destructive/10 text-destructive",
  drifted: "bg-warning/15 text-warning",
  missing: "bg-primary/10 text-primary",
  update: "bg-info/10 text-info",
  unmanaged: "bg-muted text-muted-foreground",
};

/** 统一状态标签；不给文字时用状态名（损坏、已偏离…）。 */
export function StateBadge({ state, children, title, testId }: { state: InboxState; children?: ReactNode; title?: string; testId?: string }) {
  const label = INBOX_STATES.find((item) => item.state === state)?.label ?? state;
  return (
    <span data-testid={testId} data-state={state} title={title} className={cn("whitespace-nowrap rounded px-1.5 py-0.5 text-[11px]", STATE_TONE[state])}>
      {children ?? label}
    </span>
  );
}

/** 资源页工具栏上的「只看待处理」：和待处理页同一份判断。 */
export function AttentionOnlyToggle({ checked, count, onChange }: { checked: boolean; count: number; onChange: (next: boolean) => void }) {
  if (count === 0 && !checked) return null;
  return (
    <button
      type="button"
      data-testid="attention-only"
      aria-pressed={checked}
      onClick={() => onChange(!checked)}
      className={cn(
        "rounded-md border px-3 py-1.5 text-sm",
        checked ? "border-warning/40 bg-warning/15 text-warning" : "text-muted-foreground hover:bg-accent hover:text-foreground",
      )}
    >
      只看待处理 {count}
    </button>
  );
}
