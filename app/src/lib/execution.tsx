import { createContext, useCallback, useContext, useMemo, useState, type ReactNode } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { engineErrorMessage, runEngine } from "./engine";
import type { Diagnostic, Envelope } from "./contract";
import { engineKeys } from "./queries";
import { newRunId } from "./runId";

/**
 * GUI 发起的写路径执行记录（执行中心数据源）。
 * run_id 由前端生成并透传引擎，按次操作记录据此可追踪（runs:get --run <id>）。
 */
export interface ExecutionRecord {
  runId: string;
  command: string;
  label: string;
  dryRun: boolean;
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
  execute: (request: ExecuteRequest) => Promise<Envelope | null>;
}

export interface ExecuteRequest {
  command: string;
  args?: string[];
  /** 人类可读的动作名，如「收敛 mcp 域」。 */
  label: string;
  dryRun?: boolean;
}

const ExecutionContext = createContext<ExecutionContextValue | null>(null);

export function ExecutionProvider({ children }: { children: ReactNode }) {
  const [executions, setExecutions] = useState<ExecutionRecord[]>([]);
  const queryClient = useQueryClient();

  const execute = useCallback(
    async (request: ExecuteRequest): Promise<Envelope | null> => {
      const runId = newRunId();
      const record: ExecutionRecord = {
        runId,
        command: request.command,
        label: request.label,
        dryRun: request.dryRun ?? false,
        startedAt: Date.now(),
        finishedAt: null,
        status: "running",
        envelope: null,
        bridgeError: null,
      };
      setExecutions((prev) => [record, ...prev]);

      const patch = (update: Partial<ExecutionRecord>) => {
        setExecutions((prev) => prev.map((item) => (item.runId === runId ? { ...item, ...update } : item)));
      };

      let envelope: Envelope | null = null;
      try {
        envelope = await runEngine(request.command, request.args ?? [], { runId });
      } catch (err) {
        const message = engineErrorMessage(err);
        patch({ status: "failed", finishedAt: Date.now(), bridgeError: message });
        toast.error(`${request.label} 调用失败：${message}`);
        void queryClient.invalidateQueries({ queryKey: engineKeys.all });
        return null;
      }

      const ok = envelope.ok;
      patch({ status: ok ? "ok" : "failed", finishedAt: Date.now(), envelope });
      const firstError: Diagnostic | undefined = envelope.errors[0];
      if (ok) {
        toast.success(`${request.label}${request.dryRun ? "（演练）" : ""}完成`);
      } else {
        toast.error(`${request.label} 失败：${firstError?.message ?? "详见任务记录"}`);
      }
      void queryClient.invalidateQueries({ queryKey: engineKeys.all });
      return envelope;
    },
    [queryClient],
  );

  const value = useMemo(() => ({ executions, execute }), [executions, execute]);
  return <ExecutionContext.Provider value={value}>{children}</ExecutionContext.Provider>;
}

export function useExecutions(): ExecutionContextValue {
  const ctx = useContext(ExecutionContext);
  if (!ctx) {
    throw new Error("useExecutions 必须在 ExecutionProvider 内使用");
  }
  return ctx;
}
