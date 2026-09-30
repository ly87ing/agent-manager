import { MoreHorizontal } from "lucide-react";
import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import type { AccountColumn, AccountGroup } from "../lib/accounts";
import { cn } from "../lib/utils";
import { AgentIcon } from "./AgentIcon";

/**
 * 技能、MCP 共用的账号表格：表头按助手分两层（助手 → 它的账号），账号名只在表头出现一次；
 * 每行在对齐的列里放一个开关。行尾固定宽度放编辑 / 删除等动作，鼠标移到行上才显示。
 */

export type CellTone = "on" | "off" | "warn" | "blocked";

/** 一个账号列的宽度；表头、每行都用它，列才对得齐。 */
const COLUMN = "w-16 shrink-0";
/** 行尾动作区宽度：放得下五个图标按钮（集合的更新、分享、编辑、启停、移除）。 */
export const TRAILING = "w-36 shrink-0";

/** 每个助手都只有一个账号时，表头只要一层（图标 + 助手名）。 */
function singleLevel(groups: AccountGroup[]): boolean {
  return groups.every((group) => group.columns.length === 1);
}

function GroupColumns({
  groups,
  cell,
  header = false,
}: {
  groups: AccountGroup[];
  cell: (column: AccountColumn, group: AccountGroup) => ReactNode;
  header?: boolean;
}) {
  const twoLevels = header && !singleLevel(groups);
  return (
    <div className="flex shrink-0 items-stretch">
      {groups.map((group, index) => (
        <div
          key={group.key}
          className={cn("flex flex-col", index > 0 && "ml-2 border-l pl-2", !twoLevels && "border-transparent")}
        >
          {twoLevels && (
            <div
              className="mb-1 flex items-center justify-center gap-1 border-b pb-1 text-xs font-semibold text-foreground"
              title={group.label}
            >
              <AgentIcon kind={group.kind} />
              <span className="truncate">{group.label}</span>
            </div>
          )}
          <div className="flex flex-1 items-center">
            {group.columns.map((column) => (
              <div key={column.id} className={cn(COLUMN, "flex flex-col items-center justify-center")}>
                {cell(column, group)}
              </div>
            ))}
          </div>
        </div>
      ))}
    </div>
  );
}

/**
 * 表头：左边是这一栏的标题，右边是账号列。每个助手只有一个账号时只有一层（图标 + 助手名）；
 * 有多个账号时上层是助手、下层是账号名。每列下面可放「补齐」等需要处理的入口。
 */
export function AccountHeader({
  groups,
  left,
  below,
  className,
}: {
  groups: AccountGroup[];
  left?: ReactNode;
  /** 账号名下面的一行（只在需要处理时给，如「补齐 3」）。 */
  below?: (column: AccountColumn) => ReactNode;
  className?: string;
}) {
  const single = singleLevel(groups);
  return (
    <div className={cn("flex items-center gap-3 border-b bg-muted px-4 py-2 text-xs text-muted-foreground", className)}>
      <div className="min-w-0 flex-1">{left}</div>
      <GroupColumns
        header
        groups={groups}
        cell={(column, group) => (
          <>
            {single ? (
              <span className="flex max-w-full items-center gap-1 font-semibold text-foreground" title={column.fullName}>
                <AgentIcon kind={group.kind} />
                <span className="truncate">{group.label}</span>
              </span>
            ) : (
              <span className="max-w-full truncate text-foreground" title={column.fullName}>{column.name}</span>
            )}
            {below?.(column)}
          </>
        )}
      />
      <div className={TRAILING} />
    </div>
  );
}

/** 一行里与表头对齐的账号格子。 */
export function AccountCells({ groups, cell }: { groups: AccountGroup[]; cell: (column: AccountColumn) => ReactNode }) {
  return <GroupColumns groups={groups} cell={(column) => cell(column)} />;
}

/**
 * 一个账号上的开关：蓝色为开，灰色为关，琥珀色为需要处理，半透明为不能操作。
 * 点一下执行它的主动作（启用 / 停用 / 修复 / 纳入管理…），说明在悬停提示里。
 */
export function AccountSwitch({
  tone,
  checked = tone === "on",
  title,
  testId,
  disabled,
  onClick,
}: {
  tone: CellTone;
  /** 开关位置；需要处理（warn）时由调用方说明它现在算开还是关。 */
  checked?: boolean;
  title: string;
  testId: string;
  disabled?: boolean;
  onClick?: () => void;
}) {
  const inert = disabled || !onClick || tone === "blocked";
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      data-testid={testId}
      data-tone={tone}
      title={title}
      aria-label={title}
      disabled={inert}
      onClick={onClick}
      className={cn(
        "relative h-4 w-7 shrink-0 rounded-full transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
        tone === "warn" ? "bg-warning" : checked ? "bg-primary" : "bg-input",
        tone === "blocked" && "opacity-40",
        inert ? "cursor-default" : "cursor-pointer hover:brightness-110",
      )}
    >
      <span
        aria-hidden
        className={cn(
          "absolute top-0.5 h-3 w-3 rounded-full bg-white shadow transition-all",
          checked ? "left-[14px]" : "left-0.5",
        )}
      />
    </button>
  );
}

