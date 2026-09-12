import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import PlanPreviewModal from "../components/PlanPreviewModal";
import type { Diagnostic, Envelope, PlanData, SettingsData } from "../lib/contract";
import { engineErrorMessage, runEngine } from "../lib/engine";
import { useExecutions } from "../lib/execution";
import { engineKeys } from "../lib/queries";
import { fetchSettings, settingsKey } from "./settings";

export type PlanCommandSpec = string | { command: string; args?: string[] };

export interface OpenPreviewOptions {
  title: string;
  subtitle?: string;
  /** 取这些域的 plan 做预览（多域聚合时传多个；可带参数如 { command: "mcp:plan", args: ["--only", "mcp:demo"] }）。 */
  planCommands: PlanCommandSpec[];
  /** 只保留该元素的动作（仅在未通过 args 传 --only 时作为客户端回退）。 */
  filterElement?: string;
  /** 只保留这些元素的动作（仅在未通过 args 传 --only 时作为客户端回退）。 */
  filterElements?: ReadonlySet<string>;
  /** install-remove 二次确认文本（通常元素 id）。 */
  requireConfirmText?: string;
  confirmLabel?: string;
  defaultDryRun?: boolean;
  /** 确认后执行（经 useExecutions.execute 走 run_id 追踪）。返回 envelope；null = 桥层失败。 */
  execute: (dryRun: boolean) => Promise<Envelope | null>;
}

interface PreviewState {
  options: OpenPreviewOptions;
  plans: { domain: string; actions: PlanData["actions"] }[] | null;
  planError: string | null;
  executing: boolean;
  resultErrors: Diagnostic[] | null;
}

/**
 * 「先 plan 后 apply」编排 hook（提案 §5 控制原则 1）：
 * open() 取 plan（staleTime 0 强制新鲜）→ 预览弹窗 → onConfirm 执行 → ok 关窗 / 失败留窗展示 errors。
 */
export function usePlanPreview() {
  const queryClient = useQueryClient();
  const settings = queryClient.getQueryData<SettingsData>(settingsKey);
  const [state, setState] = useState<PreviewState | null>(null);

  const open = (options: OpenPreviewOptions) => {
    void (async () => {
      try {
        const settingsData = await queryClient.ensureQueryData({ queryKey: settingsKey, queryFn: fetchSettings, staleTime: 30_000 });
        const resolvedOptions = { ...options, defaultDryRun: options.defaultDryRun ?? settingsData.preferences.defaultDryRun };
        setState({ options: resolvedOptions, plans: null, planError: null, executing: false, resultErrors: null });
        const plans: { domain: string; actions: PlanData["actions"] }[] = [];
        for (const spec of resolvedOptions.planCommands) {
          const command = typeof spec === "string" ? spec : spec.command;
          const args = typeof spec === "string" ? undefined : spec.args;
          const envelope = await queryClient.fetchQuery({
            queryKey: args && args.length > 0 ? ["engine", command, ...args] : engineKeys.command(command),
            queryFn: () => runEngine<PlanData>(command, args),
            staleTime: 0,
          });
          if (!envelope.ok) {
            throw new Error(envelope.errors[0]?.message ?? `${command} 返回错误`);
          }
          let actions = envelope.data.actions;
          const hasEngineScope = args && args.includes("--only");
          if (!hasEngineScope) {
            if (resolvedOptions.filterElement) {
              actions = actions.filter((action) => action.element === resolvedOptions.filterElement);
            } else if (resolvedOptions.filterElements) {
              actions = actions.filter((action) => resolvedOptions.filterElements!.has(action.element));
            }
          }
          plans.push({ domain: envelope.data.domain, actions });
        }
        // install-remove 级且调用方未指定确认文本时，回退用被影响元素 id（如「停用助手→卸载」场景）
        const allActions = plans.flatMap((plan) => plan.actions);
        const hasInstallRemove = allActions.some((action) => action.risk === "install-remove");
        const firstInstallRemoveElement = allActions.find((action) => action.risk === "install-remove")?.element;
        const fallbackConfirmText =
          resolvedOptions.requireConfirmText ??
          (hasInstallRemove ? (resolvedOptions.filterElement ?? firstInstallRemoveElement ?? "确认执行") : undefined);
        setState((prev) =>
          prev ? { ...prev, plans, options: { ...prev.options, requireConfirmText: fallbackConfirmText } } : prev,
        );
      } catch (err) {
        setState({ options, plans: null, planError: engineErrorMessage(err), executing: false, resultErrors: null });
      }
    })();
  };

  const confirm = (dryRun: boolean) => {
    if (!state) return;
    setState({ ...state, executing: true, resultErrors: null });
    void state.options.execute(dryRun).then((envelope) => {
      setState((prev) => {
        if (!prev) return prev;
        if (envelope && !envelope.ok) {
          return { ...prev, executing: false, resultErrors: envelope.errors };
        }
        return null; // 成功（或桥层失败已 toast）→ 关窗
      });
    });
  };

  const modal = state ? (
    <PlanPreviewModal
      title={state.options.title}
      subtitle={state.options.subtitle}
      plans={state.plans}
      planError={state.planError}
      requireConfirmText={state.options.requireConfirmText}
      confirmLabel={state.options.confirmLabel}
      defaultDryRun={state.options.defaultDryRun ?? settings?.preferences.defaultDryRun ?? false}
      executing={state.executing}
      resultErrors={state.resultErrors}
      onConfirm={confirm}
      onClose={() => setState(null)}
    />
  ) : null;

  return { open, modal };
}

/** 供页面直接构造 execute 回调的小工具：把 dryRun 落到 args 与执行记录。 */
export function useExecuteApply() {
  const { execute } = useExecutions();
  return (request: { command: string; label: string; extraArgs?: string[] }) =>
    (dryRun: boolean) =>
      execute({
        command: request.command,
        args: [...(request.extraArgs ?? []), ...(dryRun ? ["--dry-run"] : [])],
        label: request.label,
        dryRun,
      });
}
