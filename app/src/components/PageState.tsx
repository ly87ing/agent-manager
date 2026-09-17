import { Loader2 } from "lucide-react";
import { toDiagnostic } from "../lib/engine";
import { ErrorPanel } from "./ErrorPanel";

/** 页面级加载占位。 */
export function PageLoading({ text = "正在读取数据…" }: { text?: string }) {
  return (
    <div className="flex items-center gap-2 py-20 text-sm text-muted-foreground">
      <Loader2 className="h-4 w-4 animate-spin" aria-hidden />
      {text}
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
