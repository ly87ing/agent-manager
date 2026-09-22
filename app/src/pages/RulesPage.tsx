import { useState } from "react";
import { ChevronDown, ChevronRight, PlayCircle } from "lucide-react";
import { DriftExplanationPanel } from "../components/DriftExplanationPanel";
import { SourceDefinitionsPanel } from "../components/SourceDefinitionsPanel";
import { ErrorPanel } from "../components/ErrorPanel";
import { PageError, PageHeader, PageLoading } from "../components/PageState";
import { StateBadge } from "../components/StateBadge";
import type { EngineElement, StatusData } from "../lib/contract";
import type { RulesObserved, RulesSpec } from "../lib/observed";
import { useEngineQuery } from "../lib/queries";
import { TakeoverButton } from "../components/TakeoverButton";
import { useDirectApply } from "../lib/useDirectApply";
import { cn } from "../lib/utils";

const SYNC_NOTE = "从本机 SQLite 来源渲染并同步到目标文件；目标被外部修改时会拒绝覆盖。";

/** 该元素详情/操作用的精确限定参数。 */
function onlyArgs(element: EngineElement): string[] {
  return ["--only", element.id];
}

export default function RulesPage() {
  const query = useEngineQuery<StatusData>("rules:status");
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const directApply = useDirectApply();

  if (query.isPending) return <PageLoading />;
  if (query.isError) return <PageError title="规则状态读取失败" error={query.error} />;
  if (!query.data.ok) {
    return (
      <div className="space-y-4">
        <PageHeader title="规则" description={SYNC_NOTE} />
        <ErrorPanel title="规则状态读取失败" errors={query.data.errors} />
      </div>
    );
  }

  const elements = query.data.data.elements;

  return (
    <div className="space-y-4">
      <PageHeader title="规则" description={SYNC_NOTE} />
      {query.data.errors.length > 0 && <ErrorPanel title="规则状态读取失败" errors={query.data.errors} />}
      <SourceDefinitionsPanel domain="rules" />

      <div className="flex justify-end">
        <button
          type="button"
          data-testid="apply-rules-button"
          disabled={directApply.executing}
          onClick={() => void directApply.apply({ command: "rules:apply", label: "同步规则" })}
          className="inline-flex items-center gap-2 rounded-md bg-primary px-4 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-60"
        >
          <PlayCircle className="h-4 w-4" aria-hidden /> 立即同步
        </button>
      </div>

      <section className="overflow-x-auto rounded-lg border bg-card p-5 shadow-sm">
        <table className="w-full text-left text-sm"><thead>
            <tr className="border-b text-xs text-muted-foreground">
              <th className="py-2 pr-4 font-medium">名称</th>
              <th className="py-2 pr-4 font-medium">来源定义</th>
              <th className="py-2 pr-4 font-medium">写入位置</th>
              <th className="py-2 pr-4 font-medium">同步状态</th>
              <th className="py-2 font-medium">操作</th>
            </tr>
          </thead>
          <tbody>
            {elements.map((element) => (
              <TargetRow
                key={element.id}
                element={element}
                isExpanded={element.id === expandedId}
                onToggle={() => setExpandedId(element.id === expandedId ? null : element.id)}
                onSync={() => {
                  const spec = element.spec as unknown as RulesSpec;
                  const name = spec.name ?? element.id;
                  void directApply.apply({
                    command: "rules:apply",
                    label: `同步 ${name}`,
                    args: onlyArgs(element),
                  });
                }}
                busy={directApply.executing}
                onTakeover={() =>
                  void directApply.apply({
                    command: "rules:apply",
                    label: `接管原内容 ${element.id}`,
                    args: ["--only", element.id, "--allow-takeover"],
                  })
                }
              />
            ))}
          </tbody>
        </table>
      </section>
    </div>
  );
}

function TargetRow({
  element,
  isExpanded,
  onToggle,
  onSync,
  onTakeover,
  busy,
}: {
  element: EngineElement;
  isExpanded: boolean;
  onToggle: () => void;
  onTakeover: () => void;
  busy: boolean;
  onSync: () => void;
}) {
  const spec = element.spec as unknown as RulesSpec;
  const observed = element.observed as unknown as RulesObserved;
  const name = spec.name ?? element.id;
  const hasSync = element.actions.some((action) => action.id === "sync");
  return (
    <>
      <tr className={cn("border-b", !isExpanded && "last:border-0")}>
        <td className="py-2 pr-4">
          <button
            type="button"
            onClick={onToggle}
            aria-expanded={isExpanded}
            className="inline-flex items-center gap-1 text-xs font-medium text-primary hover:text-primary"
          >
            {isExpanded ? <ChevronDown className="h-3 w-3" aria-hidden /> : <ChevronRight className="h-3 w-3" aria-hidden />}
            {name}
          </button>
        </td>
        <td className="py-2 pr-4 font-mono text-xs break-all text-muted-foreground">
          {spec.source_keys && spec.source_keys.length > 0 ? spec.source_keys.join(" + ") : "—"}
        </td>
        <td className="py-2 pr-4 font-mono text-xs break-all text-muted-foreground">{observed.target ?? spec.target ?? "—"}</td>
        <td className="py-2 pr-4">
          <StateBadge state={element.state} />
        </td>
        <td className="py-2">
          {hasSync && (
            <button
              type="button"
              data-testid={`action-sync-${element.id}`}
              onClick={onSync}
              className="rounded border border-primary/30 px-2.5 py-1 text-xs text-primary hover:bg-primary/10"
            >
              同步此目标
            </button>
          )}
          <TakeoverButton
            element={element}
            domain="rules"
            disabled={busy}
            onTakeover={onTakeover}
            showNote={false}
          />
        </td>
      </tr>
      {isExpanded && (
        <tr className="border-b last:border-0">
          <td colSpan={5} className="py-3">
            <DriftExplanationPanel element={element} />
          </td>
        </tr>
      )}
    </>
  );
}
