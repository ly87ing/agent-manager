import { Inbox, type LucideIcon } from "lucide-react";
import type { ReactNode } from "react";
import { cn } from "../lib/utils";

/**
 * 空态：圆形 muted 底图标 + 标题 + 描述 + 可选动作区。
 * 默认渲染为虚线面板（页面主内容区）；compact 用于已带边框的卡片/表格容器内，避免边框嵌套。
 */
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
        "flex flex-col items-center justify-center text-center",
        compact ? "px-4 py-8" : "rounded-lg border border-dashed px-6 py-10",
      )}
    >
      <div className="mb-3 flex h-12 w-12 items-center justify-center rounded-full bg-muted">
        <Icon className="h-6 w-6 text-muted-foreground" aria-hidden />
      </div>
      <p className="text-sm font-medium text-foreground">{title}</p>
      {description ? <div className="mt-1 max-w-md text-sm text-muted-foreground">{description}</div> : null}
      {actions ? <div className="mt-4 flex flex-wrap items-center justify-center gap-2">{actions}</div> : null}
    </div>
  );
}
