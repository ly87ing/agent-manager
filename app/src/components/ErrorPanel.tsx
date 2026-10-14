import { t } from "../lib/i18n";
import { AlertOctagon, AlertTriangle } from "lucide-react";
import type { Diagnostic } from "../lib/contract";
import { adviseError } from "../lib/errorAdvice";
import { cn } from "../lib/utils";

/** 渲染 envelope 的 errors / warnings（code + element + message）。 */
export function ErrorPanel({
  title,
  errors,
  variant = "error",
  className,
}: {
  title?: string;
  errors: Diagnostic[];
  variant?: "error" | "warning";
  className?: string;
}) {
  if (errors.length === 0) {
    return null;
  }
  const Icon = variant === "error" ? AlertOctagon : AlertTriangle;
  return (
    <div
      role="alert"
      className={cn(
        "min-w-0 break-words rounded-lg border p-4",
        variant === "error" ? "border-destructive/30 bg-destructive/10" : "border-warning/30 bg-warning/10",
        className,
      )}
    >
      <div className="mb-2 flex items-center gap-2">
        <Icon className={cn("h-4 w-4", variant === "error" ? "text-destructive" : "text-warning")} aria-hidden />
        <h3 className={cn("text-sm font-semibold", variant === "error" ? "text-destructive" : "text-warning")}>
          {title ?? (variant === "warning" ? t("注意事项") : t("执行失败"))}（{errors.length}）
        </h3>
      </div>
      <ul className="space-y-2">
        {errors.map((diag, index) => (
          <li key={`${diag.code}-${index}`} className="min-w-0 break-words text-sm text-foreground">
            {(() => {
              const advice = adviseError(diag.message);
              return advice ? (
                <>
                  <p>{advice.what}</p>
                  <p className="mt-0.5 text-xs text-muted-foreground">{t("怎么办：")}{advice.todo}</p>
                </>
              ) : (
                <p>{diag.message}</p>
              );
            })()}
            <details className="mt-1 text-xs text-muted-foreground">
              <summary className="w-fit cursor-pointer select-none">{t("详细信息")}</summary>
              {adviseError(diag.message) && <p className="mt-1 break-all pl-2">{diag.message}</p>}
              <div className="mt-1 flex flex-wrap gap-2 pl-2">
                <code className="rounded bg-muted px-1.5 py-0.5 font-mono">{diag.code}</code>
                {diag.element && <code className="break-all font-mono">{diag.element}</code>}
              </div>
            </details>
          </li>
        ))}
      </ul>
    </div>
  );
}
