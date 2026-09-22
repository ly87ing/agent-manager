import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { ArrowLeft, PackageOpen, Plus, SearchX } from "lucide-react";
import { toast } from "sonner";
import { ConfirmModal } from "../components/ConfirmModal";
import { EmptyState } from "../components/EmptyState";
import { ErrorPanel } from "../components/ErrorPanel";
import { PageError, PageHeader, PageLoading } from "../components/PageState";
import { SearchInput, matchesKeyword } from "../components/SearchInput";
import { ResourceAgentMatrix, type MatrixRow, type MatrixRowCell } from "../components/ResourceAgentMatrix";
import { SkillAgentCellActions, skillNeedsUpdate } from "../components/SkillAgentActions";
import { SkillCollectionCard } from "../components/SkillCollectionCard";
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
import { useBatchApply, type BatchApplyRow } from "../lib/useBatchApply";
import { useDirectApply } from "../lib/useDirectApply";
import { OVERLAY_COPY } from "../lib/ui-copy";
import { engineKeys, useEngineQuery } from "../lib/queries";
import { runEngine } from "../lib/engine";
import { useQueryClient } from "@tanstack/react-query";
import { cn } from "../lib/utils";
import { supportedAgentKindOf } from "../lib/agentManagement";

function isCollection(element: EngineElement): boolean {
  return element.id.startsWith("skill:") && !element.id.includes("/");
}

/** 全部技能矩阵的批量动作：更新/启停直执；add/remove 仅作用于计划外链接行。 */
type BatchSkillAction = "update" | "enable" | "disable" | "add" | "remove";

/** 批量按钮配色与行内操作按钮保持一致（更新/启用/停用/纳入管理/取消链接）。 */
const BATCH_BUTTON_CLASS: Record<BatchSkillAction, string> = {
  update: "border-primary/30 bg-primary/10 text-primary hover:bg-primary/15",
  enable: "border-success/30 bg-success/10 text-success hover:bg-success/15",
  disable: "border-warning/30 bg-warning/10 text-warning hover:bg-warning/15",
  add: "border-primary/30 bg-primary/10 text-primary hover:bg-primary/15",
  remove: "border-destructive/30 bg-card text-destructive hover:bg-destructive/10",
};

function isBatchSelectableCell(cell: SkillAgentCell | null): boolean {
  const actions = cell?.actions ?? [];
  return Boolean(
    skillNeedsUpdate(cell) ||
      skillEnableAction(cell) ||
      actions.includes("disable") ||
      (cell?.state === "extra-link" && (actions.includes("add") || actions.includes("remove"))),
  );
}

