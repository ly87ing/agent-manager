import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { CheckCircle2, Loader2, Lock, Trash2, XCircle } from "lucide-react";
import { toast } from "sonner";
import { ConfirmModal } from "../components/ConfirmModal";
import { ErrorPanel } from "../components/ErrorPanel";
import { PageError, PageHeader, PageLoading } from "../components/PageState";
import type { ApplyResultData, RunDetailData, RunEntry, RunsClearData, RunsListData } from "../lib/contract";
import { engineErrorMessage, runEngine } from "../lib/engine";
import { useExecutions, type ExecutionRecord } from "../lib/execution";
import { engineKeys } from "../lib/queries";
import { cn, formatCheckedAt } from "../lib/utils";

const STEP_STATUS = {
  ok: { icon: CheckCircle2, className: "text-emerald-600" },
  failed: { icon: XCircle, className: "text-red-600" },
} as const;

function ExecutionCard({ record, onSelectRun }: { record: ExecutionRecord; onSelectRun: (runId: string) => void }) {
  const running = record.status === "running";
  const data = record.envelope?.data as Partial<ApplyResultData> | undefined;
  const steps = Array.isArray(data?.steps) ? data.steps : [];
  const lock = record.envelope?.lock;

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
        <span
          data-testid={`execution-status-${record.runId}`}
          className={cn(
            "rounded-full px-2 py-0.5 text-xs font-medium",
            running ? "bg-indigo-100 text-indigo-700" : record.status === "ok" ? "bg-emerald-100 text-emerald-700" : "bg-red-100 text-red-700",
          )}
        >
          {running ? "正在执行…" : record.status === "ok" ? "成功" : "失败"}
        </span>
        <span className="ml-auto flex items-center gap-2 text-xs text-slate-400">
          {lock && (
            <span className="inline-flex items-center gap-1" title="任务互斥：运行期间另一个任务无法同时执行（引擎锁 envelope.lock）">
              <Lock className="h-3 w-3" aria-hidden />
              {lock.held ? `独占执行 · pid ${lock.pid ?? "?"}` : "只读任务"}
            </span>
          )}
          {new Date(record.startedAt).toLocaleTimeString("zh-CN", { hour12: false })}
        </span>
        <button
          type="button"
          onClick={() => onSelectRun(record.runId)}
          className="rounded border border-indigo-200 px-2 py-0.5 text-xs text-indigo-600 hover:bg-indigo-50"
        >
          详情
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
    </section>
  );
}
function OperationDetail({ data }: { data: RunDetailData }) {
  return (
    <div className="space-y-2">
      {data.operations.map((operation) => (
        <article key={operation.id} className="rounded border border-slate-200 p-3">
          <div className="flex flex-wrap items-center gap-3 text-xs">
            <code className="font-mono font-medium text-slate-700">{operation.operation_type}</code>
            <span className="text-slate-600">{operation.target ?? "—"}</span>
            <time className="text-slate-400">{formatCheckedAt(operation.created_at)}</time>
          </div>
          <pre className="mt-2 overflow-x-auto rounded bg-slate-50 p-2 font-mono text-xs text-slate-600">
            {JSON.stringify(operation.detail, null, 2)}
          </pre>
        </article>
      ))}
    </div>
  );
}

