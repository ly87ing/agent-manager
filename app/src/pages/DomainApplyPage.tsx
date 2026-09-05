import { PlayCircle } from "lucide-react";
import { ElementTable } from "../components/ElementTable";
import { ErrorPanel } from "../components/ErrorPanel";
import { PageError, PageHeader, PageLoading } from "../components/PageState";
import type { StatusData } from "../lib/contract";
import { useEngineQuery } from "../lib/queries";
import { useExecuteApply, usePlanPreview } from "../lib/usePlanPreview";

/**
 * 简单域的只读 + 收敛页（Tools / Aliases / Hooks）：元素表 + 「收敛此域」按钮
 * （plan 预览确认 → 域级 apply）。Tools 的安装/升级属 install-remove 级，
 * 预览含此类动作时自动进入二次确认。
 */
export default function DomainApplyPage({
  domain,
  title,
  description,
}: {
  domain: string;
  title: string;
  description: string;
}) {
  const query = useEngineQuery<StatusData>(`${domain}:status`);
  const preview = usePlanPreview();
  const executeApply = useExecuteApply();

  if (query.isPending) return <PageLoading />;
  if (query.isError) return <PageError title={`${domain}:status 调用失败`} error={query.error} />;

  const elements = query.data.data.elements;

  return (
    <div className="space-y-4">
      <PageHeader title={title} description={description} />
      {query.data.errors.length > 0 && <ErrorPanel title={`${domain}:status 错误`} errors={query.data.errors} />}

      <div className="flex justify-end">
        <button
          type="button"
          data-testid={`apply-${domain}-button`}
          onClick={() =>
            preview.open({
              title: `收敛 ${title} 域`,
              subtitle: `执行 ${domain}:apply；预览为 ${domain}:plan 的完整动作清单。非 dry-run 前引擎自动快照。`,
              planCommands: [`${domain}:plan`],
              confirmLabel: `收敛 ${title}`,
              execute: executeApply({ command: `${domain}:apply`, label: `收敛 ${title} 域` }),
            })
          }
          className="inline-flex items-center gap-2 rounded-md bg-indigo-600 px-4 py-2 text-sm font-medium text-white hover:bg-indigo-700"
        >
          <PlayCircle className="h-4 w-4" aria-hidden /> 收敛此域
        </button>
      </div>

      <section className="rounded-lg border border-slate-200 bg-white p-5 shadow-sm">
        <ElementTable elements={elements} />
      </section>

      {preview.modal}
    </div>
  );
}
