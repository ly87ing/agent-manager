import { X } from "lucide-react";
import { useState } from "react";
import { useDialogFocus } from "../lib/dialogFocus";
import { decodeShareLink, type SharePayload } from "../lib/shareLink";

/**
 * 从分享链接导入：粘贴（或点开）链接后先把里面的内容摆出来，确认后交给对应页面的「新增」表单预填，
 * 由用户检查后保存。这里不写任何东西。
 */
export function ImportLinkDialog({
  initialText = "",
  onContinue,
  onClose,
}: {
  initialText?: string;
  onContinue: (payload: SharePayload) => void;
  onClose: () => void;
}) {
  const [text, setText] = useState(initialText);
  const { dialogRef, onDialogKeyDown } = useDialogFocus<HTMLDivElement>({ onEscape: onClose });
  const parsed = text.trim() ? decodeShareLink(text) : null;
  const payload = parsed && typeof parsed !== "string" ? parsed : null;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-slate-900/40 p-4" onClick={(event) => event.target === event.currentTarget && onClose()}>
      <div ref={dialogRef} role="dialog" aria-modal="true" aria-label="从分享链接导入" tabIndex={-1} onKeyDown={onDialogKeyDown} className="w-full max-w-lg rounded-xl border bg-card shadow-xl">
        <div className="flex items-center justify-between border-b px-5 py-3">
          <h3 className="text-base font-semibold">从分享链接导入</h3>
          <button type="button" aria-label="关闭" onClick={onClose} className="rounded p-1 text-muted-foreground hover:bg-accent">
            <X className="h-4 w-4" aria-hidden />
          </button>
        </div>
        <div className="space-y-3 px-5 py-4 text-sm">
          <textarea
            data-testid="import-link-input"
            aria-label="分享链接"
            value={text}
            onChange={(event) => setText(event.target.value)}
            rows={3}
            spellCheck={false}
            placeholder="粘贴 agentmanager://import?data=… 链接"
            className="w-full rounded-md border border-input bg-background px-2.5 py-1.5 font-mono text-xs"
          />
          {typeof parsed === "string" && <p role="alert" className="text-xs text-destructive">{parsed}</p>}
          {payload && (
            <div data-testid="import-link-summary" className="space-y-1 rounded-md border bg-muted/40 p-3 text-xs">
              <p>
                <span className="text-muted-foreground">{payload.kind === "mcp" ? "MCP 服务" : "技能来源"}：</span>
                <span className="font-mono font-medium">{payload.name}</span>
              </p>
              {payload.kind === "mcp" && (
                <p className="break-all font-mono text-muted-foreground">
                  {payload.spec.url ?? [payload.spec.command, ...(payload.spec.args ?? [])].join(" ")}
                </p>
              )}
              {payload.kind === "mcp" && Object.keys(payload.spec.env ?? payload.spec.headers ?? {}).length > 0 && (
                <p className="text-muted-foreground">
                  需要的变量：{Object.values(payload.spec.env ?? payload.spec.headers ?? {}).join("、")}（取值要在本机自己配）
                </p>
              )}
              {payload.kind === "skills" && <p className="break-all font-mono text-muted-foreground">{payload.definition.url}</p>}
              <p className="pt-1 text-muted-foreground">下一步打开「新增」表单并预填这些内容，检查后由你保存；在那之前什么都不会写入。</p>
            </div>
          )}
        </div>
        <div className="flex justify-end gap-2 border-t px-5 py-3">
          <button type="button" onClick={onClose} className="rounded-md border px-4 py-1.5 text-sm text-muted-foreground hover:bg-accent">
            取消
          </button>
          <button
            type="button"
            data-testid="import-link-continue"
            disabled={!payload}
            onClick={() => payload && onContinue(payload)}
            className="rounded-md bg-primary px-4 py-1.5 text-sm font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-40"
          >
            下一步
          </button>
        </div>
      </div>
    </div>
  );
}
