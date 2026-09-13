import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { ArrowLeft, Plus } from "lucide-react";
import { toast } from "sonner";
import { ErrorPanel } from "../components/ErrorPanel";
import { PageError, PageHeader, PageLoading } from "../components/PageState";
import { SkillAssignmentRow } from "../components/SkillAssignmentRow";
import { SkillCollectionCard, SkillDiscoveryInbox } from "../components/SkillCollectionCard";
import { SkillCollectionFormDialog } from "../components/SkillCollectionFormDialog";
import type { EngineElement, PolicyData, PolicyLayers, StatusData } from "../lib/contract";
import {
  agentCellsOf,
  type SkillAgentCell,
  type SkillCollectionObserved,
  type SkillCollectionSpec,
  type SkillObserved,
  type SkillSpec,
} from "../lib/observed";
import { useDefinitionOps } from "../lib/useDefinitionOps";
import { OVERLAY_COPY } from "../lib/ui-copy";
import { useEngineQuery } from "../lib/queries";
import { useExecuteApply, usePlanPreview } from "../lib/usePlanPreview";
import { cn } from "../lib/utils";

function isCollection(element: EngineElement): boolean {
  return element.id.startsWith("skill:") && !element.id.includes("/");
}

function fullSkillDefinition(layers: PolicyLayers | null, name: string): Record<string, unknown> | null {
  const repositories = (layers?.sqlite as { skills?: { repositories?: Record<string, unknown> } } | null | undefined)?.skills?.repositories;
  const definition = repositories?.[name];
  return typeof definition === "object" && definition !== null ? (definition as Record<string, unknown>) : null;
}

