import { useQuery } from "@tanstack/react-query";
import { useEffect, useRef, useState } from "react";
import { CheckCircle2, Loader2, Lock, XCircle } from "lucide-react";
import { ErrorPanel } from "../components/ErrorPanel";
import { PageError, PageHeader, PageLoading } from "../components/PageState";
import type { ApplyResultData, RunEntry, RunsData } from "../lib/contract";
import { runEngine } from "../lib/engine";
import { useExecutions, type ExecutionRecord } from "../lib/execution";
import { engineKeys } from "../lib/queries";
import { cn, formatCheckedAt } from "../lib/utils";

const STEP_STATUS = {
  ok: { icon: CheckCircle2, className: "text-emerald-600" },
  failed: { icon: XCircle, className: "text-red-600" },
} as const;

function ExecutionCard({ record, onSelectLog }: { record: ExecutionRecord; onSelectLog: (runId: string) => void }) {
  const running = record.status === "running";
  const data = record.envelope?.data as Partial<ApplyResultData> | undefined;
  const steps = Array.isArray(data?.steps) ? data.steps : [];
  const lock = record.envelope?.lock;
  const snapshot = data?.snapshot;

  return (
    <section data-testid={`execution-${record.runId}`} className="rounded-lg border border-slate-200 bg-white p-4 shadow-sm">
      <div className="flex flex-wrap items-center gap-2">
        {running ? (
          <Loader2 className="h-4 w-4 animate-spin text-indigo-600" aria-hidden />
        ) : record.status === "ok" ? (
          <CheckCircle2 className="h-4 w-4 text-emerald-600" aria-hidden />
        ) : (
          <XCircle className="h-4 w-4 text-red-600" aria-hidden />
        )}
        <span className="text-sm font-medium text-slate-800">{record.label}</span>
        <code className="font-mono text-xs text-slate-400">{record.command}</code>
        <code className="font-mono text-xs text-indigo-500">{record.runId}</code>
        {record.dryRun && <span className="rounded bg-sky-100 px-1.5 py-0.5 text-xs text-sky-700">dry-run</span>}
        <span
          data-testid={`execution-status-${record.runId}`}
          className={cn(
            "rounded-full px-2 py-0.5 text-xs font-medium",
            running ? "bg-indigo-100 text-indigo-700" : record.status === "ok" ? "bg-emerald-100 text-emerald-700" : "bg-red-100 text-red-700",
          )}
        >
          {running ? "进行中" : record.status === "ok" ? "成功" : "失败"}
        </span>
        <span className="ml-auto flex items-center gap-2 text-xs text-slate-400">
          {lock && (
            <span className="inline-flex items-center gap-1" title="引擎运行锁（envelope.lock）">
              <Lock className="h-3 w-3" aria-hidden />
              {lock.held ? `持锁 pid ${lock.pid ?? "?"}` : "未持锁"}
            </span>
          )}
          {new Date(record.startedAt).toLocaleTimeString("zh-CN", { hour12: false })}
        </span>
        <button
          type="button"
          onClick={() => onSelectLog(record.runId)}
          className="rounded border border-indigo-200 px-2 py-0.5 text-xs text-indigo-600 hover:bg-indigo-50"
        >
          日志
        </button>
      </div>

      {record.bridgeError && <p className="mt-2 text-xs text-red-600">桥错误:{record.bridgeError}</p>}
      {record.envelope && record.envelope.errors.length > 0 && (
        <div className="mt-2">
          <ErrorPanel title="引擎错误" errors={record.envelope.errors} />
        </div>
      )}

      {steps.length > 0 && (
        <ul className="mt-3 space-y-1 border-t border-slate-100 pt-2">
          {steps.map((step) => {
            const meta = step.ok ? STEP_STATUS.ok : STEP_STATUS.failed;
            const Icon = meta.icon;
            return (
              <li key={step.name} className="flex items-center gap-2 text-xs">
                <Icon className={cn("h-3.5 w-3.5", meta.className)} aria-hidden />
                <span className="font-mono text-slate-700">{step.name}</span>
                <span className="text-slate-400">exit {step.exit_code}</span>
              </li>
            );
          })}
        </ul>
      )}
      {snapshot && (
        <p className="mt-2 text-xs text-slate-500">
          快照 <code className="font-mono">{snapshot.run_id}</code>：{snapshot.file_count} 文件 / {snapshot.link_count} 链接
        </p>
      )}
    </section>
  );
}

