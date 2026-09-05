import { useState } from "react";
import { ChevronDown, ChevronRight, Inbox } from "lucide-react";
import { DriftExplanationPanel } from "../components/DriftExplanationPanel";
import { ErrorPanel } from "../components/ErrorPanel";
import { FreshnessBadge } from "../components/FreshnessBadge";
import { MatrixCell } from "../components/MatrixCell";
import { PageError, PageHeader, PageLoading } from "../components/PageState";
import { StateBadge } from "../components/StateBadge";
import type { EngineElement, StatusData } from "../lib/contract";
import {
  agentCellsOf,
  type SkillCollectionObserved,
  type SkillCollectionSpec,
  type SkillObserved,
  type SkillSpec,
} from "../lib/observed";
import { useEngineQuery } from "../lib/queries";
import { cn } from "../lib/utils";

function isCollection(element: EngineElement): boolean {
  return element.id.startsWith("skill:") && !element.id.includes("/");
}

function shortCommit(value: string | null): string {
  if (!value) return "?";
  return value.length > 12 ? value.slice(0, 7) : value;
}

function CollectionCard({ element, inboxCount }: { element: EngineElement; inboxCount: number }) {
  const spec = element.spec as SkillCollectionSpec;
  const observed = element.observed as SkillCollectionObserved;
  const name = element.id.replace(/^skill:/, "");
  return (
    <section data-testid={`collection-card-${name}`} className="rounded-lg border border-slate-200 bg-white p-5 shadow-sm">
      <div className="flex items-start justify-between gap-3">
        <div>
          <h3 className="text-base font-semibold text-slate-800">{name}</h3>
          <p className="mt-0.5 font-mono text-xs break-all text-slate-400">{spec.url ?? "（无 url）"}</p>
        </div>
        <StateBadge state={element.state} />
      </div>

      <div className="mt-3 flex flex-wrap gap-1.5 text-xs">
        <span className="rounded border border-slate-200 bg-slate-50 px-1.5 py-0.5 text-slate-600">
          {spec.matrix_mode ? "矩阵模式" : "旧式升格"}
        </span>
        <span className="rounded border border-slate-200 bg-slate-50 px-1.5 py-0.5 text-slate-600">
          new_skill_default: {spec.new_skill_default ?? "disabled"}
        </span>
        <span
          className={cn(
            "rounded border px-1.5 py-0.5",
            observed.owner_marked
              ? "border-indigo-200 bg-indigo-50 text-indigo-700"
              : "border-slate-200 bg-slate-50 text-slate-500",
          )}
        >
          {observed.owner_marked ? "自研" : "第三方"}
        </span>
        <span
          className={cn(
            "rounded border px-1.5 py-0.5",
            observed.repo_cached
              ? "border-emerald-200 bg-emerald-50 text-emerald-700"
              : "border-amber-200 bg-amber-50 text-amber-700",
          )}
          title={observed.repo_cache}
        >
          {observed.repo_cached ? "仓库已缓存" : "仓库未缓存"}
        </span>
        {inboxCount > 0 && (
          <span
            data-testid={`inbox-count-${name}`}
            className="rounded border border-sky-200 bg-sky-50 px-1.5 py-0.5 font-medium text-sky-700"
          >
            收件箱 {inboxCount}
          </span>
        )}
      </div>

      <div className="mt-3 flex items-center justify-between border-t border-slate-100 pt-3 text-xs text-slate-500">
        <span>
          发现 {observed.discovered?.length ?? 0} 个 skill · 跟随上游 HEAD{" "}
          <code className="font-mono">{shortCommit(element.freshness.upstream)}</code>
        </span>
        <FreshnessBadge freshness={element.freshness} className="items-end" />
      </div>
      {(observed.extra_links?.length ?? 0) > 0 && (
        <p className="mt-2 text-xs text-amber-700">
          存在策略外额外链接:{observed.extra_links!.join(", ")}
        </p>
      )}
    </section>
  );
}

