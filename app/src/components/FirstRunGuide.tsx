import { ArrowRight, PackageCheck } from "lucide-react";
import { useState } from "react";

/**
 * 首次使用引导：本机还没有任何自己保存过的配置时，在默认落地页上说清下一步该做什么。
 *
 * 出现条件取自 policy:get 的本地层——接管、保存定义、改偏好都会写进这一层，
 * 所以这一层为空就等于「还没用这个工具做过任何事」。用户点「知道了」后不再出现
 * （只记在浏览器本地，读写失败就退化为本次会话内不再提示）。
 */

const DISMISS_KEY = "agent-manager:first-run:dismissed";

function readDismissed(): boolean {
  try {
    return window.localStorage.getItem(DISMISS_KEY) === "1";
  } catch {
    return false;
  }
}

function persistDismissed(): void {
  try {
    window.localStorage.setItem(DISMISS_KEY, "1");
  } catch {
    // localStorage 不可用（隐私模式等）时忽略
  }
}

const STEPS: { title: string; detail: string }[] = [
  {
    title: "先扫描本机已有的东西",
    detail: "「接管中心」会列出已装好的助手、技能和 MCP 服务。官方安装的那些只做标记，继续用它们原来的方式升级卸载。",
  },
  {
    title: "把资源分给需要的助手",
    detail: "在「技能」「MCP」页勾选谁用哪些。这一步只保存你的选择，不会动任何助手的配置文件。",
  },
  {
    title: "点「立即同步」才真正写入",
    detail: "写之前会先告诉你要改哪些文件、改成什么样；发现文件被别处改过会停下来问你，不会闷头覆盖。",
  },
];

export function FirstRunGuide({ show, onGoTakeover }: { show: boolean; onGoTakeover?: () => void }) {
  const [dismissed, setDismissed] = useState(readDismissed);

  if (!show || dismissed) return null;

  const dismiss = () => {
    persistDismissed();
    setDismissed(true);
  };

  return (
    <section data-testid="first-run-guide" className="rounded-lg border border-primary/30 bg-primary/5 p-5">
      <h3 className="text-sm font-semibold text-foreground">三步开始用</h3>
      <p className="mt-1 text-xs text-muted-foreground">
        本机还没有你保存过的配置。下面三步走完，本机已有的助手、技能和 MCP 就都在这里统一管了。
      </p>
      <ol className="mt-3 space-y-2">
        {STEPS.map((step, index) => (
          <li key={step.title} className="flex gap-2.5">
            <span className="mt-0.5 flex h-5 w-5 shrink-0 items-center justify-center rounded-full bg-primary/15 text-[11px] font-semibold text-primary">
              {index + 1}
            </span>
            <div className="min-w-0">
              <p className="text-xs font-medium text-foreground">{step.title}</p>
              <p className="mt-0.5 text-[11px] leading-relaxed text-muted-foreground">{step.detail}</p>
            </div>
          </li>
        ))}
      </ol>
      <div className="mt-4 flex items-center gap-2">
        {onGoTakeover && (
          <button
            type="button"
            data-testid="first-run-goto-takeover"
            onClick={onGoTakeover}
            className="inline-flex items-center gap-1.5 rounded-md border border-primary/30 bg-primary/10 px-3 py-1.5 text-xs font-medium text-primary hover:bg-primary/15"
          >
            <PackageCheck className="h-3.5 w-3.5" aria-hidden /> 去接管中心
            <ArrowRight className="h-3 w-3" aria-hidden />
          </button>
        )}
        <button
          type="button"
          data-testid="first-run-dismiss"
          onClick={dismiss}
          className="rounded-md border px-3 py-1.5 text-xs text-muted-foreground hover:bg-accent"
        >
          知道了，不再提示
        </button>
      </div>
    </section>
  );
}
