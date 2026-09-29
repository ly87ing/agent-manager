import { Check, X } from "lucide-react";
import { useState } from "react";
import { cn } from "../lib/utils";

export interface StartStep {
  id: string;
  label: string;
  hint: string;
  done: boolean;
  action?: { label: string; onClick: () => void };
}

const HIDDEN_KEY = "agent-manager:getting-started-hidden";

function loadHidden(): boolean {
  try {
    return localStorage.getItem(HIDDEN_KEY) === "1";
  } catch {
    return false;
  }
}

/** 「开始使用」：几步做完就不再出现；用户也可以直接关掉（只存本机浏览器）。 */
export function GettingStarted({ steps }: { steps: StartStep[] }) {
  const [hidden, setHidden] = useState(loadHidden);
  const done = steps.filter((step) => step.done).length;
  if (hidden || steps.length === 0 || done === steps.length) return null;
  const hide = () => {
    setHidden(true);
    try {
      localStorage.setItem(HIDDEN_KEY, "1");
    } catch {
      // 存不下只影响下次是否再显示
    }
  };
  return (
    <section data-testid="getting-started" aria-label="开始使用" className="rounded-lg border border-primary/30 bg-primary/5">
      <div className="flex items-center gap-2 px-4 pt-3 text-sm">
        <span className="font-medium">开始使用</span>
        <span className="text-xs text-muted-foreground">
          {done}/{steps.length}
        </span>
        <button type="button" aria-label="不再显示" title="不再显示" data-testid="getting-started-hide" onClick={hide} className="ml-auto rounded p-1 text-muted-foreground hover:bg-accent hover:text-foreground">
          <X className="h-3.5 w-3.5" aria-hidden />
        </button>
      </div>
      <ol className="px-4 pb-3 pt-1">
        {steps.map((step, index) => (
          <li key={step.id} data-testid={`start-step-${step.id}`} data-done={step.done} className="flex items-center gap-3 py-1.5">
            <span
              className={cn(
                "flex h-5 w-5 shrink-0 items-center justify-center rounded-full text-[11px]",
                step.done ? "bg-success text-white" : "border border-primary/40 text-primary",
              )}
            >
              {step.done ? <Check className="h-3 w-3" aria-hidden /> : index + 1}
            </span>
            <span className={cn("text-sm", step.done && "text-muted-foreground line-through")}>{step.label}</span>
            <span className="min-w-0 truncate text-xs text-muted-foreground">{step.hint}</span>
            {!step.done && step.action && (
              <button
                type="button"
                data-testid={`start-action-${step.id}`}
                onClick={step.action.onClick}
                className="ml-auto shrink-0 rounded-md border border-primary/30 bg-primary/10 px-2 py-0.5 text-xs text-primary hover:bg-primary/15"
              >
                {step.action.label}
              </button>
            )}
          </li>
        ))}
      </ol>
    </section>
  );
}
