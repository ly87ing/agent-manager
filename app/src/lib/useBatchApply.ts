import { useQueryClient } from "@tanstack/react-query";
import { invalidateEngineQueries } from "./queries";
import { useRef, useState } from "react";
import { toast } from "sonner";
import { useExecutions } from "./execution";
import { runEngine, engineErrorMessage } from "./engine";
import { newRunId } from "./runId";
import { batchCounts, type BatchResult } from "./batchResult";
import type { StatusData } from "./contract";
import type { SkillObserved } from "./observed";

/** 批量执行的一行目标：资源 apply 的最小范围（--only/--action/--agent），takeover 行追加 --allow-takeover。 */
export interface BatchApplyRow {
  elementId: string;
  agent: string;
  /** 实际引擎动作；同一个「批量启用」里首次启用用 add，重新启用用 enable。 */
  action: string;
  /** 人类可读行名（技能 @ 助手），用于执行记录与失败日志。 */
  label: string;
  /** 外部链接接管/移除：已经过一次汇总确认，apply 携带 --allow-takeover。 */
  takeover: boolean;
}

/**
 * 资源批量执行器：对目标行顺序逐行 apply，每行走与单行相同的
 * 执行通道（独立 run_id、执行记录；silent 压掉单行结果 toast），最后统一刷新；
 * 单行失败不中断后续行；结果按目标汇总，重试先读取当前状态。
 */
export function useBatchApply() {
  const { execute, recordBatch, batches } = useExecutions();
  const queryClient = useQueryClient();
  const busyRef = useRef(false);
  const [executing, setExecuting] = useState(false);

  const applyBatch = async (request: { command?: string; actionLabel: string; rows: BatchApplyRow[] }, retry?: BatchResult): Promise<BatchResult | null> => {
    if (busyRef.current || request.rows.length === 0 || batches.some((batch) => batch.running)) return null;
    busyRef.current = true;
    setExecuting(true);
    const batch: BatchResult = retry ? { ...retry, running: true, rows: retry.rows.map((item) => item.status === "failed" ? { ...item, status: "pending", message: undefined } : item) }
      : { id: newRunId(), command: request.command ?? "skills:apply", label: request.actionLabel, running: true, rows: request.rows.map((row) => ({ row, status: "pending" })) };
    recordBatch({ ...batch, rows: batch.rows.map((item) => ({ ...item })) });
    try {
      for (const item of batch.rows.filter((item) => item.status === "pending")) {
        const row = item.row;
        try {
          if (retry) {
            if (batch.command !== "skills:apply" || row.takeover) throw new Error("请在资源页重新预览并确认此项，不能自动重试外部链接接管");
            const fresh = await runEngine<StatusData>("skills:status");
            if (!fresh.ok) throw new Error(fresh.errors[0]?.message ?? "读取当前技能状态失败");
            const cell = (fresh.data.elements.find((element) => element.id === row.elementId)?.observed as SkillObserved | undefined)?.agents?.[row.agent];
            if (!cell?.actions?.includes(row.action)) throw new Error("目标状态已变化或不再允许此动作，请在技能页检查；未重复写入");
          }
          const args = ["--only", row.elementId, "--action", row.action, "--agent", row.agent];
          if (row.takeover) args.push("--allow-takeover");
          const envelope = await execute({
            command: batch.command,
            label: `${request.actionLabel} ${row.label}`,
            args,
            silent: true,
            refresh: false,
          });
          item.status = envelope?.ok ? "ok" : "failed";
          item.message = envelope?.ok ? undefined : envelope?.errors[0]?.message ?? "引擎调用失败，详见操作记录";
        } catch (error) {
          item.status = "failed";
          item.message = engineErrorMessage(error);
        }
        if (item.message) item.message = item.message.slice(0, 2048);
        recordBatch({ ...batch, rows: batch.rows.map((item) => ({ ...item })) });
      }
    } finally {
      batch.running = false;
      recordBatch({ ...batch, rows: batch.rows.map((item) => ({ ...item })) });
      try { await invalidateEngineQueries(queryClient, batch.command); }
      finally { busyRef.current = false; setExecuting(false); }
    }
    const { succeeded, failed } = batchCounts(batch);
    const summary = `批量${request.actionLabel}完成：成功 ${succeeded}，失败 ${failed}`;
    if (failed > 0) {
      toast.error(`${summary}，失败详情见任务记录`);
    } else {
      toast.success(summary);
    }
    return batch;
  };

  const retryFailed = (batch: BatchResult) => applyBatch({ command: batch.command, actionLabel: batch.label,
    rows: batch.rows.filter((item) => item.status === "failed").map((item) => item.row) }, batch);
  return { applyBatch, retryFailed, executing, result: batches[0] };
}
