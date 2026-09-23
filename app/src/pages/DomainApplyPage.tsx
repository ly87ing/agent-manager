import { PlayCircle } from "lucide-react";
import { ElementTable } from "../components/ElementTable";
import { Switch } from "../components/Switch";
import { ErrorPanel } from "../components/ErrorPanel";
import { PageError, PageHeader, PageLoading } from "../components/PageState";
import type { StatusData } from "../lib/contract";
import { useEngineQuery } from "../lib/queries";
import { useDirectApply } from "../lib/useDirectApply";
import { useElementToggle } from "../lib/useElementToggle";

/**
 * 简单域的只读 + 同步页（Aliases）：元素表 + 「立即同步」按钮，
 * 所有写操作点击直接 apply。Hooks 以实际配置为准，另见 HooksPage。
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
  if (query.isError) return <PageError title={`${title}状态读取失败`} error={query.error} />;
  if (!query.data.ok) {
    return (
      <div className="space-y-4">
        <PageHeader title={title} description={description} />
        <ErrorPanel title={`${title}状态读取失败`} errors={query.data.errors} />
      </div>
    );
  }

  const elements = query.data.data.elements;

  return (
    <div className="space-y-4">
      <PageHeader title={title} description={description} />
      {query.data.errors.length > 0 && <ErrorPanel title={`${title}状态读取失败`} errors={query.data.errors} />}

      <div className="flex justify-end gap-2">
        <button
          type="button"
          data-testid={`apply-${domain}-button`}
          disabled={disabled}
          onClick={() => void directApply.apply({ command: `${domain}:apply`, label: `同步${title}` })}
          className="inline-flex items-center gap-2 rounded-md bg-primary px-4 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-60"
        >
          <PlayCircle className="h-4 w-4" aria-hidden /> 立即同步
        </button>
      </div>

      <section className="rounded-lg border bg-card p-5 shadow-sm">
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
