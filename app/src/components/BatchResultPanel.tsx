import { batchCounts, type BatchResult } from "../lib/batchResult";

export function BatchResultPanel({ batch, onRetry, busy }: { batch: BatchResult; onRetry: () => void; busy: boolean }) {
  const counts = batchCounts(batch);
  return <section className="rounded-lg border bg-card p-3 text-xs" aria-label={`批量${batch.label}结果`}>
    <div className="flex flex-wrap items-center gap-2">
      <strong>批量{batch.label}{batch.running ? "进行中" : "结果"}</strong>
      <span>成功 {counts.succeeded} · 失败 {counts.failed}{counts.pending ? ` · 待处理 ${counts.pending}` : ""}</span>
      {counts.failed > 0 && <button type="button" disabled={busy || batch.running} onClick={onRetry}
        className="ml-auto rounded border px-2.5 py-1 hover:bg-accent disabled:opacity-50">重新检查并重试失败项</button>}
    </div>
    <details className="mt-2" open={counts.failed > 0}>
      <summary className="cursor-pointer text-muted-foreground">查看 {batch.rows.length} 个目标的结果</summary>
      <ul className="mt-2 max-h-60 space-y-1 overflow-y-auto">
        {batch.rows.map((item) => <li key={`${item.row.elementId}@${item.row.agent}`} className="break-words [overflow-wrap:anywhere]">
          <span className={item.status === "failed" ? "text-destructive" : "text-muted-foreground"}>{item.status === "ok" ? "成功" : item.status === "failed" ? "失败" : "待处理"}</span>
          {" · "}{item.row.label}{item.message ? `：${item.message}` : ""}
        </li>)}
      </ul>
    </details>
  </section>;
}
