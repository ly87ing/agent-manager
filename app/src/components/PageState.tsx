import { t } from "../lib/i18n";
import { toDiagnostic } from "../lib/engine";
import type { Diagnostic } from "../lib/contract";
import { ErrorPanel } from "./ErrorPanel";
import { Skeleton } from "./Skeleton";

/** 页面级加载占位：标题条 + 卡片轮廓骨架，与真实卡片配方（rounded-lg border bg-card p-5）对齐。 */
export function PageLoading({ text = t("正在读取数据…") }: { text?: string }) {
  return (
    <div role="status" className="space-y-4">
      <span className="text-xs text-muted-foreground">{text}</span>
      <Skeleton className="h-7 w-48" />
      <div className="grid grid-cols-1 gap-4 xl:grid-cols-2">
        {[0, 1, 2].map((index) => (
          <div key={index} className="space-y-3 rounded-lg border bg-card p-5 shadow-sm">
            <Skeleton className="h-4 w-1/3" />
            <Skeleton className="h-3 w-2/3" />
            <Skeleton className="h-3 w-1/2" />
          </div>
        ))}
      </div>
    </div>
  );
}

/** 页面级桥错误（非 envelope 响应）。 */
export function PageError({ title, error, errors, onRetry, retrying = false, className }: {
  title: string;
  error?: unknown;
  errors?: Diagnostic[];
  onRetry: () => void;
  retrying?: boolean;
  className?: string;
}) {
  return (
    <div className="space-y-2">
      <ErrorPanel title={title} errors={errors ?? [toDiagnostic(error)]} className={className} />
      <button type="button" disabled={retrying} onClick={onRetry} className="rounded border px-3 py-1.5 text-sm hover:bg-accent disabled:opacity-40">{retrying ? t("正在读取数据…") : t("重新读取")}</button>
    </div>
  );
}

export function PageHeader({ title, description }: { title: string; description?: string }) {
  return (
    <div>
      <h2 className="text-xl font-semibold tracking-tight">{title}</h2>
      {description && <p className="mt-1 text-xs text-muted-foreground">{description}</p>}
    </div>
  );
}

export function ReadAgain({ queries, disabled = false }: {
  queries: Array<{ isFetching: boolean; dataUpdatedAt: number; refetch: () => Promise<unknown> }>;
  disabled?: boolean;
}) {
  const reading = queries.some((query) => query.isFetching);
  const updatedAt = Math.min(...queries.map((query) => query.dataUpdatedAt));
  return <div className="flex flex-wrap items-center gap-2 text-xs">
    {updatedAt > 0 && <span className="text-muted-foreground">{t("读取时间：{0}", new Date(updatedAt).toLocaleTimeString())}</span>}
    <button type="button" aria-label={t("重新读取页面")} disabled={reading || disabled} onClick={() => void Promise.all(queries.map((query) => query.refetch()))}
      className="rounded border px-3 py-1.5 hover:bg-accent disabled:opacity-40">{reading ? t("正在读取数据…") : t("重新读取")}</button>
  </div>;
}
