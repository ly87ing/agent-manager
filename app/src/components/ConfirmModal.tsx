import { t } from "../lib/i18n";
import { useState, type ReactNode } from "react";
import { Loader2, X } from "lucide-react";
import type { Diagnostic, RiskLevel } from "../lib/contract";
import { cn } from "../lib/utils";
import { useDialogFocus } from "../lib/dialogFocus";
import { ErrorPanel } from "./ErrorPanel";
import { RiskBadge } from "./RiskBadge";

/**
 * 破坏性/独立确认动作的确认弹窗（例如卸载助手、移除定义、清空操作记录）。
 * 风险级驱动：install-remove 级须输入 requireText 二次确认；其余单次确认。
 */
export function ConfirmModal({
  title,
  description,
  risk,
  riskLabel,
  requireText,
  confirmLabel = t("确认执行"),
  confirmDisabled = false,
  executing,
  resultErrors,
  error,
  children,
  secondaryLabel,
  onSecondary,
  onConfirm,
  onClose,
}: {
  title: string;
  description: string;
  risk: RiskLevel;
  riskLabel?: string;
  requireText?: string;
  confirmLabel?: string;
  confirmDisabled?: boolean;
  executing: boolean;
  resultErrors?: Diagnostic[] | null;
  error?: string | null;
  children?: ReactNode;
  secondaryLabel?: string;
  onSecondary?: () => void;
  onConfirm: () => void;
  onClose: () => void;
}) {
  const [text, setText] = useState("");
  const needsText = requireText !== undefined;
  const blocked = executing || confirmDisabled || (needsText && text !== requireText);
  const { dialogRef, onDialogKeyDown } = useDialogFocus<HTMLDivElement>({
    onEscape: executing ? undefined : onClose,
    initialFocusSelector: needsText ? '[data-testid="confirm-modal-input"]' : '[data-testid="confirm-modal-cancel"]',
  });

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-slate-900/40 p-4 animate-fade-in"
      onClick={(event) => {
        if (event.target === event.currentTarget && !executing) onClose();
      }}
    >
      <div
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-busy={executing}
        aria-label={title}
        tabIndex={-1}
        onKeyDown={onDialogKeyDown}
        className="flex max-h-[calc(100vh-2rem)] w-full max-w-lg flex-col rounded-xl border bg-card p-5 shadow-xl animate-zoom-in-95"
      >
        {/* 标题与按钮固定在上下，内容长了只滚中间：不用拖动就能点确认或取消 */}
        <div className="flex shrink-0 items-start justify-between gap-3">
          <div className="flex min-w-0 flex-wrap items-center gap-2">
            <h3 className="break-words text-base font-semibold text-foreground">{title}</h3>
            <RiskBadge risk={risk} label={riskLabel} />
          </div>
          <button type="button" onClick={onClose} disabled={executing} aria-label={t("关闭")} className="rounded p-1 text-muted-foreground hover:bg-accent disabled:opacity-40">
            <X className="h-4 w-4" aria-hidden />
          </button>
        </div>

        <div data-testid="confirm-modal-body" className="-mx-5 mt-3 min-h-0 flex-1 overflow-y-auto px-5">
          <p className="text-sm text-muted-foreground">{description}</p>
          {children}

          {needsText && (
            <input
              type="text"
              value={text}
              onChange={(event) => setText(event.target.value)}
              disabled={executing}
              placeholder={t("请输入 {0} 以确认", requireText)}
              data-testid="confirm-modal-input"
              className="mt-3 w-full rounded border border-input px-2 py-1.5 font-mono text-xs"
            />
          )}

        </div>

        <div data-testid="confirm-modal-feedback" className="mt-4 max-h-32 shrink-0 overflow-y-auto break-words">
          {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
          {resultErrors && resultErrors.length > 0 && <ErrorPanel title={t("执行失败")} errors={resultErrors} />}
        </div>
        <div className="mt-3 flex shrink-0 flex-wrap justify-end gap-2">
          {secondaryLabel && onSecondary && (
            <button type="button" data-testid="confirm-modal-secondary" onClick={onSecondary} disabled={executing} className="rounded-md border border-primary/30 px-4 py-2 text-sm text-primary hover:bg-primary/10 disabled:opacity-40">
              {secondaryLabel}
            </button>
          )}
          <button
            type="button"
            data-testid="confirm-modal-cancel"
            onClick={onClose}
            disabled={executing}
            className="rounded-md border px-4 py-2 text-sm text-muted-foreground hover:bg-accent disabled:opacity-40"
          >{t("取消")}</button>
          <button
            type="button"
            data-testid="confirm-modal-confirm"
            disabled={blocked}
            onClick={onConfirm}
            className={cn(
              "inline-flex items-center gap-2 rounded-md px-4 py-2 text-sm font-medium text-primary-foreground",
              blocked ? "cursor-not-allowed bg-muted text-muted-foreground" : risk === "install-remove" ? "bg-destructive text-destructive-foreground hover:bg-destructive/90" : "bg-primary hover:bg-primary/90",
            )}
          >
            {executing && <Loader2 className="h-4 w-4 animate-spin" aria-hidden />}
            {executing && !confirmLabel.endsWith("…") ? t("正在处理…") : confirmLabel}
          </button>
        </div>
      </div>
    </div>
  );
}
