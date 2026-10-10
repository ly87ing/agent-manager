import { useEffect, useState } from "react";
import { runEngine } from "./engine";

/** 引擎报告的一步：pending 未开始、active 进行中、done 完成、failed 失败。 */
export interface ProgressStep {
  key: string;
  label: string;
  state: "pending" | "active" | "done" | "failed" | string;
  detail: string | null;
}

export interface EngineProgress {
  started_at: number;
  elapsed_sec?: number;
  steps: ProgressStep[];
}

/**
 * 长操作进行中时按运行编号轮询引擎进度（runs:progress，只读）。runId 为 null 时不轮询。
 * 进度只是提示：读不到时保留上一次的结果，不打断操作本身。
 */
export function useEngineProgress(runId: string | null, intervalMs = 500): EngineProgress | null {
  const [progress, setProgress] = useState<EngineProgress | null>(null);
  useEffect(() => {
    setProgress(null);
    if (!runId) return;
    let alive = true;
    let pending = false;
    const poll = () => {
      if (pending) return;
      pending = true;
      void runEngine<{ progress: EngineProgress | null }>("runs:progress", ["--run", runId])
        .then((envelope) => {
          if (alive && envelope.ok && envelope.data.progress) setProgress(envelope.data.progress);
        })
        .catch(() => undefined)
        .finally(() => { pending = false; });
    };
    void poll();
    const timer = setInterval(() => void poll(), intervalMs);
    return () => {
      alive = false;
      clearInterval(timer);
    };
  }, [runId, intervalMs]);
  return progress;
}
