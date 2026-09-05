import { useEffect, useRef, useState } from "react";
import {
  ArrowDownToLine,
  Download,
  FileCog,
  FileText,
  Link2,
  Loader2,
  Terminal,
  Trash2,
  Unlink,
  X,
} from "lucide-react";
import type { Diagnostic, PlanAction, RiskLevel } from "../lib/contract";
import { cn } from "../lib/utils";
import { ErrorPanel } from "./ErrorPanel";

/** 风险级驱动确认形态（提案 §5 控制原则 2）：read < write-local < exec-local < network < install-remove。 */
export const RISK_ORDER: RiskLevel[] = ["read", "write-local", "exec-local", "network", "install-remove"];

export function maxRiskOf(actions: PlanAction[]): RiskLevel {
  return actions.reduce<RiskLevel>(
    (max, action) => (RISK_ORDER.indexOf(action.risk) > RISK_ORDER.indexOf(max) ? action.risk : max),
    "read",
  );
}

const RISK_META: Record<RiskLevel, { label: string; className: string }> = {
  read: { label: "读取", className: "border-slate-200 bg-slate-50 text-slate-500" },
  "write-local": { label: "写本地", className: "border-sky-200 bg-sky-50 text-sky-700" },
  "exec-local": { label: "本地执行", className: "border-amber-200 bg-amber-50 text-amber-700" },
  network: { label: "网络", className: "border-orange-200 bg-orange-50 text-orange-700" },
  "install-remove": { label: "安装/卸载", className: "border-red-200 bg-red-50 text-red-700" },
};

const OP_ICONS: Record<string, typeof FileText> = {
  "write-file": FileText,
  "run-cmd": Terminal,
  symlink: Link2,
  unlink: Unlink,
  install: Download,
  uninstall: Trash2,
  "git-pull": ArrowDownToLine,
  render: FileCog,
};

export function RiskBadge({ risk }: { risk: RiskLevel }) {
  const meta = RISK_META[risk];
  return (
    <span className={cn("inline-flex rounded-full border px-2 py-0.5 text-xs font-medium whitespace-nowrap", meta.className)}>
      {meta.label}
    </span>
  );
}

export function PlanActionList({ actions }: { actions: PlanAction[] }) {
  if (actions.length === 0) {
    return <p className="py-6 text-center text-sm text-slate-400">plan 为空：没有需要执行的动作。</p>;
  }
  return (
    <ul className="max-h-72 space-y-1.5 overflow-y-auto pr-1">
      {actions.map((action, index) => {
        const Icon = OP_ICONS[action.op] ?? FileText;
        return (
          <li
            key={`${action.op}-${action.element}-${action.target ?? action.cmd ?? index}`}
            className="flex items-start gap-2.5 rounded-md border border-slate-100 bg-slate-50 px-3 py-2 text-xs"
          >
            <Icon className="mt-0.5 h-3.5 w-3.5 shrink-0 text-slate-400" aria-hidden />
            <div className="min-w-0 flex-1">
              <div className="flex flex-wrap items-center gap-1.5">
                <code className="font-mono font-medium text-slate-700">{action.op}</code>
                <RiskBadge risk={action.risk} />
                <span className="font-mono text-slate-400">{action.element}</span>
              </div>
              {(action.target || action.cmd) && (
                <p className="mt-0.5 font-mono break-all text-slate-600">{action.target ?? action.cmd}</p>
              )}
              {action.note && <p className="mt-0.5 text-slate-500">{action.note}</p>}
            </div>
          </li>
        );
      })}
    </ul>
  );
}

