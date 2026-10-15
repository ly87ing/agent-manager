import { Check, ChevronDown } from "lucide-react";
import { useEffect, useId, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { cn } from "../lib/utils";

export interface SelectOption {
  value: string;
  label: string;
}

/**
 * 应用自己的下拉选择，代替原生 `<select>`：深浅色一致、弹出层渲染到页面最外层不被裁掉。
 *
 * 键盘：↑↓ 移动，Home/End 到头尾，Enter/空格选中，Esc 关闭；打字跳到以它开头的一项。
 * 当前值写在按钮的 `data-value` 上，测试与自动化据此读取。
 */
export function Select({
  value,
  options,
  onChange,
  label,
  testId,
  id,
  disabled,
  className,
  mono,
}: {
  value: string;
  options: SelectOption[];
  onChange: (value: string) => void;
  /** 无障碍名称（aria-label）。 */
  label: string;
  testId?: string;
  id?: string;
  disabled?: boolean;
  /** 按钮的尺寸与边框，沿用原先 select 的写法。 */
  className?: string;
  mono?: boolean;
}) {
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(0);
  const [position, setPosition] = useState<{ top: number; left: number; width: number; maxHeight: number } | null>(null);
  const buttonRef = useRef<HTMLButtonElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const anchorRef = useRef<{ top: number; left: number } | null>(null);
  const typed = useRef({ text: "", at: 0 });
  const listId = useId();
  const selectedIndex = Math.max(0, options.findIndex((option) => option.value === value));
  const current = options.find((option) => option.value === value);

  const close = (focusButton = true) => {
    setOpen(false);
    setPosition(null);
    if (focusButton) buttonRef.current?.focus();
  };
  const choose = (index: number) => {
    const option = options[index];
    if (!option) return;
    if (option.value !== value) onChange(option.value);
    close();
  };
  const openList = () => {
    if (disabled || options.length === 0) return;
    setActive(selectedIndex);
    setOpen(true);
  };

  useLayoutEffect(() => {
    if (!open || !buttonRef.current) return;
    const rect = buttonRef.current.getBoundingClientRect();
    anchorRef.current = { top: rect.top, left: rect.left };
    const margin = 8;
    const gap = 4;
    const below = window.innerHeight - rect.bottom - gap - margin;
    const above = rect.top - gap - margin;
    const wanted = Math.min(320, (listRef.current?.scrollHeight ?? 320) + 2);
    // 下方放不下且上方更宽裕时向上展开
    const up = below < wanted && above > below;
    const maxHeight = Math.max(120, Math.min(320, up ? above : below));
    const height = Math.min(wanted, maxHeight);
    const width = Math.max(rect.width, 160);
    const left = Math.min(Math.max(margin, rect.left), Math.max(margin, window.innerWidth - width - margin));
    const top = up ? rect.top - gap - height : rect.bottom + gap;
    setPosition((prev) =>
      prev && prev.top === top && prev.left === left && prev.width === width && prev.maxHeight === maxHeight ? prev : { top, left, width, maxHeight },
    );
  }, [open, position, options.length]);

  useEffect(() => {
    if (!open || !position) return;
    const list = listRef.current;
    const option = list?.querySelector<HTMLElement>(`[data-index="${active}"]`);
    if (!list || !option) return;
    // Keep the active option visible without scrolling the dialog or page.
    const top = option.offsetTop;
    const bottom = top + option.offsetHeight;
    if (top < list.scrollTop) list.scrollTop = top;
    else if (bottom > list.scrollTop + list.clientHeight) list.scrollTop = bottom - list.clientHeight;
  }, [open, active, position]);

  useEffect(() => {
    if (!open) return;
    const inside = (target: EventTarget | null) =>
      Boolean(target instanceof Node && (buttonRef.current?.contains(target) || listRef.current?.contains(target)));
    const onMouseDown = (event: MouseEvent) => {
      if (!inside(event.target)) close(false);
    };
    // A queued scroll from focusing the trigger may arrive after opening.
    // Close only if an outside scroll actually moved the anchor.
    const onScroll = (event: Event) => {
      if (inside(event.target)) return;
      const rect = buttonRef.current?.getBoundingClientRect();
      const anchor = anchorRef.current;
      if (!rect || !anchor || rect.top !== anchor.top || rect.left !== anchor.left) close(false);
    };
    const onResize = () => close(false);
    window.addEventListener("mousedown", onMouseDown);
    window.addEventListener("scroll", onScroll, true);
    window.addEventListener("resize", onResize);
    return () => {
      window.removeEventListener("mousedown", onMouseDown);
      window.removeEventListener("scroll", onScroll, true);
      window.removeEventListener("resize", onResize);
    };
  });

  const onKeyDown = (event: React.KeyboardEvent) => {
    if (!open) {
      if (["ArrowDown", "ArrowUp", "Enter", " "].includes(event.key)) {
        event.preventDefault();
        openList();
      }
      return;
    }
    const last = options.length - 1;
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      close();
    } else if (event.key === "ArrowDown") {
      event.preventDefault();
      setActive((index) => Math.min(last, index + 1));
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      setActive((index) => Math.max(0, index - 1));
    } else if (event.key === "Home") {
      event.preventDefault();
      setActive(0);
    } else if (event.key === "End") {
      event.preventDefault();
      setActive(last);
    } else if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      choose(active);
    } else if (event.key === "Tab") {
      close(false);
    } else if (event.key.length === 1) {
      const now = Date.now();
      typed.current = { text: (now - typed.current.at < 700 ? typed.current.text : "") + event.key.toLowerCase(), at: now };
      const hit = options.findIndex((option) => option.label.toLowerCase().startsWith(typed.current.text));
      if (hit >= 0) setActive(hit);
    }
  };

  return (
    <>
      <button
        ref={buttonRef}
        type="button"
        id={id}
        role="combobox"
        aria-label={label}
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-controls={open ? listId : undefined}
        aria-activedescendant={open ? `${listId}-${active}` : undefined}
        data-testid={testId}
        data-value={value}
        disabled={disabled}
        title={current?.label}
        onClick={() => (open ? close() : openList())}
        onKeyDown={onKeyDown}
        className={cn(
          "inline-flex min-w-0 items-center justify-between gap-1.5 text-left text-foreground disabled:cursor-not-allowed disabled:opacity-50",
          className,
        )}
      >
        <span className={cn("truncate", mono && "font-mono")}>{current?.label ?? ""}</span>
        <ChevronDown className="h-3.5 w-3.5 shrink-0 text-muted-foreground" aria-hidden />
      </button>
      {open &&
        createPortal(
          <div
            ref={listRef}
            id={listId}
            role="listbox"
            aria-label={label}
            style={
              position
                ? { top: position.top, left: position.left, minWidth: position.width, maxHeight: position.maxHeight }
                : { visibility: "hidden", top: 0, left: 0 }
            }
            className="fixed z-[60] max-w-[min(32rem,calc(100vw-16px))] overflow-y-auto rounded-md border bg-card p-1 text-sm shadow-lg"
          >
            {options.map((option, index) => (
              <div
                key={option.value}
                id={`${listId}-${index}`}
                role="option"
                aria-selected={option.value === value}
                data-index={index}
                data-value={option.value}
                data-testid={testId ? `${testId}-option-${option.value}` : undefined}
                onMouseEnter={() => setActive(index)}
                onMouseDown={(event) => event.preventDefault()}
                onClick={() => choose(index)}
                className={cn(
                  "flex cursor-default items-center gap-2 rounded px-2 py-1.5",
                  index === active && "bg-accent",
                  mono && "font-mono text-xs",
                )}
              >
                <Check className={cn("h-3.5 w-3.5 shrink-0", option.value === value ? "text-primary" : "invisible")} aria-hidden />
                <span className="min-w-0 break-words">{option.label}</span>
              </div>
            ))}
          </div>,
          document.body,
        )}
    </>
  );
}