export default function RunsPage() {
  const queryClient = useQueryClient();
  const { executions } = useExecutions();
  const [selectedRun, setSelectedRun] = useState<string | null>(null);
  const [showClearConfirm, setShowClearConfirm] = useState(false);
  const [isClearing, setIsClearing] = useState(false);
  const [clearStatus, setClearStatus] = useState<string | null>(null);

  const listQuery = useQuery({
    queryKey: engineKeys.command("runs:list"),
    queryFn: () => runEngine<RunsListData>("runs:list"),
    refetchInterval: 5000,
  });

  const detailQuery = useQuery({
    queryKey: [...engineKeys.command("runs:get"), selectedRun],
    queryFn: () => runEngine<RunDetailData>("runs:get", ["--run", selectedRun!]),
    enabled: selectedRun !== null,
    refetchInterval: executions.some((record) => record.runId === selectedRun && record.status === "running") ? 2000 : false,
    staleTime: 0,
  });

  const handleClear = async () => {
    setIsClearing(true);
    setClearStatus(null);
    try {
      const res = await runEngine<RunsClearData>("runs:clear");
      setSelectedRun(null);
      await queryClient.invalidateQueries({ queryKey: engineKeys.command("runs:list") });
      setShowClearConfirm(false);
      const count = res.data?.deleted ?? 0;
      const msg = `任务记录已清空（共清理 ${count} 条记录）`;
      setClearStatus(msg);
      toast.success(msg);
    } catch (err) {
      const msg = `清空任务记录失败：${engineErrorMessage(err)}`;
      setClearStatus(msg);
      toast.error(msg);
    } finally {
      setIsClearing(false);
    }
  };

  if (listQuery.isPending) return <PageLoading />;
  if (listQuery.isError) return <PageError title="runs:list 调用失败" error={listQuery.error} />;

  const runs: RunEntry[] = listQuery.data.data.runs;

  return (
    <div className="space-y-4">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <PageHeader title="任务记录" description="查看本工具通过 native-manager.db 记录的运行与操作详情。" />
        <button
          type="button"
          data-testid="clear-runs-button"
          onClick={() => setShowClearConfirm(true)}
          disabled={isClearing}
          className="inline-flex items-center gap-1.5 rounded-md border border-slate-200 bg-white px-3 py-1.5 text-xs font-medium text-slate-700 hover:bg-slate-50 focus:outline-none focus:ring-2 focus:ring-indigo-500 focus:ring-offset-2 disabled:opacity-50"
        >
          <Trash2 className="h-3.5 w-3.5 text-slate-500" aria-hidden />
          清空任务记录
        </button>
      </div>

      {clearStatus && (
        <p
          role={clearStatus.startsWith("清空任务记录失败") ? "alert" : "status"}
          className={cn("text-xs", clearStatus.startsWith("清空任务记录失败") ? "text-red-600" : "text-emerald-700")}
        >
          {clearStatus}
        </p>
      )}

      {executions.length > 0 && (
        <div className="space-y-2">
          <h3 className="text-sm font-semibold text-slate-700">本次会话的任务</h3>
          {executions.map((record) => (
            <ExecutionCard key={record.runId} record={record} onSelectRun={setSelectedRun} />
          ))}
        </div>
      )}

      {selectedRun && (
        <section className="rounded-lg border border-slate-200 bg-white p-4 shadow-sm">
          <div className="mb-3 flex items-center justify-between">
            <h3 className="text-sm font-semibold text-slate-700">
              操作详情 <code className="font-mono text-xs text-indigo-500">{selectedRun}</code>
            </h3>
            <button type="button" onClick={() => setSelectedRun(null)} className="text-xs text-slate-400 hover:text-slate-600">
              关闭
            </button>
          </div>
          {detailQuery.isPending ? (
            <p className="text-xs text-slate-400">详情加载中…</p>
          ) : detailQuery.isError ? (
            <p className="text-xs text-red-600">操作详情读取失败</p>
          ) : (
            <OperationDetail data={detailQuery.data.data} />
          )}
        </section>
      )}

      <section className="rounded-lg border border-slate-200 bg-white p-5 shadow-sm">
        <h3 className="mb-3 text-sm font-semibold text-slate-700">历史任务</h3>
        <table className="w-full text-left text-sm">
          <thead>
            <tr className="border-b border-slate-200 text-xs text-slate-500">
              <th className="py-2 pr-4 font-medium">运行 ID</th>
              <th className="py-2 pr-4 font-medium">操作数</th>
              <th className="py-2 pr-4 font-medium">最后时间</th>
              <th className="py-2 font-medium">操作</th>
            </tr>
          </thead>
          <tbody>
            {runs.map((run) => (
              <tr key={run.run_id} data-testid={`run-row-${run.run_id}`} className="border-b border-slate-100 last:border-0">
                <td className="py-2 pr-4 font-mono text-xs text-slate-700">{run.run_id}</td>
                <td className="py-2 pr-4 text-xs text-slate-500">{run.operations}</td>
                <td className="py-2 pr-4 text-xs text-slate-500">{formatCheckedAt(run.last_at)}</td>
                <td className="py-2">
                  <button type="button" onClick={() => setSelectedRun(run.run_id)} className="rounded border border-indigo-200 px-2 py-0.5 text-xs text-indigo-600 hover:bg-indigo-50">
                    详情
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        {runs.length === 0 && <p className="py-4 text-center text-sm text-slate-400">还没有任务记录</p>}
      </section>

      {showClearConfirm && (
        <ConfirmModal
          title="清空任务记录"
          description="将清空本地所有历史任务与操作记录，此操作不可恢复。助手定义、个性化配置及数据结构不会受到影响。"
          risk="write-local"
          riskLabel="清理本地记录"
          confirmLabel="确认清空"
          executing={isClearing}
          onConfirm={() => void handleClear()}
          onClose={() => setShowClearConfirm(false)}
        />
      )}
    </div>
  );
}
