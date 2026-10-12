import { createContext, useCallback, useContext, useMemo, useState, type ReactNode } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { errorSummary } from "./errorAdvice";
import { effectHint } from "./effectHints";
import { engineErrorMessage, runEngine } from "./engine";
import type { Diagnostic, Envelope } from "./contract";
import { invalidateEngineQueries } from "./queries";
import { newRunId } from "./runId";
import type { BatchResult } from "./batchResult";

/**
 * GUI 发起的写路径执行记录（执行中心数据源）。
 * run_id 由前端生成并透传引擎，按次操作记录据此可追踪（runs:get --run <id>）。
 */
export interface ExecutionRecord {
  runId: string;
  command: string;
  label: string;
  startedAt: number;
  finishedAt: number | null;
  status: "running" | "ok" | "failed";
  /** 引擎 envelope（完成时）；ok:false 时 errors 非空。 */
  envelope: Envelope | null;
  /** 桥层错误（未拿到 envelope 时）。 */
  bridgeError: string | null;
}

interface ExecutionContextValue {
  executions: ExecutionRecord[];
  batches: BatchResult[];
  recordBatch: (batch: BatchResult) => void;
  execute: (request: ExecuteRequest) => Promise<Envelope | null>;
}

export interface ExecuteRequest {
  command: string;
  args?: string[];
  /** 人类可读的动作名，如「收敛 mcp 域」。 */
  label: string;
  /** 批量编排时压掉单行结果 toast（执行记录、run_id、查询刷新、错误返回全部保留），由调用方统一汇总反馈。 */
  silent?: boolean;
  /** 可逆的写入：成功提示上给「撤销」，点了执行反向操作。只给能干净回到原状的动作。 */
  undo?: () => void;
  /** 调用方预先生成的运行编号：长操作据此轮询进度（runs:progress）。缺省自动生成。 */
  runId?: string;
  /** Batch callers refresh affected domains once in finally. */
  refresh?: boolean;
}

export const COMPLETED_EXECUTION_LIMIT = 50;
const textSummary = (value: string) => value.length > 2048 ? value.slice(0, 2047) + "…" : value;

/** Keep only what the list renders; callers still receive the original envelope. */
export function executionSummary(envelope: Envelope): Envelope {
  const data = envelope.data as { steps?: Array<{ name: string; ok: boolean; exit_code: number }> } | null;
  return {
    ...envelope,
    data: { steps: Array.isArray(data?.steps) ? data.steps.filter(step => typeof step.name === "string" && typeof step.ok === "boolean" && typeof step.exit_code === "number").slice(0, 50).map(({ name, ok, exit_code }) => ({ name: textSummary(name), ok, exit_code })) : [] },
    warnings: [],
    errors: envelope.errors.slice(0, 20).map(error => ({ code: textSummary(error.code), element: error.element ? textSummary(error.element) : null, message: textSummary(error.message) })),
  };
}

export function boundedExecutions(records: ExecutionRecord[]): ExecutionRecord[] {
  let completed = 0;
  return records.filter(record => record.status === "running" || ++completed <= COMPLETED_EXECUTION_LIMIT);
}

const ExecutionContext = createContext<ExecutionContextValue | null>(null);

export function ExecutionProvider({ children }: { children: ReactNode }) {
  const [executions, setExecutions] = useState<ExecutionRecord[]>([]);
  const [batches, setBatches] = useState<BatchResult[]>([]);
  const recordBatch = useCallback((batch: BatchResult) => setBatches((previous) => {
    let completed = 0;
    return [batch, ...previous.filter((item) => item.id !== batch.id)]
      .filter((item) => item.running || ++completed <= COMPLETED_EXECUTION_LIMIT);
  }), []);
  const queryClient = useQueryClient();

  const execute = useCallback(
    async (request: ExecuteRequest): Promise<Envelope | null> => {
      const runId = request.runId ?? newRunId();
      const record: ExecutionRecord = {
        runId,
        command: request.command,
        label: request.label,
        startedAt: Date.now(),
        finishedAt: null,
        status: "running",
        envelope: null,
        bridgeError: null,
      };
      setExecutions((prev) => boundedExecutions([record, ...prev]));

      const patch = (update: Partial<ExecutionRecord>) => {
        setExecutions((prev) => boundedExecutions(prev.map((item) => (item.runId === runId ? { ...item, ...update } : item))));
      };

      let envelope: Envelope | null = null;
      try {
        envelope = await runEngine(request.command, request.args ?? [], { runId });
      } catch (err) {
        const message = engineErrorMessage(err);
        patch({ status: "failed", finishedAt: Date.now(), bridgeError: textSummary(message) });
        if (!request.silent) {
          const summary = errorSummary(message);
          if (summary.description) toast.error(`${request.label} 调用失败：${summary.text}`, { description: summary.description });
          else toast.error(`${request.label} 调用失败：${summary.text}`);
        }
        if (request.refresh !== false) void invalidateEngineQueries(queryClient, request.command);
        return null;
      }

      const ok = envelope.ok;
      patch({ status: ok ? "ok" : "failed", finishedAt: Date.now(), envelope: executionSummary(envelope) });
      const firstError: Diagnostic | undefined = envelope.errors[0];
      if (!request.silent) {
        if (ok) {
          // 写入完成不等于助手已经用上：直接把生效条件说清楚
          const hint = effectHint(request.command, request.args ?? []);
          const undo = request.undo;
          toast.success(`${request.label}完成`, {
            ...(hint ? { description: hint } : {}),
            ...(undo ? { action: { label: "撤销", onClick: () => undo() } } : {}),
          });
        } else {
          const summary = errorSummary(firstError?.message ?? "详见任务记录");
          if (summary.description) toast.error(`${request.label} 失败：${summary.text}`, { description: summary.description });
          else toast.error(`${request.label} 失败：${summary.text}`);
        }
      }
      if (request.refresh !== false) void invalidateEngineQueries(queryClient, request.command);
      return envelope;
    },
    [queryClient],
  );

  const value = useMemo(() => ({ executions, execute, batches, recordBatch }), [executions, execute, batches, recordBatch]);
  return <ExecutionContext.Provider value={value}>{children}</ExecutionContext.Provider>;
}

export function useExecutions(): ExecutionContextValue {
  const ctx = useContext(ExecutionContext);
  if (!ctx) {
    throw new Error("useExecutions 必须在 ExecutionProvider 内使用");
  }
  return ctx;
}
