import { PlayCircle } from "lucide-react";
import { SourceDefinitionsPanel } from "../components/SourceDefinitionsPanel";
import { ElementTable } from "../components/ElementTable";
import { Switch } from "../components/Switch";
import { ErrorPanel } from "../components/ErrorPanel";
import { PageError, PageHeader, PageLoading } from "../components/PageState";
import type { StatusData } from "../lib/contract";
import { useEngineQuery } from "../lib/queries";
import { useElementToggle } from "../lib/useElementToggle";
import { useExecuteApply, usePlanPreview } from "../lib/usePlanPreview";

/**
 * 简单域的只读 + 收敛页（Aliases / Hooks）：元素表 + 「收敛此域」按钮
 * （plan 预览确认 → 域级 apply）。
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
  const preview = usePlanPreview();
  const executeApply = useExecuteApply();
  const elementToggle = useElementToggle();

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

  return (
    <div className="space-y-4">
      <PageHeader title={title} description={description} />
      {query.data.errors.length > 0 && <ErrorPanel title={`${domain}:status 错误`} errors={query.data.errors} />}
      {(domain === "hooks" || domain === "aliases") && <SourceDefinitionsPanel domain={domain} />}

      <div className="flex justify-end gap-2">
        <button
          type="button"
          data-testid={`apply-${domain}-button`}
          onClick={() =>
            preview.open({
              title: `同步${title}`,
              subtitle: "先预览将要执行的操作；确认后按引擎计划执行。",
              planCommands: [`${domain}:plan`],
              confirmLabel: "立即同步",
              execute: executeApply({ command: `${domain}:apply`, label: `同步${title}` }),
            })
          }
          className="inline-flex items-center gap-2 rounded-md bg-indigo-600 px-4 py-2 text-sm font-medium text-white hover:bg-indigo-700"
        >
          <PlayCircle className="h-4 w-4" aria-hidden /> 立即同步
        </button>
      </div>

      <section className="rounded-lg border border-slate-200 bg-white p-5 shadow-sm">
        <ElementTable
          elements={elements}
          renderActions={
            enableToggles
              ? (element) => {
                  const shortName = element.id.replace(`${domain}:`, "");
                  const enabled = (element.policy as { enabled?: boolean }).enabled !== false;
                  return (
                    <Switch
                      checked={enabled}
                      testId={`toggle-${element.id}`}
                      label={`${enabled ? "停用" : "启用"} ${shortName}`}
                      onChange={() =>
                        void elementToggle.toggle({
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

      {preview.modal}
      {elementToggle.modal}
    </div>
  );
}
