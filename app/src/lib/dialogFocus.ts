import { useLayoutEffect, useRef, type KeyboardEvent, type RefObject } from "react";

const FOCUSABLE_SELECTOR =
  'button, [href], input, select, textarea, summary, [tabindex]:not([tabindex="-1"])';

/**
 * 让弹窗接管键盘焦点：打开时聚焦弹窗内控件，Tab 不逃到背景，关闭后回到触发点。
 * Escape 是否允许关闭仍由调用方按执行状态决定。
 */
export function useDialogFocus<T extends HTMLElement>({
  onEscape,
  initialFocusSelector,
  active = true,
}: {
  onEscape?: () => void;
  initialFocusSelector?: string;
  active?: boolean;
} = {}): {
  dialogRef: RefObject<T>;
  onDialogKeyDown: (event: KeyboardEvent<T>) => void;
} {
  const dialogRef = useRef<T>(null);
  const restoreFocusRef = useRef<HTMLElement | null>(null);

  useLayoutEffect(() => {
    if (!active) return undefined;
    const root = dialogRef.current;
    if (!root) return undefined;
    restoreFocusRef.current = document.activeElement as HTMLElement | null;
    const preferred = initialFocusSelector
      ? root.querySelector<HTMLElement>(initialFocusSelector)
      : null;
    const first = preferred
      ?? root.querySelector<HTMLElement>("[autofocus]")
      ?? root.querySelector<HTMLElement>(FOCUSABLE_SELECTOR)
      ?? root;
    first.focus();
    return () => {
      const previous = restoreFocusRef.current;
      if (previous?.isConnected) previous.focus();
    };
  }, [active, initialFocusSelector]);

  const onDialogKeyDown = (event: KeyboardEvent<T>) => {
    if (event.key === "Escape" && onEscape) {
      event.preventDefault();
      onEscape();
      return;
    }
    if (event.key !== "Tab") return;
    const root = dialogRef.current;
    if (!root) return;
    const focusable = Array.from(root.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR)).filter(
      (element) => !element.hasAttribute("disabled") && element.getAttribute("aria-hidden") !== "true",
    );
    if (focusable.length === 0) {
      event.preventDefault();
      root.focus();
      return;
    }
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    const active = document.activeElement;
    if (!root.contains(active)) {
      event.preventDefault();
      (event.shiftKey ? last : first).focus();
    } else if (event.shiftKey && active === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && active === last) {
      event.preventDefault();
      first.focus();
    }
  };

  return { dialogRef, onDialogKeyDown };
}
