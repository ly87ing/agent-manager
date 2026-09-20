import { useRef, useState } from "react";
import { toast } from "sonner";
import { useExecutions } from "./execution";

/** 批量执行的一行目标：skills:apply 的最小范围（--only/--action/--agent），takeover 行追加 --allow-takeover。 */
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
 * 全部技能矩阵的批量执行器：对目标行顺序逐行 skills:apply，每行走与单行相同的
 * 执行通道（独立 run_id、执行记录、查询刷新；silent 压掉单行结果 toast）；
 * 单行失败不中断后续行；全部完成后汇总 toast（失败时指向任务记录并逐行
 * console.error 原因）；同步 ref 保证在途期间拒绝重入。
 */
export function useBatchApply() {
  const { execute } = useExecutions();
  const busyRef = useRef(false);
  const [executing, setExecuting] = useState(false);

  const applyBatch = async (request: { actionLabel: string; rows: BatchApplyRow[] }): Promise<void> => {
    if (busyRef.current || request.rows.length === 0) return;
    busyRef.current = true;
    setExecuting(true);
    let succeeded = 0;
    const failures: { label: string; message: string }[] = [];
    try {
      for (const row of request.rows) {
        const args = ["--only", row.elementId, "--action", row.action, "--agent", row.agent];
        if (row.takeover) args.push("--allow-takeover");
        const envelope = await execute({
          command: "skills:apply",
          label: `${request.actionLabel} ${row.label}`,
          args,
          // 单行结果 toast 由下方汇总 toast 取代；执行记录与查询刷新照常。
          silent: true,
        });
        if (envelope?.ok) {
          succeeded += 1;
        } else {
          const message = envelope?.errors[0]?.message ?? "引擎调用失败，详见任务记录";
          failures.push({ label: row.label, message });
          console.error(`[批量${request.actionLabel}] ${row.label} 失败：${message}`);
        }
      }
    } finally {
      busyRef.current = false;
      setExecuting(false);
    }
    const summary = `批量${request.actionLabel}完成：成功 ${succeeded}，失败 ${failures.length}`;
    if (failures.length > 0) {
      toast.error(`${summary}，失败详情见任务记录`);
    } else {
      toast.success(summary);
    }
  };

  return { applyBatch, executing };
}
