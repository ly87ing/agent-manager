import { useEffect, useLayoutEffect, useRef, useState } from "react";
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
import { MCP_UNMANAGED_CONFLICT_MARKER } from "../lib/mcpImport";
import { COPY, planSummaryPhrase, RISK_COPY, RISK_ORDER } from "../lib/ui-copy";
import { cn } from "../lib/utils";
import { ErrorPanel } from "./ErrorPanel";

export { RISK_ORDER };

export function maxRiskOf(actions: PlanAction[]): RiskLevel {
  return actions.reduce<RiskLevel>(
    (max, action) => (RISK_ORDER.indexOf(action.risk) > RISK_ORDER.indexOf(max) ? action.risk : max),
    "read",
  );
}

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

export function RiskBadge({ risk, label }: { risk: RiskLevel; label?: string }) {
  const copy = RISK_COPY[risk];
  return (
    <span
      title={risk}
      className={cn("inline-flex rounded-full border px-2 py-0.5 text-xs font-medium whitespace-nowrap", copy.className)}
    >
      {label ?? copy.label}
    </span>
  );
}

export function PlanActionList({ actions }: { actions: PlanAction[] }) {
  if (actions.length === 0) {
    return <p className="py-6 text-center text-sm text-slate-400">没有需要执行的操作。</p>;
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
                {action.takeover === true && (
                  <span
                    data-testid="takeover-badge"
                    className="rounded border border-amber-300 bg-amber-50 px-1.5 py-0.5 text-[10px] font-medium text-amber-700"
                  >
                    接管
                  </span>
                )}
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

/**
 * 「将要执行的操作」预览弹窗（提案 §5：先 plan 后 apply）。
 * 风险级驱动确认形态：安装/卸载软件 → 输入元素名二次确认；需要联网 → 知情勾选；其余普通确认。
 */
export default function PlanPreviewModal({
  title,
  subtitle,
  plans,
  planError,
  requireConfirmText,
  confirmLabel = "确认执行",
  defaultDryRun = false,
  executing,
  resultErrors,
  onTakeoverRetry,
  onConfirm,
  onClose,
}: {
  /** 动作名（如「同步 MCP 服务」），显示在固定标题下方。 */
  title: string;
  subtitle?: string;
  /** null = 正在计算预览 */
  plans: { domain: string; actions: PlanAction[] }[] | null;
  planError?: string | null;
  /** 安装/卸载级二次确认：须输入此文本（元素 id）才能确认。 */
  requireConfirmText?: string;
  confirmLabel?: string;
  /** 打开弹窗时的演练默认值；用户修改后由弹窗自己的状态接管。 */
  defaultDryRun?: boolean;
  executing: boolean;
  /** 执行后引擎返回的 errors（失败时留在弹窗内展示）。 */
  resultErrors?: Diagnostic[] | null;
  /** MCP 显式接管通道：错误含「非受管同名条目」时展示接管重试按钮。 */
  onTakeoverRetry?: () => void;
  onConfirm: (dryRun: boolean) => void;
  onClose: () => void;
}) {
  const [dryRun, setDryRun] = useState(defaultDryRun);
  const [confirmText, setConfirmText] = useState("");
  const [networkAck, setNetworkAck] = useState(false);
  const dialogRef = useRef<HTMLDivElement>(null);
  // 渲染期捕获触发元素：自动聚焦在提交阶段发生，effect 里再读已晚
  const restoreFocusRef = useRef<HTMLElement | null>(null);
  if (restoreFocusRef.current === null && typeof document !== "undefined") {
    const active = document.activeElement;
    restoreFocusRef.current = active instanceof HTMLElement && active !== document.body ? active : null;
  }

  // 打开时主动把焦点移入弹窗一次（plans 刷新不重抢）；清理在节点移除前执行，焦点恢复不被重置
  useLayoutEffect(() => {
    const previous = restoreFocusRef.current;
    const root = dialogRef.current;
    if (root && !root.contains(document.activeElement)) {
      const first = root.querySelector<HTMLElement>(
        'button, [href], input, select, textarea, summary, [tabindex]:not([tabindex="-1"])'
      );
      (first ?? root).focus();
    }
    return () => {
      if (previous && previous.isConnected) previous.focus();
    };
  }, []);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !executing) onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [executing, onClose]);

  // 焦点环：Tab/Shift+Tab 在弹窗内循环，不逃到背景
  const onTabKeyDown = (event: React.KeyboardEvent) => {
    if (event.key !== "Tab") return;
    const root = dialogRef.current;
    if (!root) return;
    const focusables = Array.from(
      root.querySelectorAll<HTMLElement>(
        'button, [href], input, select, textarea, summary, [tabindex]:not([tabindex="-1"])'
      )
    ).filter((el) => !el.hasAttribute("disabled"));
    if (focusables.length === 0) return;
    const first = focusables[0];
    const last = focusables[focusables.length - 1];
    const active = document.activeElement;
    if (event.shiftKey && (active === first || !root.contains(active))) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && (active === last || !root.contains(active))) {
      event.preventDefault();
      first.focus();
    }
  };

  const actions = (plans ?? []).flatMap((plan) => plan.actions);
  const maxRisk = maxRiskOf(actions);
  const summary = RISK_ORDER.map((risk) => ({ risk, count: actions.filter((a) => a.risk === risk).length }));

  // MCP 显式接管：plan 预览或 apply 执行因未受管同名条目 fail-closed 时，提供带标注的重试入口
  const errorMessages = [planError ?? "", ...(resultErrors ?? []).map((diag) => diag.message)];
  const showTakeoverRetry = onTakeoverRetry !== undefined && errorMessages.some((msg) => msg.includes(MCP_UNMANAGED_CONFLICT_MARKER));

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
        aria-label={`${COPY.planPreviewTitle}：${title}`}
        tabIndex={-1}
        onKeyDown={onTabKeyDown}
        className="flex max-h-[90vh] w-full max-w-2xl flex-col overflow-hidden rounded-xl border border-slate-200 bg-white shadow-xl"
      >
        <div className="flex flex-shrink-0 items-start justify-between gap-3 px-5 pt-5">
          <div>
            <h3 className="text-base font-semibold text-slate-800">{COPY.planPreviewTitle}</h3>
            <p className="mt-0.5 text-sm font-medium text-indigo-700">{title}</p>
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

        <div className="mt-4 flex-1 space-y-3 overflow-y-auto px-5">
          {planError ? (
            <ErrorPanel title="预览加载失败" errors={[{ code: "PLAN_FAILED", element: null, message: planError }]} />
          ) : plans === null ? (
            <p className="flex items-center gap-2 py-6 text-sm text-slate-400">
              <Loader2 className="h-4 w-4 animate-spin" aria-hidden /> 正在计算将要执行的操作…
            </p>
          ) : (
            <>
              {actions.length > 0 && (
                <p data-testid="plan-phrase" className="text-sm text-slate-600">
                  {planSummaryPhrase(actions)}
                </p>
              )}
              <div className="flex flex-wrap gap-1.5" data-testid="plan-summary">
                {summary.map(({ risk, count }) => (
                  <span
                    key={risk}
                    data-testid={`plan-summary-${risk}`}
                    className={cn(
                      "rounded-full border px-2 py-0.5 text-xs",
                      count > 0 ? RISK_COPY[risk].className : "border-slate-100 bg-slate-50 text-slate-300",
                    )}
                  >
                    {RISK_COPY[risk].label} {count}
                  </span>
                ))}
              </div>
              <PlanActionList actions={actions} />
            </>
          )}

          {resultErrors && resultErrors.length > 0 && (
            <ErrorPanel title="执行失败" errors={resultErrors} />
          )}

          {showTakeoverRetry && (
            <div
              data-testid="takeover-retry-panel"
              className="flex flex-wrap items-center justify-between gap-2 rounded-md border border-amber-200 bg-amber-50 px-3 py-2"
            >
              <p className="text-xs text-amber-800">
                目标文件中已存在同名的外部条目，默认拒绝覆盖。确认其内容可弃后，可显式接管：以本工具的标准配置替换该条目（其余条目与字段保持不变）。
              </p>
              <button
                type="button"
                data-testid="takeover-retry-button"
                disabled={executing}
                onClick={onTakeoverRetry}
                className="rounded border border-amber-300 bg-white px-2.5 py-1 text-xs font-medium text-amber-700 hover:bg-amber-100 disabled:opacity-40"
              >
                接管原条目并重试
              </button>
            </div>
          )}

          <label className="flex items-center gap-2 text-xs text-slate-600">
            <input
              type="checkbox"
              checked={dryRun}
              onChange={(event) => setDryRun(event.target.checked)}
              disabled={executing}
              className="h-3.5 w-3.5"
            />
            {COPY.dryRunToggle}
          </label>

          {plans !== null && maxRisk === "install-remove" && (
            <div className="rounded-md border border-red-200 bg-red-50 p-3">
              <p className="text-xs text-red-700">
                包含安装/卸载软件的操作。二次确认：请输入{" "}
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
              包含需要联网的操作：我已知晓将从官方渠道拉取/下载。
            </label>
          )}
        </div>

        <div className="flex flex-shrink-0 items-center justify-end gap-2 border-t border-slate-100 px-5 py-4">
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
              {dryRun ? "开始演练" : confirmLabel}
            </button>
        </div>
      </div>
    </div>
  );
}
