import { PlayCircle } from "lucide-react";
import { useState } from "react";
import { DefinitionFormDialog } from "../components/DefinitionFormDialog";
import { SourceDefinitionsPanel } from "../components/SourceDefinitionsPanel";
import { useDefinitionOps } from "../lib/useDefinitionOps";
import { ElementTable } from "../components/ElementTable";
import { Switch } from "../components/Switch";
import { ErrorPanel } from "../components/ErrorPanel";
import { PageError, PageHeader, PageLoading } from "../components/PageState";
import type { EngineElement, PolicyData, StatusData } from "../lib/contract";
import { useEngineQuery } from "../lib/queries";
import { useElementToggle } from "../lib/useElementToggle";
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
  const policyQuery = useEngineQuery<PolicyData>("policy:get");
  const definitionOps = useDefinitionOps({ section: "tools", domain: "tools" });
  const [form, setForm] = useState<EngineElement | "new" | null>(null);

  if (query.isPending) return <PageLoading />;
  if (query.isError) return <PageError title={`${domain}:status 调用失败`} error={query.error} />;

  const elements = query.data.data.elements;

  return (
    <div className="space-y-4">
      <PageHeader title={title} description={description} />
      {query.data.errors.length > 0 && <ErrorPanel title={`${domain}:status 错误`} errors={query.data.errors} />}
      {(domain === "hooks" || domain === "aliases") && <SourceDefinitionsPanel domain={domain} />}

      <div className="flex justify-end gap-2">
        {domain === "tools" && <button type="button" onClick={() => setForm("new")} className="rounded-md border border-indigo-200 px-4 py-2 text-sm text-indigo-600">添加工具</button>}
        <button
          type="button"
          data-testid={`apply-${domain}-button`}
          onClick={() =>
            preview.open({
              title: `同步${title}`,
              subtitle: "先预览将要执行的操作；实际执行前会自动备份。",
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
                    <div className="flex flex-wrap items-center gap-2">
                    {domain === "tools" && <>
                      <button type="button" onClick={() => setForm(element)} className="rounded border px-2 py-1 text-xs">编辑</button>
                      <button type="button" onClick={() => definitionOps.requestRemove(shortName, policyQuery.data?.data.layers ?? null)} className="rounded border px-2 py-1 text-xs text-red-600">移除管理</button>
                      <button type="button" onClick={() => preview.open({ title: `更新 ${shortName}`, planCommands: [{ command: "tools:plan", args: ["--only", element.id] }], execute: executeApply({ command: "tools:apply", label: `更新 ${shortName}`, extraArgs: ["--only", element.id] }) })} className="rounded border px-2 py-1 text-xs text-indigo-600">更新</button>
                    </>}
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
                    </div>
                  );
                }
              : undefined
          }
        />
      </section>

      {preview.modal}
      {elementToggle.modal}
      {domain === "tools" && form && <DefinitionFormDialog domain="tools" existingNames={elements.map((element) => element.id.replace(/^tools:/, ""))} initial={form === "new" ? undefined : { name: form.id.replace(/^tools:/, ""), definition: form.spec }} onClose={() => setForm(null)} onSubmit={(name, definition) => { setForm(null); void definitionOps.saveAndApply({ name, definition, isNew: form === "new", elementId: `tools:${name}` }); }} />}
      {definitionOps.modals}
    </div>
  );
}
