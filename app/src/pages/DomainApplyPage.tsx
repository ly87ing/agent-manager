import { PlayCircle } from "lucide-react";
import { SourceDefinitionsPanel } from "../components/SourceDefinitionsPanel";
import { ElementTable } from "../components/ElementTable";
import { Switch } from "../components/Switch";
import { ErrorPanel } from "../components/ErrorPanel";
import { PageError, PageHeader, PageLoading } from "../components/PageState";
import type { EngineElement, StatusData } from "../lib/contract";
import { useEngineQuery } from "../lib/queries";
import { useDirectApply } from "../lib/useDirectApply";
import { useElementToggle } from "../lib/useElementToggle";

/** Hooks 阻塞/非受管目标：仅在引擎给出了可安全接管的 actual_digest（string）时才显示显式接管入口。
 * actual_digest=null 表示无效 JSON、路径/类型不安全等情形，前端不得提供接管按钮。 */
function isBlockedHooksElement(element: EngineElement): boolean {
  const observed = element.observed as { blocked_reason?: unknown; actual_digest?: unknown };
  const hasDigest = typeof observed.actual_digest === "string";
  if (!hasDigest) return false;
  if (element.state === "unmanaged-detected") return true;
  return observed.blocked_reason != null;
}

/**
 * 简单域的只读 + 同步页（Aliases / Hooks）：元素表 + 「立即同步」按钮，
 * 所有写操作点击直接 apply。
 */
export default function DomainApplyPage({
  domain,
  title,
  description,
  enableToggles = false,
}: {
  domain: string;
  title: string;
  description: string;
  /** true 时每行渲染启停开关（policy key <domain>.<元素短名>.enabled，写默认策略层）。 */
  enableToggles?: boolean;
}) {
  const query = useEngineQuery<StatusData>(`${domain}:status`);
  const directApply = useDirectApply();
  const { toggle, executing: toggleExecuting } = useElementToggle();
  const disabled = directApply.executing || toggleExecuting;

  if (query.isPending) return <PageLoading />;
  if (query.isError) return <PageError title={`${domain}:status 调用失败`} error={query.error} />;
  if (!query.data.ok) {
    return (
      <div className="space-y-4">
        <PageHeader title={title} description={description} />
        <ErrorPanel title={`${domain}:status 错误`} errors={query.data.errors} />
      </div>
    );
  }

  const elements = query.data.data.elements;

  /** Hooks 阻塞目标的「接管原内容」入口：一键直接带 --allow-takeover 执行 apply。 */
  const hooksTakeoverAction = (element: EngineElement) => {
    if (!isBlockedHooksElement(element)) return null;
    const wholeFile = (element.spec as { merge?: string }).merge === "whole-file";
    return (
      <div className="flex max-w-xs flex-col items-end gap-1 text-right">
        <button
          type="button"
          data-testid={`takeover-${element.id}`}
          disabled={disabled}
          onClick={() =>
            void directApply.apply({
              command: "hooks:apply",
              label: `接管原内容 ${element.id}`,
              args: ["--only", element.id, "--allow-takeover"],
            })
          }
          className="inline-flex items-center gap-1 rounded border border-amber-300 bg-amber-50 px-2.5 py-1 text-xs font-medium text-amber-700 hover:bg-amber-100 disabled:opacity-40"
        >
          接管原内容
        </button>
        {wholeFile && (
          <p data-testid={`takeover-hint-${element.id}`} className="text-[11px] leading-snug text-amber-700">
            整文件替换：现有内容将被整体替换，先把需要保留的 hook 并入来源定义。
          </p>
        )}
      </div>
    );
  };

  return (
    <div className="space-y-4">
      <PageHeader title={title} description={description} />
      {query.data.errors.length > 0 && <ErrorPanel title={`${domain}:status 错误`} errors={query.data.errors} />}
      {(domain === "hooks" || domain === "aliases") && <SourceDefinitionsPanel domain={domain} />}

      <div className="flex justify-end gap-2">
        <button
          type="button"
          data-testid={`apply-${domain}-button`}
          disabled={disabled}
          onClick={() => void directApply.apply({ command: `${domain}:apply`, label: `同步${title}` })}
          className="inline-flex items-center gap-2 rounded-md bg-indigo-600 px-4 py-2 text-sm font-medium text-white hover:bg-indigo-700 disabled:opacity-60"
        >
          <PlayCircle className="h-4 w-4" aria-hidden /> 立即同步
        </button>
      </div>

      <section className="rounded-lg border border-slate-200 bg-white p-5 shadow-sm">
        <ElementTable
          elements={elements}
          renderActions={
            domain === "hooks"
              ? hooksTakeoverAction
              : enableToggles
                ? (element) => {
                    const shortName = element.id.replace(`${domain}:`, "");
                    const enabled = (element.policy as { enabled?: boolean }).enabled !== false;
                    return (
                      <Switch
                        checked={enabled}
                        testId={`toggle-${element.id}`}
                        label={`${enabled ? "停用" : "启用"} ${shortName}`}
                        disabled={disabled}
                        onChange={() =>
                          void toggle({
                            elementId: element.id,
                            enabledKey: `${domain}.${shortName}.enabled`,
                            enabled,
                            domain,
                            name: shortName,
                          })
                        }
                      />
                    );
                  }
                : undefined
          }
        />
      </section>

    </div>
  );
}
