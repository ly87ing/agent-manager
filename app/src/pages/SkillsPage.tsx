import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { ArrowLeft, PackageOpen, Plus, SearchX } from "lucide-react";
import { toast } from "sonner";
import { ConfirmModal } from "../components/ConfirmModal";
import { EmptyState } from "../components/EmptyState";
import { ErrorPanel } from "../components/ErrorPanel";
import { PageError, PageHeader, PageLoading } from "../components/PageState";
import { SearchInput, matchesKeyword } from "../components/SearchInput";
import { SkillAssignmentRow } from "../components/SkillAssignmentRow";
import { SkillCollectionCard, SkillDiscoveryInbox } from "../components/SkillCollectionCard";
import { SkillCollectionFormDialog } from "../components/SkillCollectionFormDialog";
import type { EngineElement, PolicyData, PolicyLayers, StatusData } from "../lib/contract";
import {
  agentCellsOf,
  type SkillCollectionObserved,
  type SkillCollectionSpec,
  type SkillObserved,
  type SkillSpec,
} from "../lib/observed";
import { useDefinitionOps } from "../lib/useDefinitionOps";
import { useDirectApply } from "../lib/useDirectApply";
import { OVERLAY_COPY } from "../lib/ui-copy";
import { useEngineQuery } from "../lib/queries";
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
  const [search, setSearch] = useState("");
  const [showAddForm, setShowAddForm] = useState(false);
  const [editing, setEditing] = useState<EngineElement | null>(null);

  const directApply = useDirectApply();
  const definitionOps = useDefinitionOps({ section: "skills", domain: "skills" });

  const [pendingRemove, setPendingRemove] = useState<
    | { kind: "collection"; element: EngineElement }
    | { kind: "skill"; element: EngineElement; agent: string }
    | null
  >(null);

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

  /** 集合操作：按钮严格来自卡片动作，apply 携带精确 --only/--action；remove 需先经独立 ConfirmModal。 */
  const runCollectionAction = (collection: EngineElement, action: "update" | "enable" | "disable" | "remove", title: string) => {
    if (action === "remove") {
      setPendingRemove({ kind: "collection", element: collection });
      return;
    }
    void directApply.apply({ command: "skills:apply", label: title, args: ["--only", collection.id, "--action", action] });
  };

  const handleUpdateCollection = (c: EngineElement) =>
    runCollectionAction(c, "update", `更新集合 ${collName(c)}`);

  const handleToggleCollection = (c: EngineElement, action: "enable" | "disable") =>
    runCollectionAction(c, action, `${action === "enable" ? "启用" : "停用"}集合 ${collName(c)}`);

  const handleRemoveCollection = (c: EngineElement) =>
    runCollectionAction(c, "remove", `删除集合 ${collName(c)}`);

  /** 技能 × 助手 操作：apply 携带精确 --only/--action/--agent；remove 需先经独立 ConfirmModal。 */
  const handleSkillAction = (skill: EngineElement, agent: string, action: string) => {
    if (action === "remove") {
      setPendingRemove({ kind: "skill", element: skill, agent });
      return;
    }
    const spec = skill.spec as SkillSpec;
    const skillName = spec.name ?? skill.id;
    const labelMap: Record<string, string> = {
      add: "添加分配",
      update: "更新",
      enable: "启用",
      disable: "停用",
      remove: "删除分配",
    };
    const label = `${labelMap[action] ?? action} ${skillName} @ ${agent}`;
    void directApply.apply({
      command: "skills:apply",
      label,
      args: ["--only", skill.id, "--action", action, "--agent", agent],
    });
  };

  const filteredSkills = skills.filter((skill) => {
    const spec = skill.spec as SkillSpec;
    if (collectionFilter !== "all" && spec.collection !== collectionFilter) return false;
    return matchesKeyword(search, [spec.name, skill.id, spec.collection]);
  });

  /** 集合列表搜索：与技能列表共用同一关键词，仅作用于本 tab 的卡片网格。 */
  const filteredCollections = collections.filter((collection) => {
    const spec = collection.spec as SkillCollectionSpec;
    return matchesKeyword(search, [collName(collection), spec.url]);
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

  const confirmRemove = () => {
    if (!pendingRemove) return;
    if (pendingRemove.kind === "collection") {
      const name = collName(pendingRemove.element);
      void directApply.apply({
        command: "skills:apply",
        label: `删除集合 ${name}`,
        args: ["--only", pendingRemove.element.id, "--action", "remove"],
      });
    } else {
      const spec = pendingRemove.element.spec as SkillSpec;
      const skillName = spec.name ?? pendingRemove.element.id;
      void directApply.apply({
        command: "skills:apply",
        label: `删除分配 ${skillName} @ ${pendingRemove.agent}`,
        args: ["--only", pendingRemove.element.id, "--action", "remove", "--agent", pendingRemove.agent],
      });
    }
    setPendingRemove(null);
  };

  return (
    <div className="space-y-6">
      <PageHeader title="技能" description="管理技能集合与技能分发。支持查看集合资源库状态，并在全部技能列表中搜索、按集合与助手筛选操作。" />

      {query.data.errors.length > 0 && <ErrorPanel title="skills:status 错误" errors={query.data.errors} />}

      <div role="tablist" aria-label="技能管理视图" className="flex border-b">
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
              "border-b-2 px-4 py-2 text-sm font-medium transition-colors focus:outline-none focus:ring-2 focus:ring-ring",
              activeTab === tab
                ? "border-primary font-semibold text-primary"
                : "border-transparent text-muted-foreground hover:border-input hover:text-foreground",
            )}
          >
            {tab === "collections" ? "集合列表" : "全部技能"}
          </button>
        ))}
      </div>

      {activeTab === "collections" && (
        <div id="panel-collections" role="tabpanel" aria-labelledby="tab-collections" className="space-y-4">
          <div className="flex items-center justify-between gap-2">
            <div className="flex items-center gap-3">
              <span className="text-xs text-muted-foreground">共 {collections.length} 个技能集合</span>
              <SearchInput value={search} onChange={setSearch} placeholder="搜索集合或技能…" testId="skills-search" />
            </div>
            <button
              type="button"
              data-testid="add-collection-button"
              onClick={() => setShowAddForm(true)}
              className="inline-flex items-center gap-1.5 rounded-md bg-primary px-3.5 py-1.5 text-xs font-medium text-primary-foreground hover:bg-primary/90 focus:outline-none focus:ring-2 focus:ring-ring focus:ring-offset-2"
            >
              <Plus className="h-4 w-4" aria-hidden /> {OVERLAY_COPY.addCollection}
            </button>
          </div>

          {search.trim() && filteredCollections.length === 0 ? (
            <EmptyState
              icon={SearchX}
              testId="skills-search-empty"
              title="无匹配结果"
              description="没有名称或地址包含该关键词的技能集合。"
              actions={
                <button
                  type="button"
                  onClick={() => setSearch("")}
                  className="inline-flex items-center gap-1 rounded border px-2 py-1 text-xs text-muted-foreground hover:bg-accent"
                >
                  清除搜索关键词
                </button>
              }
            />
          ) : (
            <div className="grid grid-cols-1 gap-4 xl:grid-cols-2">
              {filteredCollections.map((collection) => (
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
          )}

          <SkillDiscoveryInbox skills={inboxSkills} />
        </div>
      )}

      {activeTab === "skills" && (
        <div id="panel-skills" role="tabpanel" aria-labelledby="tab-skills" className="space-y-4">
          <div className="flex flex-wrap items-center justify-between gap-3 rounded-lg border bg-card p-4 shadow-sm">
            <div className="flex flex-wrap items-center gap-4">
              <SearchInput value={search} onChange={setSearch} placeholder="搜索集合或技能…" testId="skills-search" />
              <label className="flex items-center gap-2 text-xs font-medium text-foreground">
                <span>集合筛选:</span>
                <select
                  aria-label="集合筛选"
                  data-testid="filter-collection-select"
                  value={collectionFilter}
                  onChange={(e) => setCollectionFilter(e.target.value)}
                  className="rounded border border-input bg-card px-2 py-1 font-mono text-xs text-foreground"
                >
                  <option value="all">全部集合 ({collections.length})</option>
                  {collections.map((col) => (
                    <option key={col.id} value={collName(col)}>
                      {collName(col)}
                    </option>
                  ))}
                </select>
              </label>

              <label className="flex items-center gap-2 text-xs font-medium text-foreground">
                <span>助手筛选:</span>
                <select
                  aria-label="助手筛选"
                  data-testid="filter-agent-select"
                  value={agentFilter}
                  onChange={(e) => setAgentFilter(e.target.value)}
                  className="rounded border border-input bg-card px-2 py-1 font-mono text-xs text-foreground"
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
                  className="inline-flex items-center gap-1 rounded border px-2 py-1 text-xs text-muted-foreground hover:bg-accent"
                >
                  <ArrowLeft className="h-3 w-3" aria-hidden /> 清除筛选
                </button>
              )}
            </div>

            <span className="text-xs text-muted-foreground">当前展示 {assignmentRows.length} 条分配记录</span>
          </div>

          <div className="overflow-x-auto rounded-lg border bg-card p-5 shadow-sm">
            <table className="w-full text-left text-sm">
              <thead>
                <tr className="border-b text-xs text-muted-foreground">
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
            {assignmentRows.length === 0 &&
              (search.trim() ? (
                <EmptyState
                  compact
                  testId="skills-empty-state"
                  icon={SearchX}
                  title="无匹配结果"
                  description="没有名称或所属集合包含该关键词的技能分配。"
                  actions={
                    <button
                      type="button"
                      onClick={() => setSearch("")}
                      className="inline-flex items-center gap-1 rounded border px-2 py-1 text-xs text-muted-foreground hover:bg-accent"
                    >
                      清除搜索关键词
                    </button>
                  }
                />
              ) : (
                <EmptyState
                  compact
                  testId="skills-empty-state"
                  icon={allCollectionsUncached ? PackageOpen : SearchX}
                  title={allCollectionsUncached ? "集合资源库尚未拉取" : "无符合筛选条件的技能分配"}
                  description={
                    allCollectionsUncached
                      ? "请到「集合列表」点击「更新资源库」，拉取成功后技能会出现在这里"
                      : undefined
                  }
                />
              ))}
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
              .saveDefinition({
                name,
                definition,
                isNew,
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

      {definitionOps.modals}

      {pendingRemove && (
        <ConfirmModal
          title={
            pendingRemove.kind === "collection"
              ? `删除集合 ${collName(pendingRemove.element)}`
              : `删除分配 ${(pendingRemove.element.spec as SkillSpec).name ?? pendingRemove.element.id} @ ${pendingRemove.agent}`
          }
          description={
            pendingRemove.kind === "collection"
              ? "删除集合会清理已分配给各助手的入口，源目录保留。确认后执行。"
              : "只清理该助手的入口。确认后执行。"
          }
          risk={pendingRemove.kind === "collection" ? "install-remove" : "write-local"}
          requireText={pendingRemove.kind === "collection" ? collName(pendingRemove.element) : undefined}
          confirmLabel="确认删除"
          executing={directApply.executing}
          onConfirm={confirmRemove}
          onClose={() => setPendingRemove(null)}
        />
      )}
    </div>
  );
}
