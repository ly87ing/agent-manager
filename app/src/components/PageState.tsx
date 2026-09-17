import { toDiagnostic } from "../lib/engine";
import { ErrorPanel } from "./ErrorPanel";
import { Skeleton } from "./Skeleton";

/** 页面级加载占位：标题条 + 卡片轮廓骨架，与真实卡片配方（rounded-lg border bg-card p-5）对齐。 */
export function PageLoading({ text = "正在读取数据…" }: { text?: string }) {
  return (
    <div role="status" className="space-y-4">
      <span className="sr-only">{text}</span>
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
export function PageError({ title, error }: { title: string; error: unknown }) {
  return <ErrorPanel title={title} errors={[toDiagnostic(error)]} />;
}

export function PageHeader({ title, description }: { title: string; description?: string }) {
  return (
    <div>
      <h2 className="text-xl font-semibold tracking-tight">{title}</h2>
      {description && <p className="mt-1 text-xs text-muted-foreground">{description}</p>}
    </div>
  );
}