/**
 * 行尾动作区（固定宽度，各行对齐）。hover 里的鼠标移到行上（或键盘聚焦）才显示；
 * always 里放需要处理时才出现、要一直看得见的（如「有更新」）。行需带 `group` 类。
 */
export function RowActions({ children, always }: { children?: ReactNode; always?: ReactNode }) {
  return (
    <div className={cn(TRAILING, "flex items-center justify-end gap-0.5")}>
      {always}
      <div className="flex items-center gap-0.5 opacity-0 transition-opacity focus-within:opacity-100 group-hover:opacity-100">{children}</div>
    </div>
  );
}

/** 行尾的图标按钮。 */
export function IconAction({
  label,
  testId,
  danger,
  tone,
  onClick,
  disabled,
  children,
}: {
  label: string;
  testId?: string;
  danger?: boolean;
  /** primary：需要处理时常驻显示的强调色。 */
  tone?: "primary";
  onClick: () => void;
  disabled?: boolean;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      data-testid={testId}
      aria-label={label}
      title={label}
      onClick={onClick}
      disabled={disabled}
      className={cn(
        "rounded p-1.5 disabled:opacity-40",
        tone === "primary" ? "text-primary hover:bg-primary/10" : "text-muted-foreground hover:bg-accent hover:text-foreground",
        danger && "hover:bg-destructive/10 hover:text-destructive",
      )}
    >
      {children}
    </button>
  );
}

export interface MenuAction {
  label: string;
  testId: string;
  danger?: boolean;
  onSelect: () => void;
}

/** 行尾「⋯」：放不常用、按账号区分的动作（更新到新版本、取消外部链接…）。 */
export function ActionMenu({ label, items }: { label: string; items: MenuAction[] }) {
  const [open, setOpen] = useState(false);
  const buttonRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  /** 菜单的固定定位：渲染到页面最外层，不被表格等容器的 overflow 裁掉。 */
  const [position, setPosition] = useState<{ top: number; right: number } | null>(null);

  useLayoutEffect(() => {
    if (!open || !buttonRef.current) return;
    const button = buttonRef.current.getBoundingClientRect();
    const height = menuRef.current?.offsetHeight ?? 0;
    const width = menuRef.current?.offsetWidth ?? 0;
    const gap = 4;
    const margin = 8;
    // 下方放不下就向上展开；左右不超出窗口
    const below = button.bottom + gap;
    const top = height && below + height > window.innerHeight - margin ? Math.max(margin, button.top - gap - height) : below;
    const right = Math.min(Math.max(margin, window.innerWidth - button.right), Math.max(margin, window.innerWidth - width - margin));
    setPosition((prev) => (prev && prev.top === top && prev.right === right ? prev : { top, right }));
  }, [open, position]);

  useEffect(() => {
    if (!open) return;
    const inside = (target: EventTarget | null) =>
      Boolean(target instanceof Node && (buttonRef.current?.contains(target) || menuRef.current?.contains(target)));
    const onMouseDown = (event: MouseEvent) => {
      if (!inside(event.target)) setOpen(false);
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        setOpen(false);
        buttonRef.current?.focus();
      }
    };
    // 固定定位不跟着内容滚：页面滚动或窗口变化时直接关掉，免得菜单与按钮错位
    const onScroll = (event: Event) => {
      if (!inside(event.target)) setOpen(false);
    };
    const onResize = () => setOpen(false);
    window.addEventListener("mousedown", onMouseDown);
    window.addEventListener("keydown", onKey);
    window.addEventListener("scroll", onScroll, true);
    window.addEventListener("resize", onResize);
    return () => {
      window.removeEventListener("mousedown", onMouseDown);
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("scroll", onScroll, true);
      window.removeEventListener("resize", onResize);
    };
  }, [open]);

  if (items.length === 0) return null;
  return (
    <div className="relative shrink-0">
      <button
        ref={buttonRef}
        type="button"
        aria-label={label}
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => {
          setPosition(null);
          setOpen((value) => !value);
        }}
        className="rounded p-1.5 text-muted-foreground hover:bg-accent hover:text-foreground"
      >
        <MoreHorizontal className="h-4 w-4" aria-hidden />
      </button>
      {open &&
        createPortal(
          <div
            ref={menuRef}
            role="menu"
            aria-label={label}
            style={{ position: "fixed", top: position?.top ?? 0, right: position?.right ?? 0, visibility: position ? "visible" : "hidden" }}
            className="z-[60] min-w-44 rounded-md border bg-popover p-1 shadow-lg"
          >
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
                className={cn("block w-full rounded px-2.5 py-1.5 text-left text-sm hover:bg-accent", item.danger && "text-destructive")}
              >
                {item.label}
              </button>
            ))}
          </div>,
          document.body,
        )}
    </div>
  );
}
