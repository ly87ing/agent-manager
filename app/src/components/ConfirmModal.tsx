import { useEffect, useState } from "react";
import { Loader2, X } from "lucide-react";
import type { Diagnostic, RiskLevel } from "../lib/contract";
import { cn } from "../lib/utils";
import { ErrorPanel } from "./ErrorPanel";
import { RiskBadge } from "./PlanPreviewModal";

/**
 * 非 plan 类动作的确认弹窗（例如清理诊断日志、移除定义）。
 * 风险级驱动：install-remove 级须输入 requireText 二次确认；其余单次确认。
 */
export function ConfirmModal({
  title,
  description,
  risk,
  riskLabel,
  requireText,
  confirmLabel = "确认执行",
  executing,
  resultErrors,
  onConfirm,
  onClose,
}: {
  title: string;
  description: string;
  risk: RiskLevel;
  riskLabel?: string;
  requireText?: string;
  confirmLabel?: string;
  executing: boolean;
  resultErrors?: Diagnostic[] | null;
  onConfirm: () => void;
  onClose: () => void;
}) {
  const [text, setText] = useState("");

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !executing) onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [executing, onClose]);

  const needsText = requireText !== undefined;
  const blocked = executing || (needsText && text !== requireText);

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-slate-900/40 p-4"
      onClick={(event) => {
        if (event.target === event.currentTarget && !executing) onClose();
      }}
    >
      <div role="dialog" aria-modal="true" aria-label={title} className="w-full max-w-lg rounded-xl border border-slate-200 bg-white p-5 shadow-xl">
        <div className="flex items-start justify-between gap-3">
          <div className="flex items-center gap-2">
            <h3 className="text-base font-semibold text-slate-800">{title}</h3>
            <RiskBadge risk={risk} label={riskLabel} />
          </div>
          <button type="button" onClick={onClose} disabled={executing} aria-label="关闭" className="rounded p-1 text-slate-400 hover:bg-slate-100 disabled:opacity-40">
            <X className="h-4 w-4" aria-hidden />
          </button>
        </div>

        <p className="mt-3 text-sm text-slate-600">{description}</p>

        {needsText && (
          <input
            type="text"
            value={text}
            onChange={(event) => setText(event.target.value)}
            disabled={executing}
            placeholder={`请输入 ${requireText} 以确认`}
            data-testid="confirm-modal-input"
            className="mt-3 w-full rounded border border-slate-300 px-2 py-1.5 font-mono text-xs"
          />
        )}

        {resultErrors && resultErrors.length > 0 && <div className="mt-3"><ErrorPanel title="执行失败" errors={resultErrors} /></div>}

        <div className="mt-4 flex justify-end gap-2">
          <button
            type="button"
            onClick={onClose}
            disabled={executing}
            className="rounded-md border border-slate-200 px-4 py-2 text-sm text-slate-600 hover:bg-slate-50 disabled:opacity-40"
          >
            取消
          </button>
          <button
            type="button"
            data-testid="confirm-modal-confirm"
            disabled={blocked}
            onClick={onConfirm}
            className={cn(
              "inline-flex items-center gap-2 rounded-md px-4 py-2 text-sm font-medium text-white",
              blocked ? "cursor-not-allowed bg-slate-300" : risk === "install-remove" ? "bg-red-600 hover:bg-red-700" : "bg-indigo-600 hover:bg-indigo-700",
            )}
          >
            {executing && <Loader2 className="h-4 w-4 animate-spin" aria-hidden />}
            {confirmLabel}
          </button>
        </div>
      </div>
    </div>
  );
}
