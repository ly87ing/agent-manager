import { useLayoutEffect, useRef, useState } from "react";
import { Loader2, X } from "lucide-react";
import type { AgentRuntimeOptions } from "../lib/agentDefinitionForm";

const FOCUSABLE_SELECTOR =
  'button, [href], input, select, textarea, summary, [tabindex]:not([tabindex="-1"])';

/** 只管理助手运行时偏好的短表单；接入、登录和生命周期不在这里混用。 */
export function AgentRuntimeDialog({
  agentName,
  initial,
  supportsStatusline,
  busy,
  error,
  onSubmit,
  onClose,
}: {
  agentName: string;
  initial: AgentRuntimeOptions;
  supportsStatusline: boolean;
  busy: boolean;
  error?: string | null;
  onSubmit: (options: AgentRuntimeOptions) => void;
  onClose: () => void;
}) {
  const rootRef = useRef<HTMLElement>(null);
  const restoreFocusRef = useRef<HTMLElement | null>(
    typeof document === "undefined" ? null : (document.activeElement as HTMLElement | null),
  );
  const [options, setOptions] = useState(initial);

  useLayoutEffect(() => {
    return () => restoreFocusRef.current?.focus();
  }, []);

  const close = () => {
    if (!busy) onClose();
  };

  const onKeyDown = (event: React.KeyboardEvent<HTMLElement>) => {
    if (event.key === "Escape") {
      event.preventDefault();
      close();
      return;
    }
    if (event.key !== "Tab") return;
    const root = rootRef.current;
    if (!root) return;
    const focusable = Array.from(root.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR)).filter(
      (element) => !element.hasAttribute("disabled"),
    );
    if (focusable.length === 0) return;
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-slate-900/40 p-4 animate-fade-in"
      onClick={(event) => event.target === event.currentTarget && close()}
    >
      <section
        ref={rootRef}
        role="dialog"
        aria-modal="true"
        aria-label={`${agentName} 运行设置`}
        onKeyDown={onKeyDown}
        className="w-full max-w-lg rounded-xl border bg-card p-5 shadow-xl animate-zoom-in-95"
      >
        <div className="flex items-start justify-between gap-3">
          <div>
            <h3 className="text-base font-semibold text-foreground">{agentName} 运行设置</h3>
            <p className="mt-1 text-xs text-muted-foreground">
              这里只管启动后的行为。账号、接口地址、模型与凭据变量在接入卡片中维护。
            </p>
          </div>
          <button
            type="button"
            aria-label="关闭"
            disabled={busy}
            onClick={close}
            className="rounded p-1 text-muted-foreground hover:bg-accent disabled:opacity-50"
          >
            <X className="h-4 w-4" aria-hidden />
          </button>
        </div>

        <div className="mt-5 space-y-4 text-sm">
          {supportsStatusline && <div>
            <label htmlFor="agent-runtime-statusline" className="mb-1.5 block text-xs font-medium text-muted-foreground">
              状态栏
            </label>
            <select
              id="agent-runtime-statusline"
              data-testid="agent-runtime-statusline"
              value={options.statusline}
              disabled={busy}
              onChange={(event) =>
                setOptions((current) => ({
                  ...current,
                  statusline: event.target.value === "builtin" ? "builtin" : "untouched",
                }))
              }
              className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
            >
              <option value="untouched">保持助手现状</option>
              <option value="builtin">显示本工具提供的用量状态栏</option>
            </select>
          </div>}

          <label className="flex items-start gap-3 rounded-lg border border-destructive/25 bg-destructive/5 p-3">
            <input
              type="checkbox"
              data-testid="agent-runtime-yolo"
              checked={options.yolo}
              disabled={busy}
              onChange={(event) => setOptions((current) => ({ ...current, yolo: event.target.checked }))}
              className="mt-0.5"
            />
            <span>
              <span className="block font-medium text-foreground">跳过权限确认</span>
              <span className="mt-1 block text-xs text-muted-foreground">
                开启后，助手执行命令和修改文件时不再逐次询问。保存时还会要求再次确认。
              </span>
            </span>
          </label>

          <p className="rounded-lg bg-muted px-3 py-2 text-xs text-muted-foreground">
            官方账号仍在助手自身完成登录；本工具不读取账号密码或令牌明文。
          </p>
          {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
        </div>

        <div className="mt-5 flex justify-end gap-2 border-t pt-4">
          <button
            type="button"
            disabled={busy}
            onClick={close}
            className="rounded-md border px-3 py-2 text-sm text-muted-foreground hover:bg-accent disabled:opacity-50"
          >
            取消
          </button>
          <button
            type="button"
            data-testid="agent-runtime-submit"
            disabled={busy}
            onClick={() => onSubmit(options)}
            className="inline-flex items-center gap-1.5 rounded-md bg-primary px-4 py-2 text-sm text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
          >
            {busy && <Loader2 className="h-4 w-4 animate-spin" aria-hidden />}
            {busy ? "正在写入…" : "保存并写入助手"}
          </button>
        </div>
      </section>
    </div>
  );
}
