import { t } from "../lib/i18n";
import { batchCounts, type BatchResult } from "../lib/batchResult";

export function BatchResultPanel({ batch, onRetry, busy }: { batch: BatchResult; onRetry: () => void; busy: boolean }) {
  const counts = batchCounts(batch);
  return <section className="rounded-lg border bg-card p-3 text-xs" aria-label={t("批量{0}结果", batch.label)}>
    <div className="flex flex-wrap items-center gap-2">
      <strong>{t("批量")}{batch.label}{batch.running ? t("进行中") : t("结果")}</strong>
      <span>{(t("成功") + " ")}{counts.succeeded}{(" " + t("· 失败") + " ")}{counts.failed}{counts.pending ? (" " + t("· 待处理 {0}", counts.pending)) : ""}</span>
      {counts.failed > 0 && <button type="button" disabled={busy || batch.running} onClick={onRetry}
        className="ml-auto rounded border px-2.5 py-1 hover:bg-accent disabled:opacity-50">{t("重新检查并重试失败项")}</button>}
    </div>
    <details className="mt-2" open={counts.failed > 0}>
      <summary className="cursor-pointer text-muted-foreground">{(t("查看") + " ")}{batch.rows.length}{(" " + t("个目标的结果"))}</summary>
      <ul className="mt-2 max-h-60 space-y-1 overflow-y-auto">
        {batch.rows.map((item) => <li key={`${item.row.elementId}@${item.row.agent}`} className="break-words [overflow-wrap:anywhere]">
          <span className={item.status === "failed" ? "text-destructive" : "text-muted-foreground"}>{item.status === "ok" ? t("成功") : item.status === "failed" ? t("失败") : t("待处理")}</span>
          {" · "}{item.row.label}{item.message ? `：${item.message}` : ""}
        </li>)}
      </ul>
    </details>
  </section>;
}
