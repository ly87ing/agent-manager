import { useLayoutEffect, useRef, useState } from "react";
import { X } from "lucide-react";
import { parseConnectionBundle, type ConnectionEntry } from "../lib/agentDefinitionForm";
import type { ConnectionKind } from "./ConnectionFormDialog";

const FOCUSABLE_SELECTOR =
  'button, [href], input, select, textarea, summary, [tabindex]:not([tabindex="-1"])';

export function ConnectionImportDialog({
  kind,
  existingNames,
  busy,
  onImport,
  onClose,
}: {
  kind: ConnectionKind;
  existingNames: string[];
  busy: boolean;
  onImport: (connection: ConnectionEntry) => void;
  onClose: () => void;
}) {
  const rootRef = useRef<HTMLElement>(null);
  const restoreFocusRef = useRef<HTMLElement | null>(
    typeof document === "undefined" ? null : (document.activeElement as HTMLElement | null),
  );
  const [text, setText] = useState("");
  const [preview, setPreview] = useState<ConnectionEntry | null>(null);
  const [error, setError] = useState<string | null>(null);

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

  const inspect = () => {
    try {
      const connection = parseConnectionBundle(text.trim(), kind);
      if (existingNames.includes(connection.name)) {
        throw new Error(`已经有名为 ${connection.name} 的接入，请先修改分享内容里的唯一名称`);
      }
      setPreview(connection);
      setError(null);
    } catch (err) {
      setPreview(null);
      setError(err instanceof Error ? err.message : String(err));
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
        aria-label="导入接入"
        onKeyDown={onKeyDown}
        className="w-full max-w-xl rounded-xl border bg-card p-5 shadow-xl animate-zoom-in-95"
      >
        <div className="flex items-start justify-between gap-3">
          <div>
            <h2 className="text-lg font-semibold text-foreground">导入接入</h2>
            <p className="mt-1 text-sm text-muted-foreground">
              只保存连接定义，不导入凭据取值、不启用接入，也不会修改助手配置或认证文件。
            </p>
          </div>
          <button type="button" aria-label="关闭" disabled={busy} onClick={close} className="rounded p-1 text-muted-foreground hover:bg-accent disabled:opacity-50">
            <X className="h-4 w-4" aria-hidden />
          </button>
        </div>
        <label className="mt-4 block text-sm font-medium text-foreground" htmlFor="connection-import-text">接入分享 JSON</label>
        <textarea
          id="connection-import-text"
          data-testid="connection-import-text"
          autoFocus
          rows={10}
          value={text}
          onChange={(event) => { setText(event.target.value); setPreview(null); setError(null); }}
          spellCheck={false}
          className="mt-1 w-full rounded border border-input p-2 font-mono text-xs"
        />
        {error && <p role="alert" className="mt-2 text-sm text-destructive">{error}</p>}
        {preview && (
          <div data-testid="connection-import-preview" className="mt-3 rounded-lg border bg-muted/30 p-3 text-sm">
            <p className="font-medium text-foreground">将新增：{preview.label || preview.name}</p>
            <p className="mt-1 break-all font-mono text-xs text-muted-foreground">{preview.baseUrl}</p>
            <p className="mt-1 text-xs text-muted-foreground">模型 {preview.model} · 凭据变量 {preview.tokenVar}</p>
          </div>
        )}
        <div className="mt-4 flex justify-end gap-2">
          <button type="button" disabled={busy} onClick={close} className="rounded border px-3 py-2 text-sm text-muted-foreground hover:bg-accent disabled:opacity-50">取消</button>
          {!preview ? (
            <button type="button" data-testid="connection-import-inspect" disabled={!text.trim()} onClick={inspect} className="rounded bg-primary px-3 py-2 text-sm text-primary-foreground disabled:opacity-50">检查内容</button>
          ) : (
            <button type="button" data-testid="connection-import-confirm" disabled={busy} onClick={() => onImport(preview)} className="rounded bg-primary px-3 py-2 text-sm text-primary-foreground disabled:opacity-50">保存但不启用</button>
          )}
        </div>
      </section>
    </div>
  );
}