export default function RunsPage() {
  const { executions } = useExecutions();
  const [selectedRun, setSelectedRun] = useState<string | null>(null);

  const listQuery = useQuery({
    queryKey: engineKeys.command("runs:list"),
    queryFn: () => runEngine<RunsData>("runs:list"),
    refetchInterval: 5000,
  });

  const selectedIsRunning = executions.some((record) => record.runId === selectedRun && record.status === "running");
  const logQuery = useQuery({
    queryKey: [...engineKeys.command("runs:get"), selectedRun],
    queryFn: () => runEngine<RunsData>("runs:get", ["--run", selectedRun!, "--lines", "200"]),
    enabled: selectedRun !== null,
    // 运行中 2s 轮询日志尾；历史运行只拉一次
    refetchInterval: selectedIsRunning ? 2000 : false,
    staleTime: 0,
  });

  const logRef = useRef<HTMLPreElement>(null);
  useEffect(() => {
    // jsdom 无 scrollTo；scrollTop 赋值两端皆可
    const node = logRef.current;
    if (node) node.scrollTop = node.scrollHeight;
  }, [logQuery.data?.data.tail]);

  if (listQuery.isPending) return <PageLoading />;
  if (listQuery.isError) return <PageError title="runs:list 调用失败" error={listQuery.error} />;

  const runs: RunEntry[] = listQuery.data.data.runs;

  return (
    <div className="space-y-4">
      <PageHeader title="执行中心" description="运行历史（5s 轮询）与按次日志。GUI 发起的动作带 gui- 前缀 run_id，运行中 2s 轮询日志尾部。" />

      {executions.length > 0 && (
        <div className="space-y-2">
          <h3 className="text-sm font-semibold text-slate-700">本次会话的执行</h3>
          {executions.map((record) => (
            <ExecutionCard key={record.runId} record={record} onSelectLog={setSelectedRun} />
          ))}
        </div>
      )}

      {selectedRun && (
        <section className="rounded-lg border border-slate-200 bg-white p-4 shadow-sm">
          <div className="mb-2 flex items-center justify-between">
            <h3 className="text-sm font-semibold text-slate-700">
              日志尾部 <code className="font-mono text-xs text-indigo-500">{selectedRun}</code>
              {selectedIsRunning && <span className="ml-2 text-xs text-indigo-500">（运行中，2s 轮询）</span>}
            </h3>
            <button type="button" onClick={() => setSelectedRun(null)} className="text-xs text-slate-400 hover:text-slate-600">
              关闭
            </button>
          </div>
          {logQuery.isPending ? (
            <p className="text-xs text-slate-400">日志加载中…</p>
          ) : logQuery.isError ? (
            <p className="text-xs text-red-600">日志读取失败（该运行可能不写日志，如纯 status 调用）</p>
          ) : (
            <pre
              ref={logRef}
              data-testid="run-log-tail"
              className="max-h-72 overflow-y-auto rounded bg-slate-900 p-3 font-mono text-xs whitespace-pre-wrap break-all text-slate-100"
            >
              {logQuery.data.data.tail || "（日志为空）"}
            </pre>
          )}
        </section>
      )}

      <section className="rounded-lg border border-slate-200 bg-white p-5 shadow-sm">
        <h3 className="mb-3 text-sm font-semibold text-slate-700">运行历史</h3>
        <table className="w-full text-left text-sm">
          <thead>
            <tr className="border-b border-slate-200 text-xs text-slate-500">
              <th className="py-2 pr-4 font-medium">run_id</th>
              <th className="py-2 pr-4 font-medium">时间</th>
              <th className="py-2 pr-4 font-medium">日志大小</th>
              <th className="py-2 font-medium">操作</th>
            </tr>
          </thead>
          <tbody>
            {runs.map((run) => (
              <tr key={run.run_id} data-testid={`run-row-${run.run_id}`} className="border-b border-slate-100 last:border-0">
                <td className="py-2 pr-4 font-mono text-xs text-slate-700">
                  {run.run_id}
                  {run.is_latest && (
                    <span className="ml-2 rounded bg-emerald-100 px-1.5 py-0.5 text-xs text-emerald-700">latest</span>
                  )}
                </td>
                <td className="py-2 pr-4 text-xs text-slate-500">{formatCheckedAt(run.mtime)}</td>
                <td className="py-2 pr-4 text-xs text-slate-500">{(run.size_bytes / 1024).toFixed(1)} KB</td>
                <td className="py-2">
                  <button
                    type="button"
                    onClick={() => setSelectedRun(run.run_id)}
                    className="rounded border border-indigo-200 px-2 py-0.5 text-xs text-indigo-600 hover:bg-indigo-50"
                  >
                    日志
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        {runs.length === 0 && <p className="py-4 text-center text-sm text-slate-400">暂无运行记录</p>}
      </section>
    </div>
  );
}
