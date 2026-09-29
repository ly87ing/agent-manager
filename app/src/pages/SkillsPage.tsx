import { useEffect, useState } from "react";
import { ChevronRight, PackageOpen, Pencil, Plus, RefreshCw, SearchX, Trash2, Wand2 } from "lucide-react";
import { toast } from "sonner";
import { useQueryClient } from "@tanstack/react-query";
import { ConfirmModal } from "../components/ConfirmModal";
import { EmptyState } from "../components/EmptyState";
import { ErrorPanel } from "../components/ErrorPanel";
import { PageError, PageLoading } from "../components/PageState";
import { SearchInput, matchesKeyword } from "../components/SearchInput";
import { SkillAgentCellActions } from "../components/SkillAgentActions";
import { SkillCollectionFormDialog } from "../components/SkillCollectionFormDialog";
import { SkillInstalledView } from "../components/SkillInstalledView";
import { SkillRecommendations } from "../components/SkillRecommendations";
import type { EngineElement, PolicyData, PolicyLayers, SkillRecommendation, StatusData } from "../lib/contract";
import { runEngine } from "../lib/engine";
import {
  agentCellsOf,
  type SkillAgentCell,
  type SkillCollectionObserved,
  type SkillCollectionSpec,
  type SkillObserved,
  type SkillSpec,
} from "../lib/observed";
import { engineKeys, useEngineQuery } from "../lib/queries";
import { addToAllRows, fillRows } from "../lib/skillFill";
import { useBatchApply } from "../lib/useBatchApply";
import { useAccountNames } from "../lib/useAccountNames";
import { useDefinitionOps } from "../lib/useDefinitionOps";
import { useDirectApply } from "../lib/useDirectApply";
import { cn } from "../lib/utils";

function isCollection(element: EngineElement): boolean {
  return element.id.startsWith("skill:") && !element.id.includes("/");
}

function fullSkillDefinition(layers: PolicyLayers | null, name: string): Record<string, unknown> | null {
  const repositories = (layers?.sqlite as { skills?: { repositories?: Record<string, unknown> } } | null | undefined)?.skills?.repositories;
  const definition = repositories?.[name];
  return typeof definition === "object" && definition !== null ? (definition as Record<string, unknown>) : null;
}

const ACTIVE = new Set(["linked", "missing-link", "dead-link", "drifted"]);
const TONE = {
  ok: "text-muted-foreground",
  warn: "bg-warning/15 text-warning",
  info: "bg-info/10 text-info",
} as const;

/** 一个技能在各账号上的情况，压成一行：全部账号 / 某某除外 / 新发现，外加需要处理的标签。 */
function summarize(skill: EngineElement, cells: Record<string, SkillAgentCell>, nameOf: (agent: string) => string) {
  const entries = Object.entries(cells);
  const off = entries.filter(([, cell]) => !ACTIVE.has(cell.state) && cell.state !== "extra-link").map(([agent]) => nameOf(agent));
  const anyActive = entries.some(([, cell]) => ACTIVE.has(cell.state));
  const badges: { text: string; tone: keyof typeof TONE }[] = [];
  if (entries.some(([, cell]) => cell.source_missing)) badges.push({ text: "来源里已删除", tone: "warn" });
  if (entries.some(([, cell]) => cell.state === "missing-link" || cell.state === "dead-link")) badges.push({ text: "链接损坏", tone: "warn" });
  if (entries.some(([, cell]) => cell.state === "drifted")) badges.push({ text: "入口被外部内容占用", tone: "warn" });
  if (entries.some(([, cell]) => cell.state === "extra-link")) badges.push({ text: "有外部链接", tone: "info" });
  const inbox = (skill.observed as SkillObserved).discovery_inbox === true;
  const text = inbox
    ? "新发现"
    : !anyActive
      ? "未启用"
      : off.length === 0
        ? "全部账号"
        : `全部账号（${off.join("、")} 除外）`;
  return { text, badges, inbox };
}

