import { getLanguage, t } from "../lib/i18n";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { CheckCircle2, History, Loader2, SearchX, Trash2, Undo2, XCircle } from "lucide-react";
import { toast } from "sonner";
import { ConfirmModal } from "../components/ConfirmModal";
import { EmptyState } from "../components/EmptyState";
import { ErrorPanel } from "../components/ErrorPanel";
import { PageError, PageHeader, PageLoading } from "../components/PageState";
import { SearchInput, matchesKeyword } from "../components/SearchInput";
import { Skeleton } from "../components/Skeleton";
import type { ApplyResultData, RunDetailData, RunEntry, RunOperation, RunsClearData, RunsListData, StatusData } from "../lib/contract";
import { rollbackPlan, type RollbackPlan } from "../lib/rollback";
import { engineErrorMessage, runEngine } from "../lib/engine";
import { useExecutions, type ExecutionRecord } from "../lib/execution";
import { engineKeys } from "../lib/queries";
import { cn, formatCheckedAt } from "../lib/utils";
import { BatchResultPanel } from "../components/BatchResultPanel";
import { useBatchApply } from "../lib/useBatchApply";

const STEP_STATUS = {
  ok: { icon: CheckCircle2, className: "text-success" },
  failed: { icon: XCircle, className: "text-destructive" },
} as const;

