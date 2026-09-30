import { useEffect, useState } from "react";
import { Check, Circle, Loader2, X } from "lucide-react";
import type { EngineProgress } from "../lib/useEngineProgress";
import { cn } from "../lib/utils";

/** 长操作的步骤清单：逐步打勾，当前步骤转圈并写出细节，右上角是已用时间。 */
export function ProgressSteps({ progress, fallback, testId }: { progress: EngineProgress | null; fallback: string; testId?: string }) {
  const [startedAt] = useState(() => Date.now());
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, []);
  const elapsed = Math.max(0, Math.round((now - startedAt) / 1000));
  return (
    <div data-testid={testId} className="space-y-1.5 text-xs">
      <p className="flex items-center justify-between text-muted-foreground">
        <span>{fallback}</span>
        <span data-testid={testId ? `${testId}-elapsed` : undefined} className="tabular-nums">已用 {elapsed} 秒</span>
      </p>
      {progress && (
        <ol className="space-y-1">
          {progress.steps.map((step) => (
            <li key={step.key} data-testid={testId ? `${testId}-${step.key}` : undefined} data-state={step.state} className="flex items-start gap-2">
              {step.state === "done" ? (
                <Check className="mt-0.5 h-3.5 w-3.5 shrink-0 text-primary" aria-hidden />
              ) : step.state === "active" ? (
                <Loader2 className="mt-0.5 h-3.5 w-3.5 shrink-0 animate-spin text-primary" aria-hidden />
              ) : step.state === "failed" ? (
                <X className="mt-0.5 h-3.5 w-3.5 shrink-0 text-destructive" aria-hidden />
              ) : (
                <Circle className="mt-0.5 h-3.5 w-3.5 shrink-0 text-muted-foreground/50" aria-hidden />
              )}
              <span className={cn(step.state === "pending" ? "text-muted-foreground" : "text-foreground")}>
                {step.label}
                {step.detail && step.state === "active" && <span className="ml-1 text-muted-foreground">· {step.detail}</span>}
              </span>
            </li>
          ))}
        </ol>
      )}
    </div>
  );
}
