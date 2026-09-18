import { useRef, useState } from "react";
import { toast } from "sonner";
import { useExecutions } from "./execution";

/**
 * 接管编排：把「发现候选」转成最小写操作序列并顺序执行。
 *
 * 每个候选是一个任务，任务内的步骤按序执行，任一步失败即中止该任务（后续任务继续）：
 * - `policy`：保存定义（policy:set，整条替换）。保存只写定义，不安装、不链接、不同步。
 * - `apply`：真实 apply（各域内部保留 preflight/写前校验、锁、外部修改检测、CAS 与审计）。
 *
 * 每一步都走与单项操作相同的执行通道（独立 run_id、执行记录、查询刷新），
 * silent 压掉逐步 toast，由本 hook 汇总反馈。
 */
export type TakeoverStep =
  | { kind: "policy"; key: string; definition: Record<string, unknown> }
  | { kind: "apply"; command: string; args: string[] };

export interface TakeoverTask {
  /** 勾选键，同时用于结果汇总定位。 */
  key: string;
  /** 人类可读任务名（进执行记录与失败日志）。 */
  label: string;
  steps: TakeoverStep[];
}

export interface TakeoverOutcome {
  succeeded: number;
  failures: { label: string; message: string }[];
}

/** 批量执行进度：批量任务可能跑几十秒，界面要说清「跑到第几项、正在做哪一项」。 */
export interface TakeoverProgress {
  /** 已结束的任务数（含失败）。 */
  done: number;
  total: number;
  /** 正在执行的任务名。 */
  current: string;
}

export function useTakeover() {
  const { execute } = useExecutions();
  const busyRef = useRef(false);
  const [executing, setExecuting] = useState(false);
  const [progress, setProgress] = useState<TakeoverProgress | null>(null);

  const run = async (tasks: TakeoverTask[]): Promise<TakeoverOutcome | null> => {
    if (busyRef.current || tasks.length === 0) return null;
    busyRef.current = true;
    setExecuting(true);
    let succeeded = 0;
    const failures: { label: string; message: string }[] = [];
    try {
      for (const [index, task] of tasks.entries()) {
        setProgress({ done: index, total: tasks.length, current: task.label });
        let ok = true;
        for (const step of task.steps) {
          const request =
            step.kind === "policy"
              ? {
                  command: "policy:set",
                  args: [
                    "--layer",
                    "local",
                    "--key",
                    step.key,
                    "--value",
                    JSON.stringify({ ...step.definition, _delete: false, _replace: true }),
                  ],
                }
              : { command: step.command, args: step.args };
          const envelope = await execute({ ...request, label: `接管 ${task.label}`, silent: true });
          if (!envelope?.ok) {
            const message = envelope?.errors[0]?.message ?? "引擎调用失败，详见任务记录";
            failures.push({ label: task.label, message });
            console.error(`[接管] ${task.label} 失败：${message}`);
            ok = false;
            break;
          }
        }
        if (ok) succeeded += 1;
      }
    } finally {
      busyRef.current = false;
      setExecuting(false);
      setProgress(null);
    }
    const summary = `接管完成：成功 ${succeeded}，失败 ${failures.length}`;
    if (failures.length > 0) {
      toast.error(`${summary}，失败详情见任务记录`);
    } else {
      toast.success(summary);
    }
    return { succeeded, failures };
  };

  return { run, executing, progress };
}
