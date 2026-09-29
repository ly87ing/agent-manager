import { MoreHorizontal } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { cn } from "../lib/utils";

export type ChipTone = "on" | "off" | "warn" | "blocked";

const TONE: Record<ChipTone, string> = {
  on: "border-primary/40 bg-primary/15 text-primary hover:bg-primary/25",
  off: "border-dashed border-border text-muted-foreground hover:border-primary/40 hover:text-foreground",
  warn: "border-warning/50 bg-warning/10 text-warning hover:bg-warning/20",
  blocked: "cursor-not-allowed border-border text-muted-foreground/60",
};

const DOT: Record<ChipTone, string> = {
  on: "bg-primary",
  off: "border border-muted-foreground/60",
  warn: "bg-warning",
  blocked: "border border-muted-foreground/30",
};

/**
 * 一个账号上的开关：实心为已启用，空心为未启用，黄色为需要处理，灰色为不能操作。
 * 点一下执行它的主动作（启用 / 停用 / 修复 / 纳入管理…），说明在悬停提示里。
 */
export function AccountChip({
  label,
  tone,
  title,
  testId,
  disabled,
  onClick,
}: {
  label: string;
  tone: ChipTone;
  title: string;
  testId: string;
  disabled?: boolean;
  onClick?: () => void;
}) {
  const inert = disabled || !onClick || tone === "blocked";
  return (
    <button
      type="button"
      data-testid={testId}
      data-tone={tone}
      title={title}
      aria-label={title}
      disabled={inert}
      onClick={onClick}
      className={cn(
        "inline-flex max-w-40 items-center gap-1.5 rounded-full border px-2.5 py-0.5 text-xs transition-colors disabled:opacity-100",
        TONE[tone],
        inert && tone !== "blocked" && "cursor-default",
      )}
    >
      <span className={cn("h-1.5 w-1.5 shrink-0 rounded-full", DOT[tone])} aria-hidden />
      <span className="truncate">{label}</span>
    </button>
  );
}

export interface MenuAction {
  label: string;
  testId: string;
  danger?: boolean;
  onSelect: () => void;
}

/** 行尾「⋯」：放不常用、按账号区分的动作（编辑、复制到、删除、更新到新版本…）。 */
export function ActionMenu({ label, items }: { label: string; items: MenuAction[] }) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const close = (event: MouseEvent) => {
      if (!ref.current?.contains(event.target as Node)) setOpen(false);
    };
    window.addEventListener("mousedown", close);
    return () => window.removeEventListener("mousedown", close);
  }, [open]);
  if (items.length === 0) return <span className="w-7 shrink-0" aria-hidden />;
  return (
    <div ref={ref} className="relative shrink-0">
      <button
        type="button"
        aria-label={label}
        aria-expanded={open}
        onClick={() => setOpen((value) => !value)}
        className="rounded p-1.5 text-muted-foreground hover:bg-accent hover:text-foreground"
      >
        <MoreHorizontal className="h-4 w-4" aria-hidden />
      </button>
      {open && (
        <div role="menu" className="absolute right-0 z-20 mt-1 min-w-44 rounded-md border bg-popover p-1 shadow-lg">
          {items.map((item) => (
            <button
              key={item.testId}
              type="button"
              role="menuitem"
              data-testid={item.testId}
              onClick={() => {
                setOpen(false);
                item.onSelect();
              }}
              className={cn(
                "block w-full rounded px-2.5 py-1.5 text-left text-sm hover:bg-accent",
                item.danger && "text-destructive",
              )}
            >
              {item.label}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
