import { AlertOctagon, AlertTriangle } from "lucide-react";
import type { Diagnostic } from "../lib/contract";
import { cn } from "../lib/utils";

/** 渲染 envelope 的 errors / warnings（code + element + message）。 */
export function ErrorPanel({
  title = "引擎返回诊断",
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
        "rounded-lg border p-4",
        variant === "error" ? "border-destructive/30 bg-destructive/10" : "border-warning/30 bg-warning/10",
        className,
      )}
    >
      <div className="mb-2 flex items-center gap-2">
        <Icon className={cn("h-4 w-4", variant === "error" ? "text-destructive" : "text-warning")} aria-hidden />
        <h3 className={cn("text-sm font-semibold", variant === "error" ? "text-destructive" : "text-warning")}>
          {title}（{errors.length}）
        </h3>
      </div>
      <ul className="space-y-1.5">
        {errors.map((diag, index) => (
          <li key={`${diag.code}-${index}`} className="text-sm">
            <code
              className={cn(
                "mr-2 rounded px-1.5 py-0.5 font-mono text-xs",
                variant === "error" ? "bg-destructive/15 text-destructive" : "bg-warning/15 text-warning",
              )}
            >
              {diag.code}
            </code>
            {diag.element && <span className="mr-2 font-mono text-xs text-muted-foreground">{diag.element}</span>}
            <span className="text-foreground">{diag.message}</span>
          </li>
        ))}
      </ul>
    </div>
  );
}
