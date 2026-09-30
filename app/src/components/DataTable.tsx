import type { ReactNode } from "react";
import { cn } from "../lib/utils";

/*
 * 列表页共用的表格部件：名字列占满剩余宽度，其余列固定宽度；表头与每一行用同样的列宽，
 * 所有内容上下对齐。需要处理的按钮放在按钮列（常驻），次要操作是行尾图标（鼠标移到行上才显示）。
 */
export const COL_MIDDLE = "w-72 shrink-0";
export const COL_NARROW = "w-16 shrink-0 text-right tabular-nums";
export const COL_ACTION = "flex w-28 shrink-0 justify-start";
export const COL_ICONS = "flex w-28 shrink-0 items-center justify-end gap-0.5";
export const ROW = "group flex items-center gap-4 px-4 py-2 hover:bg-accent/40";
export const PILL =
  "whitespace-nowrap rounded-md border border-primary/30 bg-primary/10 px-2 py-0.5 text-xs text-primary hover:bg-primary/15 disabled:opacity-40";
/** 按钮列里只在鼠标移到行上才出现的按钮。 */
export const PILL_HOVER = cn(PILL, "opacity-0 transition-opacity focus-visible:opacity-100 group-hover:opacity-100");

/** 表格外框：表头与各行在同一个容器里。 */
export function DataTable({ label, children, testId }: { label: string; children: ReactNode; testId?: string }) {
  return (
    <section data-testid={testId} className="overflow-clip rounded-lg border bg-card" aria-label={label}>
      {children}
    </section>
  );
}

/** 表头：标题（「XX · 数量」）、中间列与窄列的列名，右边放「新增」等入口。不给 `narrow` 就没有窄列（行里也不放）。 */
export function TableHeader({ title, middle, narrow, children }: { title: string; middle: string; narrow?: string; children?: ReactNode }) {
  return (
    <div className="flex items-center gap-4 border-b bg-muted px-4 py-2 text-xs text-muted-foreground">
      <span className="min-w-0 flex-1 font-medium text-foreground">{title}</span>
      <span className={COL_MIDDLE}>{middle}</span>
      {narrow !== undefined && <span className={COL_NARROW}>{narrow}</span>}
      <span className="flex w-60 shrink-0 items-center justify-end gap-3">{children}</span>
    </div>
  );
}

/** 行尾图标按钮：鼠标移到行上（或键盘聚焦）才显示。 */
export function RowIcon({
  label,
  testId,
  danger,
  onClick,
  disabled,
  children,
}: {
  label: string;
  testId?: string;
  danger?: boolean;
  onClick: () => void;
  disabled?: boolean;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      data-testid={testId}
      disabled={disabled}
      onClick={onClick}
      className={cn(
        "rounded p-1.5 text-muted-foreground opacity-0 transition-opacity hover:bg-accent hover:text-foreground focus-visible:opacity-100 group-hover:opacity-100 disabled:opacity-40",
        danger && "hover:bg-destructive/10 hover:text-destructive",
      )}
    >
      {children}
    </button>
  );
}