function ExecutionCard({ record, onSelectRun }: { record: ExecutionRecord; onSelectRun: (runId: string) => void }) {
  const running = record.status === "running";
  const data = record.envelope?.data as Partial<ApplyResultData> | undefined;
  const steps = Array.isArray(data?.steps) ? data.steps : [];

  return (
    <section data-testid={`execution-${record.runId}`} className="rounded-lg border bg-card p-4 shadow-sm">
      <div className="flex flex-wrap items-center gap-2">
        {running ? (
          <Loader2 className="h-4 w-4 animate-spin text-primary" aria-hidden />
        ) : record.status === "ok" ? (
          <CheckCircle2 className="h-4 w-4 text-success" aria-hidden />
        ) : (
          <XCircle className="h-4 w-4 text-destructive" aria-hidden />
        )}
        <span className="text-sm font-medium text-foreground">{record.label}</span>
        <code className="font-mono text-xs text-muted-foreground">{record.command}</code>
        <span
          data-testid={`execution-status-${record.runId}`}
          className={cn(
            "rounded-full px-2 py-0.5 text-xs font-medium",
            running ? "bg-primary/15 text-primary" : record.status === "ok" ? "bg-success/15 text-success" : "bg-destructive/15 text-destructive",
          )}
        >
          {running ? t("正在执行…") : record.status === "ok" ? t("成功") : t("失败")}
        </span>
        <span className="ml-auto text-xs text-muted-foreground">
          {new Date(record.startedAt).toLocaleTimeString(getLanguage(), { hour12: false })}
        </span>
        <button
          type="button"
          onClick={() => onSelectRun(record.runId)}
          className="rounded border border-primary/30 px-2 py-0.5 text-xs text-primary hover:bg-primary/10"
        >{t("详情")}</button>
      </div>

      {record.bridgeError && <p className="mt-2 text-xs text-destructive">{t("无法连接到本机执行服务：")}{record.bridgeError}</p>}
      {record.envelope && record.envelope.errors.length > 0 && (
        <div className="mt-2">
          <ErrorPanel title={t("执行失败")} errors={record.envelope.errors} />
        </div>
      )}

      {steps.length > 0 && (
        <ul className="mt-3 space-y-1 border-t pt-2">
          {steps.map((step) => {
            const meta = step.ok ? STEP_STATUS.ok : STEP_STATUS.failed;
            const Icon = meta.icon;
            return (
              <li key={step.name} className="flex items-center gap-2 text-xs">
                <Icon className={cn("h-3.5 w-3.5", meta.className)} aria-hidden />
                <span className="font-mono text-foreground">{step.name}</span>
                <span className="text-muted-foreground">{(t("退出码") + " ")}{step.exit_code}</span>
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}
function OperationDetail({ data }: { data: RunDetailData }) {
  const { execute } = useExecutions();
  const [pending, setPending] = useState<{ operation: RunOperation; plan: Exclude<RollbackPlan, { kind: "none" }> } | null>(null);
  const [busy, setBusy] = useState(false);

  /** 撤回：技能直接做反向动作；规则先读出文件当前的写入基准，再把备份写回（引擎照常核对与备份）。 */
  const rollback = async () => {
    if (!pending) return;
    setBusy(true);
    try {
      const { plan } = pending;
      if (plan.kind === "skills") {
        await execute({ command: "skills:apply", label: t("撤回：{0}", plan.label), args: ["--only", plan.elementId, "--action", plan.action, "--agent", plan.agent] });
      } else if (plan.kind === "adopt") {
        await execute({ command: "skills:adopt", label: t("撤回：{0}", plan.label), args: ["--undo", plan.backup] });
      } else {
        const status = await runEngine<StatusData>("rules:status", []);
        const element = status.ok ? status.data.elements.find((item) => item.id === plan.target) : undefined;
        const base = (element?.observed as { base?: string | null } | undefined)?.base;
        if (typeof base !== "string") {
          toast.error(t("这个规则文件现在读写不了，没有撤回"));
          return;
        }
        await execute({ command: "rules:apply", label: t("撤回：{0}", plan.label), args: ["--only", plan.target, "--base", base, "--template", plan.template] });
      }
      setPending(null);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="space-y-2">
      {data.operations.map((operation) => {
        const plan = rollbackPlan(operation);
        return (
          <article key={operation.id} className="rounded border p-3">
            <div className="flex flex-wrap items-center gap-3 text-xs">
              <code className="font-mono font-medium text-foreground">{operation.operation_type}</code>
              <span className="text-muted-foreground">{operation.target ?? "—"}</span>
              <time className="text-muted-foreground">{formatCheckedAt(operation.created_at)}</time>
              {plan.kind === "none" ? (
                <span data-testid={`rollback-none-${operation.id}`} className="ml-auto text-muted-foreground" title={plan.reason}>{t("不能自动撤回")}</span>
              ) : (
                <button
                  type="button"
                  data-testid={`rollback-${operation.id}`}
                  onClick={() => setPending({ operation, plan })}
                  className="ml-auto inline-flex items-center gap-1 rounded-md border px-2 py-0.5 text-xs hover:bg-accent"
                >
                  <Undo2 className="h-3.5 w-3.5" aria-hidden />{(" " + t("撤回"))}</button>
              )}
            </div>
            <pre className="mt-2 overflow-x-auto rounded bg-muted p-2 font-mono text-xs text-muted-foreground">
              {JSON.stringify(operation.detail, null, 2)}
            </pre>
          </article>
        );
      })}
      {pending && (
        <ConfirmModal
          title={t("撤回这次操作")}
          description={
            pending.plan.kind === "skills"
              ? t("{0}。只动这一个助手上的这一个技能。", pending.plan.label)
              : pending.plan.kind === "adopt"
                ? t("{0}。原位置被别的内容占用时不动手。", pending.plan.label)
                : t("{0}（用当时另存的备份模板）。写入前核对文件；现在的内容如果对不上任何已知内容，会先另存一份备份。", pending.plan.label)
          }
          risk="write-local"
          confirmLabel={t("撤回")}
          executing={busy}
          onConfirm={() => void rollback()}
          onClose={() => setPending(null)}
        />
      )}
    </div>
  );
}

export default function RunsPage() {
  const batchApply = useBatchApply();
  const queryClient = useQueryClient();
  const { executions, batches } = useExecutions();
  const [selectedRun, setSelectedRun] = useState<string | null>(null);
  const [showClearConfirm, setShowClearConfirm] = useState(false);
  const [isClearing, setIsClearing] = useState(false);
  const [clearStatus, setClearStatus] = useState<string | null>(null);
  const [search, setSearch] = useState("");

  const listQuery = useQuery({
    queryKey: engineKeys.command("runs:list"),
    queryFn: () => runEngine<RunsListData>("runs:list"),
    refetchInterval: executions.some(record => record.status === "running") ? 2000 : false,
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
      if (!res.ok) throw new Error(res.errors[0]?.message ?? t("未能清空任务记录"));
      setSelectedRun(null);
      await queryClient.invalidateQueries({ queryKey: engineKeys.command("runs:list") });
      setShowClearConfirm(false);
      const count = res.data?.deleted ?? 0;
      const msg = t("任务记录已清空（共清理 {0} 条记录）", count);
      setClearStatus(msg);
      toast.success(msg);
    } catch (err) {
      const msg = t("清空任务记录失败：{0}", engineErrorMessage(err));
      setClearStatus(msg);
      toast.error(msg);
    } finally {
      setIsClearing(false);
    }
  };

  if (listQuery.isPending) return <PageLoading />;
  if (listQuery.isError) return <PageError title={t("任务记录读取失败")} error={listQuery.error} onRetry={() => void listQuery.refetch()} retrying={listQuery.isFetching} />;
  if (!listQuery.data.ok) return <PageError title={t("任务记录读取失败")} errors={listQuery.data.errors} onRetry={() => void listQuery.refetch()} retrying={listQuery.isFetching} />;

  const runs: RunEntry[] = listQuery.data.data.runs;
  const currentRunIds = new Set(executions.map((record) => record.runId));
  const filteredRuns = runs.filter((run) =>
    !currentRunIds.has(run.run_id) && matchesKeyword(search, [run.run_id]),
  );
  const filteredExecutions = executions.filter((record) =>
    matchesKeyword(search, [record.label, record.command, record.runId]),
  );

  return (
    <div className="space-y-4">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <PageHeader title={t("任务记录")} description={t("查看本工具执行过的同步、应用与配置操作。")} />
        <div className="flex items-center gap-2">
          <SearchInput value={search} onChange={setSearch} placeholder={t("搜索任务记录…")} testId="runs-search" />
          <button
            type="button"
            data-testid="clear-runs-button"
            onClick={() => setShowClearConfirm(true)}
            disabled={isClearing}
            className="inline-flex items-center gap-1.5 rounded-md border bg-card px-3 py-1.5 text-xs font-medium text-foreground hover:bg-accent focus:outline-none focus:ring-2 focus:ring-ring focus:ring-offset-2 disabled:opacity-50"
          >
            <Trash2 className="h-3.5 w-3.5 text-muted-foreground" aria-hidden />{t("清空任务记录")}</button>
        </div>
      </div>

      {clearStatus && (
        <p
          role={clearStatus.startsWith(t("清空任务记录失败")) ? "alert" : "status"}
          className={cn("text-xs", clearStatus.startsWith(t("清空任务记录失败")) ? "text-destructive" : "text-success")}
        >
          {clearStatus}
        </p>
      )}

      {filteredExecutions.length > 0 && (
        <div className="space-y-2">
          <h3 className="text-sm font-semibold text-foreground">{t("本次会话的任务")}</h3>
          {filteredExecutions.map((record) => (
            <ExecutionCard key={record.runId} record={record} onSelectRun={setSelectedRun} />
          ))}
        </div>
      )}

      {batches.filter((batch) => matchesKeyword(search, [batch.label, ...batch.rows.map((item) => item.row.label)])).map((batch) =>
        <BatchResultPanel key={batch.id} batch={batch} busy={batchApply.executing || batches.some((item) => item.running)} onRetry={() => void batchApply.retryFailed(batch)} />)}

      {selectedRun && (
        <section className="rounded-lg border bg-card p-4 shadow-sm">
          <div className="mb-3 flex items-center justify-between">
            <h3 className="text-sm font-semibold text-foreground">{(t("操作详情") + " ")}<code className="font-mono text-xs text-primary">{selectedRun}</code>
            </h3>
            <button type="button" onClick={() => setSelectedRun(null)} className="text-xs text-muted-foreground hover:text-muted-foreground">{t("关闭")}</button>
          </div>
          {detailQuery.isPending ? (
            <div role="status" className="space-y-2">
              <span className="sr-only">{t("详情加载中…")}</span>
              <Skeleton className="h-14 w-full" />
              <Skeleton className="h-14 w-full" />
            </div>
          ) : detailQuery.isError ? (
            <p className="text-xs text-destructive">{t("操作详情读取失败")}</p>
          ) : (
            <OperationDetail data={detailQuery.data.data} />
          )}
        </section>
      )}

      <section className="rounded-lg border bg-card p-5 shadow-sm">
        <h3 className="mb-3 text-sm font-semibold text-foreground">{t("其他历史任务")}</h3>
        <table className="w-full text-left text-sm">
          <thead>
            <tr className="border-b text-xs text-muted-foreground">
              <th className="py-2 pr-4 font-medium">{t("运行 ID")}</th>
              <th className="py-2 pr-4 font-medium">{t("操作数")}</th>
              <th className="py-2 pr-4 font-medium">{t("最后时间")}</th>
              <th className="py-2 font-medium">{t("操作")}</th>
            </tr>
          </thead>
          <tbody>
            {filteredRuns.map((run) => (
              <tr key={run.run_id} data-testid={`run-row-${run.run_id}`} className="border-b last:border-0">
                <td className="py-2 pr-4 font-mono text-xs text-foreground">{run.run_id}</td>
                <td className="py-2 pr-4 text-xs text-muted-foreground">{run.operations}</td>
                <td className="py-2 pr-4 text-xs text-muted-foreground">{formatCheckedAt(run.last_at)}</td>
                <td className="py-2">
                  <button type="button" onClick={() => setSelectedRun(run.run_id)} className="rounded border border-primary/30 px-2 py-0.5 text-xs text-primary hover:bg-primary/10">{t("详情")}</button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        {filteredRuns.length === 0 &&
          (search.trim() ? (
            <EmptyState
              compact
              testId="runs-search-empty"
              icon={SearchX}
              title={t("无匹配结果")}
              description={t("没有运行 ID 包含该关键词的任务记录。")}
              actions={
                <button
                  type="button"
                  onClick={() => setSearch("")}
                  className="inline-flex items-center gap-1 rounded border px-2 py-1 text-xs text-muted-foreground hover:bg-accent"
                >{t("清除搜索关键词")}</button>
              }
            />
          ) : (
            <EmptyState
              compact
              icon={History}
              title={executions.length > 0 ? t("没有其他历史任务") : t("还没有任务记录")}
              description={executions.length > 0
                ? t("本次会话的任务已显示在上方。")
                : t("通过本工具执行的安装、更新与分配操作会记录在这里。")}
            />
          ))}
      </section>

      {showClearConfirm && (
        <ConfirmModal
          title={t("清空任务记录")}
          description={t("将清空本地所有历史任务与操作记录，此操作不可恢复。助手定义、个性化配置及数据结构不会受到影响。")}
          risk="write-local"
          riskLabel={t("清理本地记录")}
          confirmLabel={t("确认清空")}
          executing={isClearing}
          onConfirm={() => void handleClear()}
          onClose={() => setShowClearConfirm(false)}
        />
      )}
    </div>
  );
}