export default function SkillsPage({
  initialAgent,
  initialSearch,
  onResetAgent,
}: {
  /** 从账号页进来：只看这个账号。 */
  initialAgent?: string | null;
  /** 从总览「去处理」进来时直接搜到那个技能。 */
  initialSearch?: string | null;
  onResetAgent?: () => void;
} = {}) {
  const query = useEngineQuery<StatusData>("skills:status");
  const nameOf = useAccountNames();
  const policyQuery = useEngineQuery<PolicyData>("policy:get");
  const [collectionFilter, setCollectionFilter] = useState<string>("all");
  const [agentFilter, setAgentFilter] = useState<string>(() => initialAgent ?? "all");
  const [search, setSearch] = useState(initialSearch ?? "");
  /** 用户点过展开/收起的行；搜索中或有待处理问题的行默认展开，点一下反过来。 */
  const [toggled, setToggled] = useState<ReadonlySet<string>>(() => new Set());
  const [showAddForm, setShowAddForm] = useState(false);
  const [editing, setEditing] = useState<EngineElement | null>(null);
  const [pendingRemove, setPendingRemove] = useState<EngineElement | null>(null);
  const [pendingRecommendation, setPendingRecommendation] = useState<SkillRecommendation | null>(null);
  const [pendingTakeover, setPendingTakeover] = useState<{ element: EngineElement; agent: string; action: "add" | "remove" } | null>(null);
  const [pendingDropped, setPendingDropped] = useState<{ element: EngineElement; agent: string; cell: SkillAgentCell } | null>(null);

  const directApply = useDirectApply();
  const batchApply = useBatchApply();
  const queryClient = useQueryClient();
  const definitionOps = useDefinitionOps({ section: "skills", domain: "skills" });

  useEffect(() => {
    setAgentFilter(initialAgent ?? "all");
  }, [initialAgent]);

  if (query.isPending) return <PageLoading />;
  if (query.isError) return <PageError title="技能状态读取失败" error={query.error} />;

  const elements = query.data.data.elements ?? [];
  const recommendations = query.data.data.recommendations ?? [];
  const installed = query.data.data.installed ?? [];
  const collections = elements.filter(isCollection);
  const skills = elements.filter((element) => !isCollection(element));
  const layers = policyQuery.data?.data.layers ?? null;

  const agentOptions = Array.from(new Set(skills.flatMap((skill) => Object.keys(agentCellsOf(skill))))).sort();

  const collName = (collection: EngineElement) => collection.id.replace(/^skill:/, "");
  const collectionMatches = (collection: EngineElement) => {
    const spec = collection.spec as SkillCollectionSpec;
    return Boolean(search.trim()) && matchesKeyword(search, [collName(collection), spec.url ?? undefined, spec.source_path ?? undefined]);
  };
  /** 搜索命中集合名或地址时整组显示；否则只留名字命中的技能。 */
  const skillsOf = (collection: EngineElement) =>
    skills.filter((skill) => {
      const spec = skill.spec as SkillSpec;
      return spec.collection === collName(collection) && (collectionMatches(collection) || matchesKeyword(search, [spec.name, skill.id]));
    });
  const visibleCollections = collections.filter((collection) =>
    (collectionFilter === "all" || collName(collection) === collectionFilter)
    && (!search.trim() || collectionMatches(collection) || skillsOf(collection).length > 0));
  const pendingFill = fillRows(elements, agentFilter === "all" ? undefined : [agentFilter]);

  const refreshUpstream = (collection: EngineElement) =>
    runEngine("system:refresh-upstream", ["--only", collection.id])
      .catch(() => undefined)
      .finally(() => void queryClient.invalidateQueries({ queryKey: engineKeys.all }));

  const runCollectionAction = (collection: EngineElement, action: "update" | "enable" | "disable", title: string) =>
    void directApply
      .apply({ command: "skills:apply", label: title, args: ["--only", collection.id, "--action", action] })
      .then((envelope) => {
        // 拉完代码顺手重查一次最新提交，否则还挂着「可更新」
        if (action === "update" && envelope?.ok) return refreshUpstream(collection);
      });

  /** 单个账号上的动作：外部链接接管、移除来源里已删除的技能先确认，其余直接执行。 */
  const handleSkillAction = (skill: EngineElement, agent: string, action: string, cell: SkillAgentCell | null) => {
    if (cell?.source_missing && action === "remove") {
      setPendingDropped({ element: skill, agent, cell });
      return;
    }
    const isExtraLinkAction = cell?.state === "extra-link" && (action === "add" || action === "remove");
    if (isExtraLinkAction && cell?.managed === false) {
      setPendingTakeover({ element: skill, agent, action: action as "add" | "remove" });
      return;
    }
    const skillName = (skill.spec as SkillSpec).name ?? skill.id;
    const labels: Record<string, string> = isExtraLinkAction
      ? { add: "纳入管理", remove: "取消链接" }
      : { add: "启用", update: "更新", enable: "启用", disable: "停用" };
    void directApply.apply({
      command: "skills:apply",
      label: `${labels[action] ?? action} ${skillName} @ ${nameOf(agent)}`,
      args: ["--only", skill.id, "--action", action, "--agent", agent],
    });
  };

  const toggleExpanded = (id: string) =>
    setToggled((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });

  const confirmRemove = () => {
    if (!pendingRemove) return;
    void directApply.apply({
      command: "skills:apply",
      label: `删除集合 ${collName(pendingRemove)}`,
      args: ["--only", pendingRemove.id, "--action", "remove"],
    });
    setPendingRemove(null);
  };

  const confirmRecommendation = () => {
    if (!pendingRecommendation) return;
    const { name, definition } = pendingRecommendation;
    setPendingRecommendation(null);
    void definitionOps.saveDefinition({ name, definition, isNew: true }).then((ok) => {
      if (ok) toast.info(`集合 ${name} 已添加。点「更新」取回仓库后，在技能上点「添加」加到全部账号。`);
    });
  };

  const confirmDropped = () => {
    if (!pendingDropped) return;
    const { element, agent, cell } = pendingDropped;
    void directApply.apply({
      command: "skills:apply",
      label: `移除 ${(element.spec as SkillSpec).name ?? element.id} @ ${nameOf(agent)}`,
      args: ["--only", element.id, "--action", "remove", "--agent", agent, ...(cell.state === "drifted" ? ["--allow-takeover"] : [])],
    });
    setPendingDropped(null);
  };

  const confirmTakeover = () => {
    if (!pendingTakeover) return;
    const { element, agent, action } = pendingTakeover;
    void directApply.apply({
      command: "skills:apply",
      label: `${action === "add" ? "纳入管理" : "取消链接"} ${(element.spec as SkillSpec).name ?? element.id} @ ${nameOf(agent)}`,
      args: ["--only", element.id, "--action", action, "--agent", agent, "--allow-takeover"],
    });
    setPendingTakeover(null);
  };

  const shownSkillCount = visibleCollections.reduce((sum, collection) => sum + skillsOf(collection).length, 0);

  return (
    <div className="space-y-3">
      <h1 className="sr-only">技能</h1>
      {query.data.errors.length > 0 && <ErrorPanel title="技能状态读取失败" errors={query.data.errors} />}

      <div className="flex flex-wrap items-center gap-2">
        <SearchInput value={search} onChange={setSearch} placeholder="搜索技能…" testId="skills-search" />
        <select
          aria-label="集合筛选"
          data-testid="filter-collection-select"
          value={collectionFilter}
          onChange={(event) => setCollectionFilter(event.target.value)}
          className="rounded-md border bg-card px-2 py-1.5 text-sm"
        >
          <option value="all">全部集合</option>
          {collections.map((col) => (
            <option key={col.id} value={collName(col)}>{collName(col)}</option>
          ))}
        </select>
        <select
          aria-label="账号筛选"
          data-testid="filter-agent-select"
          value={agentFilter}
          onChange={(event) => {
            setAgentFilter(event.target.value);
            if (event.target.value === "all") onResetAgent?.();
          }}
          className="rounded-md border bg-card px-2 py-1.5 text-sm"
        >
          <option value="all">全部账号</option>
          {agentOptions.map((agent) => (
            <option key={agent} value={agent}>{nameOf(agent)}</option>
          ))}
        </select>
        <div className="ml-auto flex items-center gap-2">
          {pendingFill.length > 0 && (
            <button
              type="button"
              data-testid="skills-fill"
              disabled={batchApply.executing}
              title="把在用的技能链接到还缺它的账号；账号上停用的不动"
              onClick={() => void batchApply.applyBatch({ actionLabel: "补齐", rows: pendingFill })}
              className="inline-flex items-center gap-1 rounded-md border border-primary/30 bg-primary/10 px-3 py-1.5 text-sm text-primary hover:bg-primary/15 disabled:opacity-40"
            >
              <Wand2 className="h-4 w-4" aria-hidden /> 补齐 {pendingFill.length}
            </button>
          )}
          <button
            type="button"
            data-testid="add-collection-button"
            onClick={() => setShowAddForm(true)}
            className="inline-flex items-center gap-1 rounded-md bg-primary px-3 py-1.5 text-sm text-primary-foreground hover:bg-primary/90"
          >
            <Plus className="h-4 w-4" aria-hidden /> 添加集合
          </button>
        </div>
      </div>

      {collections.length === 0 ? (
        <EmptyState
          icon={PackageOpen}
          testId="skills-collections-empty"
          title="还没有技能集合"
          description="添加一个 git 仓库或本机目录，或从下面的推荐来源添加。"
        />
      ) : search.trim() && shownSkillCount === 0 ? (
        <EmptyState
          icon={SearchX}
          testId="skills-empty-state"
          title="无匹配结果"
          actions={
            <button type="button" onClick={() => setSearch("")} className="rounded border px-2 py-1 text-xs text-muted-foreground hover:bg-accent">
              清除搜索关键词
            </button>
          }
        />
      ) : (
        <div className="divide-y rounded-lg border bg-card">
          {visibleCollections.map((collection) => {
            const name = collName(collection);
            const spec = collection.spec as SkillCollectionSpec;
            const observed = collection.observed as SkillCollectionObserved;
            const actions = observed.actions ?? [];
            const enabled = (collection.policy as { enabled?: boolean }).enabled !== false;
            const inboxCount = observed.discovery_inbox?.length ?? 0;
            const rows = skillsOf(collection);
            return (
              <section key={collection.id} data-testid={`collection-card-${name}`}>
                <header className="flex items-center gap-2 bg-muted/40 px-4 py-2">
                  <span className="font-medium">{name}</span>
                  <span className="truncate font-mono text-xs text-muted-foreground" title={spec.source_path ?? spec.url ?? undefined}>
                    {spec.source_path ?? spec.url ?? "（未填来源）"}
                  </span>
                  {!enabled && <span className="rounded bg-muted px-1.5 py-0.5 text-[11px] text-muted-foreground">已停用</span>}
                  {!observed.repo_cached && (
                    <span className="rounded bg-warning/15 px-1.5 py-0.5 text-[11px] text-warning">
                      {spec.source_path ? "本机目录不存在" : "尚未下载"}
                    </span>
                  )}
                  {collection.freshness.state === "behind" && (
                    <span className="rounded bg-info/10 px-1.5 py-0.5 text-[11px] text-info">可更新</span>
                  )}
                  {inboxCount > 0 && (
                    <span data-testid={`inbox-count-${name}`} className="rounded bg-info/10 px-1.5 py-0.5 text-[11px] text-info">
                      新发现 {inboxCount}
                    </span>
                  )}
                  <div className="ml-auto flex items-center gap-1">
                    {actions.includes("update") && (
                      <button
                        type="button"
                        data-testid={`update-collection-${name}`}
                        onClick={() => runCollectionAction(collection, "update", `更新集合 ${name}`)}
                        className="inline-flex items-center gap-1 rounded px-2 py-1 text-xs text-primary hover:bg-primary/10"
                      >
                        <RefreshCw className="h-3.5 w-3.5" aria-hidden /> 更新
                      </button>
                    )}
                    <button
                      type="button"
                      data-testid={`edit-collection-${name}`}
                      aria-label={`编辑集合 ${name}`}
                      title="编辑定义"
                      onClick={() => setEditing(collection)}
                      className="rounded p-1.5 text-muted-foreground hover:bg-accent"
                    >
                      <Pencil className="h-3.5 w-3.5" aria-hidden />
                    </button>
                    {actions.includes("enable") && !enabled && (
                      <button type="button" data-testid={`enable-collection-${name}`} onClick={() => runCollectionAction(collection, "enable", `启用集合 ${name}`)} className="rounded px-2 py-1 text-xs text-success hover:bg-success/10">
                        启用
                      </button>
                    )}
                    {actions.includes("disable") && enabled && (
                      <button type="button" data-testid={`disable-collection-${name}`} onClick={() => runCollectionAction(collection, "disable", `停用集合 ${name}`)} className="rounded px-2 py-1 text-xs text-muted-foreground hover:bg-accent">
                        停用
                      </button>
                    )}
                    {actions.includes("remove") && (
                      <button
                        type="button"
                        data-testid={`remove-collection-${name}`}
                        aria-label={`删除集合 ${name}`}
                        title="删除集合"
                        onClick={() => setPendingRemove(collection)}
                        className="rounded p-1.5 text-muted-foreground hover:bg-destructive/10 hover:text-destructive"
                      >
                        <Trash2 className="h-3.5 w-3.5" aria-hidden />
                      </button>
                    )}
                  </div>
                </header>
                {observed.discovery_error && (
                  <p data-testid={`discovery-error-${name}`} className="bg-warning/10 px-4 py-1.5 text-xs text-warning">
                    {observed.discovery_error.message ?? "技能发现失败"}
                  </p>
                )}
                {observed.blocked_reason && <p className="bg-warning/10 px-4 py-1.5 text-xs text-warning">{observed.blocked_reason}</p>}
                {rows.length === 0 && !search.trim() && (
                  <p className="px-4 py-2 text-xs text-muted-foreground">
                    {observed.repo_cached ? "没有找到技能" : "点「更新」取回后技能会出现在这里"}
                  </p>
                )}
                <ul className="divide-y">
                  {rows.map((skill) => {
                    const allCells = (skill.observed as SkillObserved).agents ?? {};
                    const cells = agentFilter === "all"
                      ? allCells
                      : Object.fromEntries(Object.entries(allCells).filter(([agent]) => agent === agentFilter));
                    const summary = summarize(skill, cells, nameOf);
                    const addRows = summary.inbox ? addToAllRows(skill) : [];
                    const autoOpen = Boolean(search.trim()) || summary.badges.length > 0;
                    const isOpen = autoOpen !== toggled.has(skill.id);
                    const single = agentFilter !== "all" ? Object.entries(cells)[0] : undefined;
                    const skillName = (skill.spec as SkillSpec).name ?? skill.id;
                    return (
                      <li key={skill.id} data-testid={`skill-row-${skill.id}`}>
                        <div className="flex items-center gap-2 px-4 py-2">
                          <button
                            type="button"
                            aria-expanded={isOpen}
                            aria-label={`${skillName} 在各账号上的情况`}
                            onClick={() => toggleExpanded(skill.id)}
                            className="rounded p-0.5 text-muted-foreground hover:bg-accent"
                          >
                            <ChevronRight className={cn("h-4 w-4 transition-transform", isOpen && "rotate-90")} aria-hidden />
                          </button>
                          <span className="min-w-0 truncate text-sm">{skillName}</span>
                          {summary.badges.map((badge) => (
                            <span key={badge.text} className={cn("rounded px-1.5 py-0.5 text-[11px]", TONE[badge.tone])}>{badge.text}</span>
                          ))}
                          {single ? (
                            <span className="ml-auto">
                              <SkillAgentCellActions skill={skill} assignment={{ agent: single[0], cell: single[1] }} statusTitle={`${single[0]}: ${single[1].state}`} onAction={handleSkillAction} />
                            </span>
                          ) : (
                            <span data-testid={`skill-summary-${skill.id}`} className={cn("ml-auto shrink-0 text-xs", summary.inbox ? "text-info" : "text-muted-foreground")}>
                              {summary.text}
                            </span>
                          )}
                          {addRows.length > 0 && (
                            <button
                              type="button"
                              data-testid={`skill-add-all-${skill.id}`}
                              disabled={batchApply.executing}
                              onClick={() => void batchApply.applyBatch({ actionLabel: "添加", rows: addRows })}
                              className="rounded-md border border-primary/30 bg-primary/10 px-2 py-0.5 text-xs text-primary hover:bg-primary/15 disabled:opacity-40"
                            >
                              添加
                            </button>
                          )}
                        </div>
                        {isOpen && !single && (
                          <ul className="space-y-1 bg-muted/20 px-11 pb-2 pt-1">
                            {Object.entries(cells).map(([agent, cell]) => (
                              <li key={agent} className="flex items-center justify-between gap-2 text-xs">
                                <span>{nameOf(agent)}</span>
                                <SkillAgentCellActions skill={skill} assignment={{ agent, cell }} statusTitle={`${agent}: ${cell.state}`} onAction={handleSkillAction} />
                              </li>
                            ))}
                          </ul>
                        )}
                      </li>
                    );
                  })}
                </ul>
              </section>
            );
          })}
        </div>
      )}

      <details className="rounded-lg border bg-card">
        <summary className="cursor-pointer px-4 py-2 text-sm text-muted-foreground">各账号技能目录里的实际内容</summary>
        <div className="border-t p-3">
          <SkillInstalledView entries={installed} agents={agentOptions} />
        </div>
      </details>
      {recommendations.length > 0 && <SkillRecommendations items={recommendations} onAdd={setPendingRecommendation} />}

      {(showAddForm || editing) && (
        <SkillCollectionFormDialog
          agentIds={agentOptions}
          initial={editing ? { name: collName(editing), definition: editing.spec } : undefined}
          fullDefinition={editing ? fullSkillDefinition(layers, collName(editing)) : null}
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
            void definitionOps.saveDefinition({ name, definition, isNew, complete }).then(() => {
              if (isNew) toast.info(`集合 ${name} 已保存，点「更新」取回技能。`);
            });
          }}
        />
      )}
      {definitionOps.modals}
      {pendingRemove && (
        <ConfirmModal
          title={`删除集合 ${collName(pendingRemove)}`}
          description="删除集合会清理已分配给各账号的入口，源目录保留。确认后执行。"
          risk="install-remove"
          requireText={collName(pendingRemove)}
          confirmLabel="确认删除"
          executing={directApply.executing}
          onConfirm={confirmRemove}
          onClose={() => setPendingRemove(null)}
        />
      )}
      {pendingRecommendation && (
        <ConfirmModal
          title={`添加推荐来源 ${pendingRecommendation.name}`}
          description={`只保存集合定义：仓库 ${pendingRecommendation.url}，技能取自 ${pendingRecommendation.skill_paths.map((path) => `${path}/`).join("、")}。不会下载，也不会加到任何账号；保存后点「更新」取回仓库，再在技能上点「添加」。`}
          risk="write-local"
          confirmLabel="添加"
          executing={false}
          onConfirm={confirmRecommendation}
          onClose={() => setPendingRecommendation(null)}
        />
      )}
      {pendingDropped && (
        <ConfirmModal
          title={`移除 ${(pendingDropped.element.spec as SkillSpec).name ?? pendingDropped.element.id} @ ${nameOf(pendingDropped.agent)}`}
          description={`来源里已经没有这个技能，删掉它在 ${nameOf(pendingDropped.agent)} 上的分配记录${
            pendingDropped.cell.state === "drifted"
              ? "；入口现在是本工具以外的链接，会一并删除这条链接"
              : pendingDropped.cell.state === "missing-link"
                ? "（入口已经不在了，不改动任何文件）"
                : "，并删除本工具建的入口链接"
          }。不影响其他账号。`}
          risk="write-local"
          confirmLabel="确认移除"
          executing={directApply.executing}
          onConfirm={confirmDropped}
          onClose={() => setPendingDropped(null)}
        />
      )}
      {pendingTakeover && (
        <ConfirmModal
          title={`${pendingTakeover.action === "add" ? "纳入管理" : "取消链接"} ${(pendingTakeover.element.spec as SkillSpec).name ?? pendingTakeover.element.id} @ ${nameOf(pendingTakeover.agent)}`}
          description={`目标位置存在外部链接内容，确认后将被${pendingTakeover.action === "add" ? "替换" : "移除"}（此操作会修改你的文件系统）。`}
          risk="write-local"
          confirmLabel={pendingTakeover.action === "add" ? "确认纳入管理" : "确认取消链接"}
          executing={directApply.executing}
          onConfirm={confirmTakeover}
          onClose={() => setPendingTakeover(null)}
        />
      )}
    </div>
  );
}
