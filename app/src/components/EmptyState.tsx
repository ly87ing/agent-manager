import { Inbox, type LucideIcon } from "lucide-react";
import type { ReactNode } from "react";
import { cn } from "../lib/utils";

/** A short inline guide; compact omits the border inside an existing panel. */
export function EmptyState({
  icon: Icon = Inbox,
  title,
  description,
  actions,
  compact = false,
  testId,
}: {
  icon?: LucideIcon;
  title: string;
  description?: ReactNode;
  actions?: ReactNode;
  compact?: boolean;
  testId?: string;
}) {
  return (
    <div
      data-testid={testId}
      className={cn(
        "flex flex-wrap items-center gap-3 text-left",
        compact ? "px-4 py-4" : "rounded-lg border bg-card px-4 py-5",
      )}
    >
      <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-md bg-muted">
        <Icon className="h-4 w-4 text-muted-foreground" aria-hidden />
      </div>
      <div className="min-w-0 flex-1 basis-48 break-words">
        <p className="text-sm font-medium text-foreground">{title}</p>
        {description ? <div className="mt-1 text-sm text-muted-foreground">{description}</div> : null}
      </div>
      {actions ? <div className="flex flex-wrap items-center gap-2">{actions}</div> : null}
    </div>
  );
}