/** 用户界面的「启用」：未分配时实际执行 add，已停用时执行 enable。 */
function skillEnableAction(cell: SkillAgentCell | null): "add" | "enable" | null {
  const actions = cell?.actions ?? [];
  if (actions.includes("enable")) return "enable";
  if (cell?.state === "not-assigned" && actions.includes("add")) return "add";
  return null;
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
  const profilesQuery = useEngineQuery<StatusData>("profiles:status");
  const policyQuery = useEngineQuery<PolicyData>("policy:get");
  const [activeTab, setActiveTab] = useState<"collections" | "skills">(() => (initialAgent ? "skills" : "collections"));
  const [collectionFilter, setCollectionFilter] = useState<string>("all");
  const [agentFilter, setAgentFilter] = useState<string>(() => initialAgent ?? "all");
  const [search, setSearch] = useState("");
  const [showAddForm, setShowAddForm] = useState(false);
  const [editing, setEditing] = useState<EngineElement | null>(null);

  const directApply = useDirectApply();

  const queryClient = useQueryClient();
  const batchApply = useBatchApply();
  const definitionOps = useDefinitionOps({ section: "skills", domain: "skills" });

  /** 全部技能矩阵勾选集合：元素为 `${skill.id}-${agent}` 行键；仅可见且可执行的行保留在集合内。 */
  const [selectedKeys, setSelectedKeys] = useState<ReadonlySet<string>>(new Set());

  const [pendingRemove, setPendingRemove] = useState<EngineElement | null>(null);
  /** 计划外链接的外部链接接管确认：add=纳入管理、remove=取消链接，确认后携带 --allow-takeover。 */
  const [pendingTakeover, setPendingTakeover] = useState<
    { element: EngineElement; agent: string; action: "add" | "remove" } | null
  >(null);
  /** 批量 纳入管理/取消链接 的汇总确认：含外部链接（managed=false）的行先统一确认一次。 */
  const [pendingBatch, setPendingBatch] = useState<
    { action: BatchSkillAction; actionLabel: string; rows: BatchApplyRow[]; unmanagedCount: number } | null
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

  // 数据计算容忍查询未就绪（空数组兜底），选择剔除 effect 必须在下方早退之前声明。
  const elements = query.data?.data.elements ?? [];
  const collections = elements.filter(isCollection);
  const skills = elements.filter((element) => !isCollection(element));

  const layers = policyQuery.data?.data.layers ?? null;

  // 助手列来源：agents:status、集合声明的 agents、技能已报告的 cell。
  // 不取 policy 层的 agents 结构——其中混有 definitions 等保留字段，不能当助手 ID 列表。
  const supportedAgents = new Set([
    "claude-code",
    "codex",
    ...(agentsQuery.data?.data?.elements ?? [])
      .filter((element) => supportedAgentKindOf(element) !== null)
      .map((element) => element.id.replace(/^agents:/, "")),
    ...(profilesQuery.data?.ok ? profilesQuery.data.data.elements : [])
      .filter((element) => {
        const kind = (element.observed as { kind?: unknown }).kind;
        return kind === "claude" || kind === "codex";
      })
      .map((element) => element.id.replace(/^profiles:/, "")),
  ]);
  const statusAgents = Array.from(supportedAgents);
  const collectionAgents = collections.flatMap((col) => (col.spec as SkillCollectionSpec).agents ?? []);
  const skillAgents = skills.flatMap((element) => Object.keys(agentCellsOf(element)));

  const agentColumns = Array.from(new Set([...statusAgents, ...collectionAgents, ...skillAgents]))
    .filter((agent) => Boolean(agent) && supportedAgents.has(agent))
    .sort();

  const collName = (collection: EngineElement) => collection.id.replace(/^skill:/, "");

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

  /** 矩阵行：一行一个技能，每个助手一格，格内直接展示状态与操作。 */
  const matrixRows: MatrixRow[] = filteredSkills.map((skill) => {
    const spec = skill.spec as SkillSpec;
    const cells = (skill.observed as SkillObserved)?.agents ?? {};
    const rowCells: Record<string, MatrixRowCell | null> = {};
    let anyActionable = false;
    for (const agent of relevantAgents) {
      const cell = cells[agent];
      if (!cell) {
        rowCells[agent] = null;
        continue;
      }
      const actions = cell.actions ?? [];
      if (isBatchSelectableCell(cell)) anyActionable = true;
      rowCells[agent] = {
        state: cell.state,
        // 版本与阻塞原因放悬停：单开一列多半整列「未获取」，白占宽度
        title: [
          `${agent}: ${cell.state}`,
          cell.version ? `当前版本 ${cell.version}` : null,
          cell.blocked_reason ?? null,
        ]
          .filter(Boolean)
          .join(" · "),
        hasActions: actions.length > 0,
      };
    }
    return {
      key: skill.id,
      label: spec.name ?? skill.id,
      sublabel: spec.collection ?? undefined,
      cells: rowCells,
      selectable: anyActionable,
    };
  });

  // 筛选/搜索变化或数据刷新后，剔除不再可见、或已无可执行动作的选中项。
  useEffect(() => {
    setSelectedKeys((prev) => {
      if (prev.size === 0) return prev;
      const selectableKeys = new Set(
        assignmentRows.filter((row) => isBatchSelectableCell(row.cell)).map((row) => row.key),
      );
      const next = new Set([...prev].filter((key) => selectableKeys.has(key)));
      return next.size === prev.size ? prev : next;
    });
  }, [assignmentRows]);

  if (query.isPending) return <PageLoading />;
  if (query.isError) return <PageError title="技能状态读取失败" error={query.error} />;

  /** 集合操作：按钮严格来自卡片动作，apply 携带精确 --only/--action；remove 需先经独立 ConfirmModal。 */
  const runCollectionAction = (collection: EngineElement, action: "update" | "enable" | "disable" | "remove", title: string) => {
    if (action === "remove") {
      setPendingRemove(collection);
      return;
    }
    void directApply
      .apply({ command: "skills:apply", label: title, args: ["--only", collection.id, "--action", action] })
      .then((envelope) => {
        // 拉完代码要顺手重查一次这个集合的最新提交，否则卡片还挂着「可更新」，
        // 看上去像没生效——本地已经对齐了，落后的只是那份检查记录。
        if (action !== "update" || !envelope?.ok) return;
        return runEngine("system:refresh-upstream", ["--only", collection.id])
          .catch(() => undefined)
          .finally(() => void queryClient.invalidateQueries({ queryKey: engineKeys.all }));
      });
  };

  const handleUpdateCollection = (c: EngineElement) =>
    runCollectionAction(c, "update", `更新集合 ${collName(c)}`);

  const handleToggleCollection = (c: EngineElement, action: "enable" | "disable") =>
    runCollectionAction(c, action, `${action === "enable" ? "启用" : "停用"}集合 ${collName(c)}`);

  const handleRemoveCollection = (c: EngineElement) =>
    runCollectionAction(c, "remove", `删除集合 ${collName(c)}`);

  /** 技能 × 助手 操作：apply 携带精确 --only/--action/--agent。
   * 计划外链接（extra-link）行：外部链接（managed=false）的纳入管理/取消链接先经确认，
   * 确认后携带 --allow-takeover；受管残留链接（managed=true）直接执行不弹确认。 */
  const handleSkillAction = (skill: EngineElement, agent: string, action: string, cell: SkillAgentCell | null) => {
    const isExtraLinkAction = cell?.state === "extra-link" && (action === "add" || action === "remove");
    if (isExtraLinkAction && cell?.managed === false) {
      setPendingTakeover({ element: skill, agent, action: action as "add" | "remove" });
      return;
    }
    const spec = skill.spec as SkillSpec;
    const skillName = spec.name ?? skill.id;
    const labelMap: Record<string, string> = {
      add: "添加分配",
      update: "更新",
      enable: "启用",
      disable: "停用",
    };
    const extraLinkLabelMap: Record<string, string> = { add: "纳入管理", remove: "取消链接" };
    const actionLabel = isExtraLinkAction ? (extraLinkLabelMap[action] ?? action) : (labelMap[action] ?? action);
    const label = `${actionLabel} ${skillName} @ ${agent}`;
    void directApply.apply({
      command: "skills:apply",
      label,
      args: ["--only", skill.id, "--action", action, "--agent", agent],
    });
  };

  /** 批量派生：一个技能行勾选后，按每个助手当前真正可执行的动作分组。 */
  const selectableRows = assignmentRows.filter((row) => isBatchSelectableCell(row.cell));
  const selectedRows = assignmentRows.filter((row) => selectedKeys.has(row.key));
  const batchGroups: { action: BatchSkillAction; label: string; rows: typeof selectedRows }[] = [
    { action: "update", label: "更新", rows: selectedRows.filter((row) => skillNeedsUpdate(row.cell)) },
    { action: "enable", label: "启用", rows: selectedRows.filter((row) => skillEnableAction(row.cell)) },
    { action: "disable", label: "停用", rows: selectedRows.filter((row) => row.cell?.actions?.includes("disable")) },
    {
      action: "add",
      label: "纳入管理",
      rows: selectedRows.filter((row) => row.cell?.state === "extra-link" && row.cell?.actions?.includes("add")),
    },
    {
      action: "remove",
      label: "取消链接",
      rows: selectedRows.filter((row) => row.cell?.state === "extra-link" && row.cell?.actions?.includes("remove")),
    },
  ];
  const selectedSkillCount = new Set(selectedRows.map((row) => row.skill.id)).size;
  const allVisibleSelected = selectableRows.length > 0 && selectableRows.every((row) => selectedKeys.has(row.key));

  const toggleSelectAllVisible = () => {
    setSelectedKeys((prev) => {
      const next = new Set(prev);
      for (const row of selectableRows) {
        if (allVisibleSelected) next.delete(row.key);
        else next.add(row.key);
      }
      return next;
    });
  };

  /** 批量执行结束后清空选择；数据刷新由各行的执行通道统一触发。 */
  const runBatch = async (actionLabel: string, rows: BatchApplyRow[]) => {
    await batchApply.applyBatch({ actionLabel, rows });
    setSelectedKeys(new Set());
  };

  /** 批量入口：界面启用按行映射为 add/enable；计划外链接操作含外部链接时先汇总确认一次。 */
  const handleBatchAction = (action: BatchSkillAction, actionLabel: string, rows: typeof selectedRows) => {
    if (batchApply.executing || rows.length === 0) return;
    const batchRows: BatchApplyRow[] = rows.map((row) => {
      const applyAction = action === "enable" ? (skillEnableAction(row.cell) ?? action) : action;
      return {
        elementId: row.skill.id,
        agent: row.agent,
        action: applyAction,
        label: `${row.skillName} @ ${row.agent}`,
        takeover:
          row.cell?.state === "extra-link" &&
          (applyAction === "add" || applyAction === "remove") &&
          row.cell?.managed === false,
      };
    });
    if (action === "add" || action === "remove") {
      const unmanagedCount = batchRows.filter((row) => row.takeover).length;
      if (unmanagedCount > 0) {
        setPendingBatch({ action, actionLabel, rows: batchRows, unmanagedCount });
        return;
      }
    }
    void runBatch(actionLabel, batchRows);
  };

  const confirmBatch = () => {
    if (!pendingBatch) return;
    const { actionLabel, rows } = pendingBatch;
    setPendingBatch(null);
    void runBatch(actionLabel, rows);
  };

  // 空态引导：有集合但全未拉取时指向「更新资源库」；其余（含无集合、筛选无结果、
  // 已拉取但发现失败/无技能）保持通用文案，发现失败由集合卡片的 discovery_error 呈现。
  const allCollectionsUncached =
    collections.length > 0 &&
    collections.every((col) => (col.observed as SkillCollectionObserved).repo_cached === false);

  const confirmRemove = () => {
    if (!pendingRemove) return;
    const name = collName(pendingRemove);
    void directApply.apply({
      command: "skills:apply",
      label: `删除集合 ${name}`,
      args: ["--only", pendingRemove.id, "--action", "remove"],
    });
    setPendingRemove(null);
  };

  /** 接管确认后执行：apply 在原范围上追加 --allow-takeover（引擎侧仍对真实文件/目录 fail-closed）。 */
  const confirmTakeover = () => {
    if (!pendingTakeover) return;
    const spec = pendingTakeover.element.spec as SkillSpec;
    const skillName = spec.name ?? pendingTakeover.element.id;
    const actionLabel = pendingTakeover.action === "add" ? "纳入管理" : "取消链接";
    void directApply.apply({
      command: "skills:apply",
      label: `${actionLabel} ${skillName} @ ${pendingTakeover.agent}`,
      args: [
        "--only",
        pendingTakeover.element.id,
        "--action",
        pendingTakeover.action,
        "--agent",
        pendingTakeover.agent,
        "--allow-takeover",
      ],
    });
    setPendingTakeover(null);
  };

  return (
    <div className="space-y-6">
      <PageHeader title="技能" description="管理技能集合与技能分发。支持查看集合资源库状态，并在全部技能列表中搜索、按集合与助手筛选操作。" />

      {query.data.errors.length > 0 && <ErrorPanel title="技能状态读取失败" errors={query.data.errors} />}

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

            <span className="text-xs text-muted-foreground">
              当前展示 {filteredSkills.length} 个技能 × {relevantAgents.length} 个助手
            </span>
          </div>

          {selectedRows.length > 0 && (
            <div
              data-testid="batch-toolbar"
              className="flex flex-wrap items-center gap-3 rounded-lg border bg-card p-3 shadow-sm"
            >
              <span className="text-xs font-medium text-foreground">已选 {selectedSkillCount} 个技能</span>
              <div className="flex flex-wrap items-center gap-1.5">
                {batchGroups
                  .filter(({ action, rows }) => ["update", "enable", "disable"].includes(action) || rows.length > 0)
                  .map(({ action, label, rows }) => (
                    <button
                      key={action}
                      type="button"
                      data-testid={`batch-${action}`}
                      disabled={batchApply.executing || rows.length === 0}
                      title={rows.length === 0 ? `所选技能中没有可${label}的助手分配` : undefined}
                      onClick={() => handleBatchAction(action, label, rows)}
                      className={cn("rounded border px-2 py-0.5 text-xs disabled:opacity-40", BATCH_BUTTON_CLASS[action])}
                    >
                      批量{label} ({rows.length})
                    </button>
                  ))}
              </div>
              <button
                type="button"
                data-testid="batch-clear"
                disabled={batchApply.executing}
                onClick={() => setSelectedKeys(new Set())}
                className="ml-auto inline-flex items-center gap-1 rounded border px-2 py-1 text-xs text-muted-foreground hover:bg-accent disabled:opacity-40"
              >
                清除选择
              </button>
            </div>
          )}

          <ResourceAgentMatrix
            rows={matrixRows}
            agents={relevantAgents}
            kind="skills"
            resourceHeader="技能"
            isRowSelected={(key) => {
              // 行选中 = 该技能所有可执行的「技能×助手」格都已选中
              const keys = assignmentRows
                .filter((row) => row.skill.id === key && isBatchSelectableCell(row.cell))
                .map((row) => row.key);
              return keys.length > 0 && keys.every((k) => selectedKeys.has(k));
            }}
            allSelected={allVisibleSelected}
            someSelected={selectableRows.some((row) => selectedKeys.has(row.key))}
            selectDisabled={batchApply.executing}
            onToggleSelect={(key) => {
              // 选中一行 = 选中它所有可执行的格子，批量动作照旧按格子发
              const keys = assignmentRows
                .filter((row) => row.skill.id === key && isBatchSelectableCell(row.cell))
                .map((row) => row.key);
              setSelectedKeys((prev) => {
                const next = new Set(prev);
                const allOn = keys.every((k) => next.has(k));
                keys.forEach((k) => (allOn ? next.delete(k) : next.add(k)));
                return next;
              });
            }}
            onToggleSelectAll={toggleSelectAllVisible}
            renderCell={(row, agent, cell) => {
              const assignment = assignmentRows.find((item) => item.skill.id === row.key && item.agent === agent);
              if (!assignment) return null;
              return (
                <SkillAgentCellActions
                  skill={assignment.skill}
                  assignment={{ agent: assignment.agent, cell: assignment.cell }}
                  statusTitle={cell.title}
                  onAction={handleSkillAction}
                />
              );
            }}
            emptyHint={
              search.trim() ? (
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
                      ? "先更新集合资源库，拉取成功后技能会出现在这里"
                      : undefined
                  }
                  actions={
                    allCollectionsUncached ? (
                      <button
                        type="button"
                        data-testid="open-collections-to-update"
                        onClick={() => setActiveTab("collections")}
                        className="inline-flex items-center gap-1 rounded border border-primary/30 bg-primary/10 px-2.5 py-1 text-xs font-medium text-primary hover:bg-primary/15"
                      >
                        去更新资源库
                      </button>
                    ) : undefined
                  }
                />
              )
            }
          />
        </div>
      )}

      {(showAddForm || editing) && (
        <SkillCollectionFormDialog
          agentIds={agentColumns}
          initial={editing ? { name: editing.id.replace(/^skill:/, ""), definition: editing.spec } : undefined}
          fullDefinition={editing ? fullSkillDefinition(layers, editing.id.replace(/^skill:/, "")) : null}
          discovered={editing ? (editing.observed as SkillCollectionObserved).discovered : undefined}
          repoCached={editing ? (editing.observed as SkillCollectionObserved).repo_cached : undefined}
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
          title={`删除集合 ${collName(pendingRemove)}`}
          description="删除集合会清理已分配给各助手的入口，源目录保留。确认后执行。"
          risk="install-remove"
          requireText={collName(pendingRemove)}
          confirmLabel="确认删除"
          executing={directApply.executing}
          onConfirm={confirmRemove}
          onClose={() => setPendingRemove(null)}
        />
      )}

      {pendingTakeover && (
        <ConfirmModal
          title={`${pendingTakeover.action === "add" ? "纳入管理" : "取消链接"} ${(pendingTakeover.element.spec as SkillSpec).name ?? pendingTakeover.element.id} @ ${pendingTakeover.agent}`}
          description={`目标位置存在外部链接内容，确认后将被${pendingTakeover.action === "add" ? "替换" : "移除"}（此操作会修改你的文件系统）。`}
          risk="write-local"
          confirmLabel={pendingTakeover.action === "add" ? "确认纳入管理" : "确认取消链接"}
          executing={directApply.executing}
          onConfirm={confirmTakeover}
          onClose={() => setPendingTakeover(null)}
        />
      )}

      {pendingBatch && (
        <ConfirmModal
          title={`批量${pendingBatch.actionLabel}（${pendingBatch.rows.length} 行）`}
          description={`将对 ${pendingBatch.rows.length} 行执行${pendingBatch.actionLabel}。其中 ${pendingBatch.unmanagedCount} 行存在外部链接内容，确认后将被${pendingBatch.action === "add" ? "替换" : "移除"}（此操作会修改你的文件系统）。`}
          risk="write-local"
          confirmLabel={`确认${pendingBatch.actionLabel}`}
          executing={batchApply.executing}
          onConfirm={confirmBatch}
          onClose={() => setPendingBatch(null)}
        />
      )}
    </div>
  );
}