export default function SkillsPage({
  initialAgent,
  onResetAgent,
}: {
  initialAgent?: string | null;
  onResetAgent?: () => void;
} = {}) {
  const query = useEngineQuery<StatusData>("skills:status");
  const agentsQuery = useEngineQuery<StatusData>("agents:status");
  const policyQuery = useEngineQuery<PolicyData>("policy:get");
  const [activeTab, setActiveTab] = useState<"collections" | "skills">(() => (initialAgent ? "skills" : "collections"));
  const [collectionFilter, setCollectionFilter] = useState<string>("all");
  const [agentFilter, setAgentFilter] = useState<string>(() => initialAgent ?? "all");
  const [showAddForm, setShowAddForm] = useState(false);
  const [editing, setEditing] = useState<EngineElement | null>(null);

  const preview = usePlanPreview();
  const executeApply = useExecuteApply();
  const definitionOps = useDefinitionOps({ section: "skills", domain: "skills" });

  const tabRefs = useRef<Record<"collections" | "skills", HTMLButtonElement | null>>({
    collections: null,
    skills: null,
  });

  const handleTabKeyDown = (e: KeyboardEvent<HTMLButtonElement>, tab: "collections" | "skills") => {
    const tabs: ("collections" | "skills")[] = ["collections", "skills"];
    const index = tabs.indexOf(tab);
    let nextIndex = -1;
    if (e.key === "ArrowRight" || e.key === "ArrowDown") {
      e.preventDefault();
      nextIndex = (index + 1) % tabs.length;
    } else if (e.key === "ArrowLeft" || e.key === "ArrowUp") {
      e.preventDefault();
      nextIndex = (index - 1 + tabs.length) % tabs.length;
    } else if (e.key === "Home") {
      e.preventDefault();
      nextIndex = 0;
    } else if (e.key === "End") {
      e.preventDefault();
      nextIndex = tabs.length - 1;
    }
    if (nextIndex >= 0) {
      const nextTab = tabs[nextIndex];
      setActiveTab(nextTab);
      tabRefs.current[nextTab]?.focus();
    }
  };

  useEffect(() => {
    if (initialAgent) {
      setAgentFilter(initialAgent);
      setActiveTab("skills");
    } else {
      setAgentFilter("all");
    }
  }, [initialAgent]);

  if (query.isPending) return <PageLoading />;
  if (query.isError) return <PageError title="skills:status 调用失败" error={query.error} />;

  const elements = query.data.data.elements;
  const collections = elements.filter(isCollection);
  const skills = elements.filter((element) => !isCollection(element));
  const inboxSkills = skills.filter((element) => (element.observed as SkillObserved).discovery_inbox === true);

  const layers = policyQuery.data?.data.layers ?? null;

  // 助手列来源：agents:status、集合声明的 agents、技能已报告的 cell。
  // 不取 policy 层的 agents 结构——其中混有 definitions 等保留字段，不能当助手 ID 列表。
  const statusAgents = (agentsQuery.data?.data?.elements ?? []).map((el) => el.id.replace(/^agents:/, ""));
  const collectionAgents = collections.flatMap((col) => (col.spec as SkillCollectionSpec).agents ?? []);
  const skillAgents = skills.flatMap((element) => Object.keys(agentCellsOf(element)));

  const agentColumns = Array.from(new Set([...statusAgents, ...collectionAgents, ...skillAgents]))
    .filter(Boolean)
    .sort();

  const collName = (collection: EngineElement) => collection.id.replace(/^skill:/, "");

  const openCollectionAction = (
    collection: EngineElement,
    action: "update" | "enable" | "disable" | "remove",
    title: string,
    subtitle: string,
    confirmLabel: string,
  ) => {
    const args = ["--only", collection.id, "--action", action];
    preview.open({
      title,
      subtitle,
      planCommands: [{ command: "skills:plan", args }],
      confirmLabel,
      // 删除集合是单级确认：install-remove 风险时须在预览里输入集合名
      requireConfirmText: action === "remove" ? collName(collection) : undefined,
      execute: executeApply({ command: "skills:apply", label: title, extraArgs: args }),
    });
  };

  const handleUpdateCollection = (c: EngineElement) =>
    openCollectionAction(c, "update", `更新集合 ${collName(c)}`, "更新集合资源库中的技能定义，不会改动助手当前已安装的版本。", "立即更新");

  const handleToggleCollection = (c: EngineElement, action: "enable" | "disable") =>
    openCollectionAction(
      c,
      action,
      `${action === "enable" ? "启用" : "停用"}集合 ${collName(c)}`,
      action === "enable" ? "启用集合，恢复原来已启用的分配。" : "停用集合，撤下已分配入口。",
      "立即执行",
    );

  const handleRemoveCollection = (c: EngineElement) => {
    // 预览说明里给出受影响分配数（仅 assigned=true）与源目录保留语义，单级确认（无前置弹窗）
    const affected = skills
      .filter((skill) => (skill.spec as SkillSpec).collection === collName(c))
      .reduce((count, skill) => count + Object.values(agentCellsOf(skill)).filter((cell) => (cell as SkillAgentCell).assigned === true).length, 0);
    openCollectionAction(
      c,
      "remove",
      `删除集合 ${collName(c)}`,
      `从配置中移除集合，并清理 ${affected} 条受影响的助手分配；集合源目录保留在本地，不会被删除。`,
      "立即删除",
    );
  };

  const handleSkillAction = (skill: EngineElement, agent: string, action: string) => {
    const spec = skill.spec as SkillSpec;
    const skillName = spec.name ?? skill.id;
    const args = ["--only", skill.id, "--action", action, "--agent", agent];
    const labelMap: Record<string, string> = {
      add: "添加分配",
      update: "更新",
      enable: "启用",
      disable: "停用",
      remove: "删除分配",
    };
    const actionLabel = labelMap[action] ?? action;
    const label = `${actionLabel} ${skillName} @ ${agent}`;
    preview.open({
      title: label,
      subtitle: "仅同步此技能在对应助手中的配置与文件链接。",
      planCommands: [{ command: "skills:plan", args }],
      confirmLabel: "立即执行",
      execute: executeApply({ command: "skills:apply", label, extraArgs: args }),
    });
  };

  const filteredSkills = skills.filter((skill) => {
    const spec = skill.spec as SkillSpec;
    if (collectionFilter !== "all" && spec.collection !== collectionFilter) return false;
    return true;
  });

  const relevantAgents = agentFilter === "all" ? agentColumns : [agentFilter];
  const assignmentRows = filteredSkills.flatMap((skill) => {
    const spec = skill.spec as SkillSpec;
    const cells = (skill.observed as SkillObserved)?.agents ?? {};
    // cell 缺失 = 引擎未报告此助手的分配路径：不伪造可执行 cell，行内明确显示未配置
    return relevantAgents.map((agent) => ({
      key: `${skill.id}-${agent}`,
      skill,
      skillName: spec.name ?? skill.id,
      collection: spec.collection ?? "",
      agent,
      cell: cells[agent] ?? null,
    }));
  });

  // 空态引导：有集合但全未拉取时指向「更新资源库」；其余（含无集合、筛选无结果、
  // 已拉取但发现失败/无技能）保持通用文案，发现失败由集合卡片的 discovery_error 呈现。
  const allCollectionsUncached =
    collections.length > 0 &&
    collections.every((col) => (col.observed as SkillCollectionObserved).repo_cached === false);
  const emptyAssignmentsText = allCollectionsUncached
    ? "集合资源库尚未拉取：请到「集合列表」点击「更新资源库」，拉取成功后技能会出现在这里"
    : "无符合筛选条件的技能分配";

  return (
    <div className="space-y-6">
      <PageHeader title="技能" description="管理技能集合与技能分发。支持查看集合资源库状态，并在全部技能列表中按集合与助手筛选操作。" />

      {query.data.errors.length > 0 && <ErrorPanel title="skills:status 错误" errors={query.data.errors} />}

      <div role="tablist" aria-label="技能管理视图" className="flex border-b border-slate-200">
        {(["collections", "skills"] as const).map((tab) => (
          <button
            key={tab}
            ref={(el) => {
              tabRefs.current[tab] = el;
            }}
            type="button"
            role="tab"
            id={`tab-${tab}`}
            aria-controls={`panel-${tab}`}
            aria-selected={activeTab === tab}
            tabIndex={activeTab === tab ? 0 : -1}
            onClick={() => setActiveTab(tab)}
            onKeyDown={(e) => handleTabKeyDown(e, tab)}
            className={cn(
              "border-b-2 px-4 py-2 text-sm font-medium transition-colors focus:outline-none focus:ring-2 focus:ring-indigo-500",
              activeTab === tab
                ? "border-indigo-600 font-semibold text-indigo-600"
                : "border-transparent text-slate-500 hover:border-slate-300 hover:text-slate-700",
            )}
          >
            {tab === "collections" ? "集合列表" : "全部技能"}
          </button>
        ))}
      </div>

      {activeTab === "collections" && (
        <div id="panel-collections" role="tabpanel" aria-labelledby="tab-collections" className="space-y-4">
          <div className="flex items-center justify-between gap-2">
            <span className="text-xs text-slate-500">共 {collections.length} 个技能集合</span>
            <button
              type="button"
              data-testid="add-collection-button"
              onClick={() => setShowAddForm(true)}
              className="inline-flex items-center gap-1.5 rounded-md bg-indigo-600 px-3.5 py-1.5 text-xs font-medium text-white hover:bg-indigo-700 focus:outline-none focus:ring-2 focus:ring-indigo-500 focus:ring-offset-2"
            >
              <Plus className="h-4 w-4" aria-hidden /> {OVERLAY_COPY.addCollection}
            </button>
          </div>

          <div className="grid grid-cols-1 gap-4 xl:grid-cols-2">
            {collections.map((collection) => (
              <SkillCollectionCard
                key={collection.id}
                collection={collection}
                onDrilldown={(name) => {
                  setCollectionFilter(name);
                  setActiveTab("skills");
                }}
                onEdit={setEditing}
                onUpdate={handleUpdateCollection}
                onToggle={handleToggleCollection}
                onRemove={handleRemoveCollection}
              />
            ))}
          </div>

          <SkillDiscoveryInbox skills={inboxSkills} />
        </div>
      )}

      {activeTab === "skills" && (
        <div id="panel-skills" role="tabpanel" aria-labelledby="tab-skills" className="space-y-4">
          <div className="flex flex-wrap items-center justify-between gap-3 rounded-lg border border-slate-200 bg-white p-4 shadow-sm">
            <div className="flex flex-wrap items-center gap-4">
              <label className="flex items-center gap-2 text-xs font-medium text-slate-700">
                <span>集合筛选:</span>
                <select
                  aria-label="集合筛选"
                  data-testid="filter-collection-select"
                  value={collectionFilter}
                  onChange={(e) => setCollectionFilter(e.target.value)}
                  className="rounded border border-slate-300 bg-white px-2 py-1 font-mono text-xs text-slate-700"
                >
                  <option value="all">全部集合 ({collections.length})</option>
                  {collections.map((col) => (
                    <option key={col.id} value={collName(col)}>
                      {collName(col)}
                    </option>
                  ))}
                </select>
              </label>

              <label className="flex items-center gap-2 text-xs font-medium text-slate-700">
                <span>助手筛选:</span>
                <select
                  aria-label="助手筛选"
                  data-testid="filter-agent-select"
                  value={agentFilter}
                  onChange={(e) => setAgentFilter(e.target.value)}
                  className="rounded border border-slate-300 bg-white px-2 py-1 font-mono text-xs text-slate-700"
                >
                  <option value="all">全部助手 ({agentColumns.length})</option>
                  {agentColumns.map((ag) => (
                    <option key={ag} value={ag}>
                      {ag}
                    </option>
                  ))}
                </select>
              </label>

              {(collectionFilter !== "all" || agentFilter !== "all") && (
                <button
                  type="button"
                  onClick={() => {
                    setCollectionFilter("all");
                    setAgentFilter("all");
                    onResetAgent?.();
                  }}
                  className="inline-flex items-center gap-1 rounded border border-slate-200 px-2 py-1 text-xs text-slate-600 hover:bg-slate-50"
                >
                  <ArrowLeft className="h-3 w-3" aria-hidden /> 清除筛选
                </button>
              )}
            </div>

            <span className="text-xs text-slate-500">当前展示 {assignmentRows.length} 条分配记录</span>
          </div>

          <div className="overflow-x-auto rounded-lg border border-slate-200 bg-white p-5 shadow-sm">
            <table className="w-full text-left text-sm">
              <thead>
                <tr className="border-b border-slate-200 text-xs text-slate-500">
                  <th className="py-2.5 pr-4 font-medium">技能名</th>
                  <th className="py-2.5 pr-4 font-medium">所属集合</th>
                  <th className="py-2.5 pr-4 font-medium">助手</th>
                  <th className="py-2.5 pr-4 font-medium">资源库版本</th>
                  <th className="py-2.5 pr-4 font-medium">当前分配版本</th>
                  <th className="py-2.5 pr-4 font-medium">状态</th>
                  <th className="py-2.5 font-medium">操作</th>
                </tr>
              </thead>
              <tbody>
                {assignmentRows.map(({ key, skill, skillName, collection, agent, cell }) => (
                  <SkillAssignmentRow
                    key={key}
                    skill={skill}
                    skillName={skillName}
                    collection={collection}
                    agent={agent}
                    cell={cell}
                    onFilterCollection={setCollectionFilter}
                    onFilterAgent={setAgentFilter}
                    onAction={handleSkillAction}
                  />
                ))}
              </tbody>
            </table>
            {assignmentRows.length === 0 && (
              <p data-testid="skills-empty-state" className="py-8 text-center text-sm text-slate-400">{emptyAssignmentsText}</p>
            )}
          </div>
        </div>
      )}

      {(showAddForm || editing) && (
        <SkillCollectionFormDialog
          agentIds={agentColumns}
          initial={editing ? { name: editing.id.replace(/^skill:/, ""), definition: editing.spec } : undefined}
          fullDefinition={editing ? fullSkillDefinition(layers, editing.id.replace(/^skill:/, "")) : null}
          onClose={() => {
            setShowAddForm(false);
            setEditing(null);
          }}
          onSubmit={(name, definition, complete) => {
            setShowAddForm(false);
            setEditing(null);
            const isNew = !editing;
            void definitionOps
              .saveAndApply({
                name,
                definition,
                isNew,
                elementId: `skill:${name}`,
                complete,
              })
              .then(() => {
                if (isNew) {
                  toast.info(`集合 ${name} 定义已保存，请点击「更新资源库」以拉取最新技能。`);
                }
              });
          }}
        />
      )}

      {preview.modal}
      {definitionOps.modals}
    </div>
  );
}