export default function PlanPreviewModal({
  title,
  subtitle,
  plans,
  planError,
  requireConfirmText,
  confirmLabel = "确认执行",
  executing,
  resultErrors,
  onConfirm,
  onClose,
}: {
  title: string;
  subtitle?: string;
  /** null = plan 加载中 */
  plans: { domain: string; actions: PlanAction[] }[] | null;
  planError?: string | null;
  /** install-remove 级二次确认：须输入此文本（元素 id）才能确认。 */
  requireConfirmText?: string;
  confirmLabel?: string;
  executing: boolean;
  /** 执行后引擎返回的 errors（ok:false 时留在弹窗内展示）。 */
  resultErrors?: Diagnostic[] | null;
  onConfirm: (dryRun: boolean) => void;
  onClose: () => void;
}) {
  const [dryRun, setDryRun] = useState(false);
  const [confirmText, setConfirmText] = useState("");
  const [networkAck, setNetworkAck] = useState(false);
  const dialogRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !executing) onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [executing, onClose]);

  const actions = (plans ?? []).flatMap((plan) => plan.actions);
  const maxRisk = maxRiskOf(actions);
  const summary = RISK_ORDER.map((risk) => ({ risk, count: actions.filter((a) => a.risk === risk).length }));

  const confirmBlocked =
    plans === null || planError != null || actions.length === 0 || executing ||
    (maxRisk === "install-remove" && confirmText !== (requireConfirmText ?? "")) ||
    (maxRisk === "network" && !networkAck);

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-slate-900/40 p-4"
      onClick={(event) => {
        if (event.target === event.currentTarget && !executing) onClose();
      }}
    >
      <div
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-label={title}
        className="w-full max-w-2xl rounded-xl border border-slate-200 bg-white p-5 shadow-xl"
      >
        <div className="flex items-start justify-between gap-3">
          <div>
            <h3 className="text-base font-semibold text-slate-800">{title}</h3>
            {subtitle && <p className="mt-0.5 text-xs text-slate-400">{subtitle}</p>}
          </div>
          <button
            type="button"
            onClick={onClose}
            disabled={executing}
            aria-label="关闭"
            className="rounded p-1 text-slate-400 hover:bg-slate-100 hover:text-slate-600 disabled:opacity-40"
          >
            <X className="h-4 w-4" aria-hidden />
          </button>
        </div>

        <div className="mt-4 space-y-3">
          {planError ? (
            <ErrorPanel title="plan 加载失败" errors={[{ code: "PLAN_FAILED", element: null, message: planError }]} />
          ) : plans === null ? (
            <p className="flex items-center gap-2 py-6 text-sm text-slate-400">
              <Loader2 className="h-4 w-4 animate-spin" aria-hidden /> 正在计算 plan 预览…
            </p>
          ) : (
            <>
              <div className="flex flex-wrap gap-1.5" data-testid="plan-summary">
                {summary.map(({ risk, count }) => (
                  <span
                    key={risk}
                    data-testid={`plan-summary-${risk}`}
                    className={cn(
                      "rounded-full border px-2 py-0.5 text-xs",
                      count > 0 ? RISK_META[risk].className : "border-slate-100 bg-slate-50 text-slate-300",
                    )}
                  >
                    {RISK_META[risk].label} {count}
                  </span>
                ))}
              </div>
              <PlanActionList actions={actions} />
            </>
          )}

          {resultErrors && resultErrors.length > 0 && (
            <ErrorPanel title="执行失败" errors={resultErrors} />
          )}
        </div>

        <div className="mt-4 space-y-3 border-t border-slate-100 pt-4">
          <label className="flex items-center gap-2 text-xs text-slate-600">
            <input
              type="checkbox"
              checked={dryRun}
              onChange={(event) => setDryRun(event.target.checked)}
              disabled={executing}
              className="h-3.5 w-3.5"
            />
            仅预演（dry-run）：输出计划动作但不写入任何变更
          </label>

          {plans !== null && maxRisk === "install-remove" && (
            <div className="rounded-md border border-red-200 bg-red-50 p-3">
              <p className="text-xs text-red-700">
                包含安装/卸载级动作（install-remove）。二次确认：请输入{" "}
                <code className="font-mono font-semibold">{requireConfirmText}</code> 以确认。
              </p>
              <input
                type="text"
                value={confirmText}
                onChange={(event) => setConfirmText(event.target.value)}
                disabled={executing}
                placeholder={requireConfirmText}
                className="mt-2 w-full rounded border border-red-200 bg-white px-2 py-1.5 font-mono text-xs"
                data-testid="install-remove-confirm-input"
              />
            </div>
          )}
          {plans !== null && maxRisk === "network" && (
            <label className="flex items-center gap-2 rounded-md border border-orange-200 bg-orange-50 p-3 text-xs text-orange-800">
              <input
                type="checkbox"
                checked={networkAck}
                onChange={(event) => setNetworkAck(event.target.checked)}
                disabled={executing}
                className="h-3.5 w-3.5"
                data-testid="network-ack-checkbox"
              />
              包含联网动作（network）：我已知晓将从上游拉取/下载。
            </label>
          )}

          <div className="flex items-center justify-end gap-2">
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
              data-testid="plan-confirm-button"
              disabled={confirmBlocked}
              onClick={() => onConfirm(dryRun)}
              className={cn(
                "inline-flex items-center gap-2 rounded-md px-4 py-2 text-sm font-medium text-white",
                confirmBlocked ? "cursor-not-allowed bg-slate-300" : maxRisk === "install-remove" ? "bg-red-600 hover:bg-red-700" : "bg-indigo-600 hover:bg-indigo-700",
              )}
            >
              {executing && <Loader2 className="h-4 w-4 animate-spin" aria-hidden />}
              {dryRun ? "预演执行（dry-run）" : confirmLabel}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