export default function SkillsPage() {
  const query = useEngineQuery<StatusData>("skills:status");
  const [expandedId, setExpandedId] = useState<string | null>(null);

  if (query.isPending) return <PageLoading />;
  if (query.isError) return <PageError title="skills:status 调用失败" error={query.error} />;

  const elements = query.data.data.elements;
  const collections = elements.filter(isCollection);
  const skills = elements.filter((element) => !isCollection(element));
  const inboxSkills = skills.filter((element) => (element.observed as SkillObserved).discovery_inbox === true);
  const matrixSkills = skills.filter((element) => (element.observed as SkillObserved).discovery_inbox !== true);
  const agentColumns = Array.from(
    new Set(matrixSkills.flatMap((element) => Object.keys(agentCellsOf(element)))),
  ).sort();

  return (
    <div className="space-y-6">
      <PageHeader
        title="Skills"
        description="集合 / 发现收件箱 / 启用矩阵三级视图（只读）。上游新增 skill 只进收件箱，显式启用才分发；矩阵点击切换在 Phase 2 开放。"
      />
      {query.data.errors.length > 0 && <ErrorPanel title="skills:status 错误" errors={query.data.errors} />}

      <div className="grid grid-cols-1 gap-4 xl:grid-cols-2">
        {collections.map((collection) => (
          <CollectionCard
            key={collection.id}
            element={collection}
            inboxCount={(collection.observed as SkillCollectionObserved).discovery_inbox?.length ?? 0}
          />
        ))}
      </div>

      <section data-testid="discovery-inbox" className="rounded-lg border border-slate-200 bg-white p-5 shadow-sm">
        <h3 className="mb-3 flex items-center gap-2 text-sm font-semibold text-slate-700">
          <Inbox className="h-4 w-4 text-sky-600" aria-hidden />
          发现收件箱
          <span data-testid="inbox-total" className="rounded-full bg-sky-100 px-2 py-0.5 text-xs font-medium text-sky-700">
            {inboxSkills.length}
          </span>
        </h3>
        {inboxSkills.length === 0 ? (
          <p className="text-sm text-slate-400">收件箱为空：上游集合没有新发现的 skill。</p>
        ) : (
          <table className="w-full text-left text-sm">
            <thead>
              <tr className="border-b border-slate-200 text-xs text-slate-500">
                <th className="py-2 pr-4 font-medium">名称</th>
                <th className="py-2 pr-4 font-medium">来源</th>
                <th className="py-2 font-medium">状态说明</th>
              </tr>
            </thead>
            <tbody>
              {inboxSkills.map((skill) => {
                const spec = skill.spec as SkillSpec;
                return (
                  <tr key={skill.id} className="border-b border-slate-100 last:border-0">
                    <td className="py-2 pr-4 font-mono text-xs text-slate-700">{spec.name ?? skill.id}</td>
                    <td className="py-2 pr-4 font-mono text-xs break-all text-slate-500">{spec.source_path ?? spec.collection}</td>
                    <td className="py-2 text-xs text-slate-600">上游集合新出现；显式启用才分发（new_skill_default=disabled）</td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        )}
      </section>

      <section className="overflow-x-auto rounded-lg border border-slate-200 bg-white p-5 shadow-sm">
        <h3 className="mb-3 text-sm font-semibold text-slate-700">skill × agent 启用矩阵</h3>
        <table className="w-full text-left text-sm">
          <thead>
            <tr className="border-b border-slate-200 text-xs text-slate-500">
              <th className="py-2 pr-4 font-medium">Skill</th>
              {agentColumns.map((agent) => (
                <th key={agent} className="py-2 pr-4 font-mono font-medium">
                  {agent}
                </th>
              ))}
              <th className="py-2 font-medium">收敛状态</th>
            </tr>
          </thead>
          <tbody>
            {matrixSkills.map((skill) => {
              const spec = skill.spec as SkillSpec;
              const cells = agentCellsOf(skill);
              const isExpanded = skill.id === expandedId;
              return (
                <FragmentRow
                  key={skill.id}
                  skill={skill}
                  name={spec.name ?? skill.id}
                  cells={cells}
                  agentColumns={agentColumns}
                  isExpanded={isExpanded}
                  onToggle={() => setExpandedId(isExpanded ? null : skill.id)}
                />
              );
            })}
          </tbody>
        </table>
        {matrixSkills.length === 0 && <p className="py-4 text-center text-sm text-slate-400">无已声明 skill</p>}
      </section>
    </div>
  );
}

function FragmentRow({
  skill,
  name,
  cells,
  agentColumns,
  isExpanded,
  onToggle,
}: {
  skill: EngineElement;
  name: string;
  cells: Record<string, { state: string }>;
  agentColumns: string[];
  isExpanded: boolean;
  onToggle: () => void;
}) {
  return (
    <>
      <tr className={cn("border-b border-slate-100", !isExpanded && "last:border-0")}>
        <td className="py-2 pr-4">
          <button
            type="button"
            onClick={onToggle}
            aria-expanded={isExpanded}
            className="inline-flex items-center gap-1 font-mono text-xs text-indigo-600 hover:text-indigo-800"
          >
            {isExpanded ? <ChevronDown className="h-3 w-3" aria-hidden /> : <ChevronRight className="h-3 w-3" aria-hidden />}
            {name}
          </button>
        </td>
        {agentColumns.map((agent) => {
          const cell = cells[agent];
          return (
            <td key={agent} className="py-2 pr-4">
              {cell ? (
                <MatrixCell state={cell.state} kind="skills" title={`${agent}: ${cell.state}`} />
              ) : (
                <span className="text-xs text-slate-300">—</span>
              )}
            </td>
          );
        })}
        <td className="py-2">
          <StateBadge state={skill.state} />
        </td>
      </tr>
      {isExpanded && (
        <tr className="border-b border-slate-100 last:border-0">
          <td colSpan={agentColumns.length + 2} className="py-3">
            <DriftExplanationPanel element={skill} planCommand="skills:plan" cellKind="skills" />
          </td>
        </tr>
      )}
    </>
  );
}
