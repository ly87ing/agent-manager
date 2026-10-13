import type { CSSProperties } from "react";
import { useEffect, useState } from "react";
import { ChevronDown, ChevronRight, ChevronsDownUp, ChevronsUpDown, Library, PackageOpen, SearchX } from "lucide-react";
import { toast } from "sonner";
import { useQueryClient } from "@tanstack/react-query";
import { ConfirmModal } from "../components/ConfirmModal";
import { EmptyState } from "../components/EmptyState";
import { ErrorPanel } from "../components/ErrorPanel";
import { PageError, PageLoading } from "../components/PageState";
import { SearchInput, matchesKeyword } from "../components/SearchInput";
import { AccountCells, AccountHeader, AccountSwitch, ActionMenu, RowActions, TRAILING } from "../components/AccountGrid";
import { skillChip, skillNeedsUpdate } from "../lib/skillChip";
import { SkillInstalledView } from "../components/SkillInstalledView";
import { AttentionOnlyToggle, StateBadge } from "../components/StateBadge";
import { skillItems } from "../lib/inbox";
import { DependencyChips, dependenciesOf } from "../components/DependencyChips";
import type { ProgramsData } from "../lib/programs";
import type { InboxState } from "../lib/inbox";
import { describeTrack, versionLabel } from "../lib/skillTrack";
import type { EngineElement, StatusData } from "../lib/contract";
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
import { useExecutions } from "../lib/execution";
import { isCollection } from "../lib/skillCollections";
import { ForkSkillDialog } from "../components/ForkSkillDialog";
import { useBatchApply } from "../lib/useBatchApply";
import { BatchResultPanel } from "../components/BatchResultPanel";
import { useAccountGroups, useAccountNames } from "../lib/useAccountNames";
import { useDirectApply } from "../lib/useDirectApply";
import { cn } from "../lib/utils";
import { Select } from "../components/Select";

const ACTIVE = new Set(["linked", "missing-link", "dead-link", "drifted"]);

/** 一个技能在各助手上的情况，压成一行：全部助手 / 某某除外 / 新发现，外加需要处理的标签。 */
function summarize(skill: EngineElement, cells: Record<string, SkillAgentCell>, nameOf: (agent: string) => string) {
  const entries = Object.entries(cells);
  const off = entries.filter(([, cell]) => !ACTIVE.has(cell.state) && cell.state !== "extra-link").map(([agent]) => nameOf(agent));
  const anyActive = entries.some(([, cell]) => ACTIVE.has(cell.state));
  // 统一状态（lib/inbox.ts）：损坏 / 已偏离 / 未纳管 / 有新版本，与待处理页同一套标签
  const badges: { text: string; state: InboxState }[] = [];
  if (entries.some(([, cell]) => cell.source_missing)) badges.push({ text: "来源里已删除", state: "broken" });
  if (entries.some(([, cell]) => cell.state === "missing-link" || cell.state === "dead-link")) badges.push({ text: "链接损坏", state: "broken" });
  if (entries.some(([, cell]) => cell.state === "drifted")) badges.push({ text: "入口被外部内容占用", state: "drifted" });
  if (entries.some(([, cell]) => cell.state === "extra-link")) badges.push({ text: "有外部链接", state: "unmanaged" });
  if (entries.some(([, cell]) => skillNeedsUpdate(cell))) badges.push({ text: "有新版本", state: "update" });
  const inbox = (skill.observed as SkillObserved).discovery_inbox === true;
  const text = inbox
    ? "新发现"
    : !anyActive
      ? "未启用"
      : off.length === 0
        ? "全部助手"
        : `全部助手（${off.join("、")} 除外）`;
  return { text, badges, inbox };
}

const EXPANDED_KEY = "agent-manager:skills-expanded";
/** 用户手动展开 / 收起过的集合，只存在本机浏览器里；读不到就按默认。 */
function loadExpanded(): Record<string, boolean> {
  try {
    const value = JSON.parse(localStorage.getItem(EXPANDED_KEY) ?? "{}");
    return typeof value === "object" && value !== null ? value : {};
  } catch {
    return {};
  }
}

function saveExpanded(value: Record<string, boolean>) {
  try {
    localStorage.setItem(EXPANDED_KEY, JSON.stringify(value));
  } catch {
    // 存不下只影响下次打开时的展开状态
  }
}

export default function SkillsPage({
  initialAgent,
  initialSearch,
  onResetAgent,
  onOpenSource,
  onOpenProgram,
}: {
  /** 依赖标签点过去：程序页的那一行（程序标识）。 */
  onOpenProgram?: (program: string) => void;
  /** 来源的添加、更新、换版本、纳入都在来源页：点「可更新」「尚未下载」等跳过去（名称；`unmanaged` 为未纳管）。 */
  onOpenSource?: (name: string | null) => void;
  /** 从助手页进来：只看这个助手。 */
  initialAgent?: string | null;
  /** 从总览「去处理」进来时直接搜到那个技能。 */
  initialSearch?: string | null;
  onResetAgent?: () => void;
} = {}) {
  const query = useEngineQuery<StatusData>("skills:status");
  const nameOf = useAccountNames();
  const groupsOf = useAccountGroups();
  const [collectionFilter, setCollectionFilter] = useState<string>("all");
  const [agentFilter, setAgentFilter] = useState<string>(() => initialAgent ?? "all");
  const [search, setSearch] = useState(initialSearch ?? "");
  const [expanded, setExpanded] = useState<Record<string, boolean>>(loadExpanded);
  const [pendingTakeover, setPendingTakeover] = useState<{ element: EngineElement; agent: string; action: "add" | "remove" } | null>(null);
  /** 正在「改成我的版本」的技能。 */
  const [forking, setForking] = useState<EngineElement | null>(null);
  const [forkBusy, setForkBusy] = useState(false);
  const [pendingDropped, setPendingDropped] = useState<{ element: EngineElement; agent: string; cell: SkillAgentCell } | null>(null);

  const directApply = useDirectApply();
  const batchApply = useBatchApply();
  // Derive attention from this page's status without querying other domains.
  const attentionSubjects = new Set(
    skillItems(query.data?.ok ? query.data.data.elements ?? [] : [], nameOf).map((item) => item.subject),
  );
  const [attentionOnly, setAttentionOnly] = useState(false);
  const programsQuery = useEngineQuery<ProgramsData>("programs:status");
  const programs = programsQuery.data?.ok ? programsQuery.data.data.elements : [];
  const { execute } = useExecutions();
  const queryClient = useQueryClient();

  useEffect(() => {
    setAgentFilter(initialAgent ?? "all");
  }, [initialAgent]);


  if (query.isPending) return <PageLoading />;
  if (query.isError) return <PageError title="技能状态读取失败" error={query.error} />;

  const elements = query.data.data.elements ?? [];
  const installed = query.data.data.installed ?? [];
  const unmanaged = query.data.data.unmanaged ?? null;
  const unmanagedCount = (unmanaged?.skills.length ?? 0) + (unmanaged?.duplicates.length ?? 0);
  const collections = elements.filter(isCollection);
  const skills = elements.filter((element) => !isCollection(element));

  // 其他来源里出现的助手也要有列，否则只装了别处技能的助手看不到
  const agentOptions = Array.from(
    new Set([
      ...skills.flatMap((skill) => Object.keys(agentCellsOf(skill))),
      ...(unmanaged?.skills ?? []).flatMap((skill) => skill.locations.flatMap((location) => (location.agent ? [location.agent] : []))),
      ...(unmanaged?.duplicates ?? []).map((dup) => dup.agent),
    ]),
  ).sort();

  const collName = (collection: EngineElement) => collection.id.replace(/^skill:/, "");
  const collectionMatches = (collection: EngineElement) => {
    const spec = collection.spec as SkillCollectionSpec;
    return Boolean(search.trim()) && matchesKeyword(search, [collName(collection), spec.url ?? undefined, spec.source_path ?? undefined]);
  };
  /** 搜索命中集合名或地址时整组显示；否则只留名字命中的技能。 */
  const skillsOf = (collection: EngineElement) =>
    skills.filter((skill) => {
      const spec = skill.spec as SkillSpec;
      return (
        spec.collection === collName(collection)
        && (!attentionOnly || attentionSubjects.has(skill.id))
        && (collectionMatches(collection) || matchesKeyword(search, [spec.name, skill.id]))
      );
    });
  const visibleCollections = collections.filter((collection) =>
    (collectionFilter === "all" || collName(collection) === collectionFilter)
    && ((!search.trim() && !attentionOnly) || (collectionMatches(collection) && !attentionOnly) || skillsOf(collection).length > 0));
  const pendingFill = fillRows(elements, agentFilter === "all" ? undefined : [agentFilter]);

  /**
   * 复制并登记后，逐个目标原子替换链接与分配；失败保留原版本。
   */
  const runFork = async (skill: EngineElement, input: { dest: string; target: string; switchAccounts: string[] }) => {
    const spec = skill.spec as SkillSpec;
    const name = spec.name ?? skill.id;
    setForkBusy(true);
    try {
      const forked = await execute({
        command: "skills:fork",
        label: `把 ${name} 改成我的版本`,
        args: ["--only", skill.id, "--dest", input.dest, "--collection", input.target],
        silent: true,
      });
      if (!forked?.ok) {
        toast.error(`没有复制：${forked?.errors[0]?.message ?? "详见任务记录"}`);
        return;
      }
      let failed = 0;
      for (const agent of input.switchAccounts) {
        const version = (skill.observed as SkillObserved).agents?.[agent]?.version;
        const added = version ? await execute({ command: "skills:apply", label: `切换技能版本 ${name} @ ${nameOf(agent)}`,
          args: ["--only", `skill:${input.target}/${name}`, "--action", "add", "--agent", agent,
            "--replace-from", skill.id, "--replace-version", version], silent: true }) : null;
        if (!added?.ok) failed += 1;
      }
      const path = (forked.data as { path?: string }).path ?? input.dest;
      (failed ? toast.error : toast.success)(
        failed ? `已复制到 ${path}，但 ${failed} 个助手没换过去，详见任务记录` : `已复制到 ${path}，${input.switchAccounts.length} 个助手换成了你的版本`,
      );
      setForking(null);
    } finally {
      setForkBusy(false);
      await queryClient.invalidateQueries({ queryKey: engineKeys.all });
    }
  };

  /** 单个助手上的动作：外部链接接管、移除来源里已删除的技能先确认，其余直接执行。 */
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
    // 能干净回到原状的才给撤销：启用 ↔ 停用；首次启用（add）撤销是取消分配，回到「未启用」
    const reverse: Record<string, string> = isExtraLinkAction ? {} : { enable: "disable", disable: "enable", add: "remove" };
    const undoAction = reverse[action];
    void directApply.apply({
      command: "skills:apply",
      label: `${labels[action] ?? action} ${skillName} @ ${nameOf(agent)}`,
      args: ["--only", skill.id, "--action", action, "--agent", agent],
      undo: undoAction
        ? () =>
            void directApply.apply({
              command: "skills:apply",
              label: `撤销：${labels[action] ?? action} ${skillName} @ ${nameOf(agent)}`,
              args: ["--only", skill.id, "--action", undoAction, "--agent", agent],
            })
        : undefined,
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

  const cellsIn = (skill: EngineElement) =>
    Object.entries((skill.observed as SkillObserved).agents ?? {}).filter(([agent]) => agentFilter === "all" || agent === agentFilter);
  /** 需要处理：待处理里有它（用户忽略的不算），且在所选助手上有黄色开关或是新发现。 */
  const needsAttention = (skill: EngineElement) =>
    attentionSubjects.has(skill.id)
    && ((skill.observed as SkillObserved).discovery_inbox === true || cellsIn(skill).some(([, cell]) => skillChip(cell).tone === "warn"));
  /** 搜索或只看一个集合时展开命中的；否则按用户上次的选择，没选过的有待处理或还没有技能时才展开。 */
  const isOpen = (name: string, rows: EngineElement[]) =>
    Boolean(search.trim()) || attentionOnly || collectionFilter === name || (expanded[name] ?? (rows.length === 0 || rows.some(needsAttention)));
  const setOpen = (next: Record<string, boolean>) => {
    const merged = { ...expanded, ...next };
    setExpanded(merged);
    saveExpanded(merged);
  };
  const allOpen = visibleCollections.every((collection) => isOpen(collName(collection), skillsOf(collection)));
  const shownAgents = agentFilter === "all" ? agentOptions : [agentFilter];
  const groups = groupsOf(shownAgents);
  /** 每个助手还缺哪些在用的技能：表头「补齐」逐个助手做。 */
  const fillOf = (agent: string) => pendingFill.filter((row) => row.agent === agent);

  return (
    <div className="space-y-3">
      {batchApply.result && <BatchResultPanel batch={batchApply.result} busy={batchApply.executing} onRetry={() => void batchApply.retryFailed(batchApply.result!)} />}
      <h1 className="text-lg font-semibold tracking-tight">技能</h1>
      {query.data.errors.length > 0 && <ErrorPanel title="技能状态读取失败" errors={query.data.errors} />}

      <div className="flex flex-wrap items-center gap-2">
        <SearchInput value={search} onChange={setSearch} placeholder="搜索技能…" testId="skills-search" />
        <Select
          label="集合筛选"
          testId="filter-collection-select"
          value={collectionFilter}
          onChange={setCollectionFilter}
          className="rounded-md border bg-card px-2 py-1.5 text-sm"
          options={[{ value: "all", label: "全部集合" }, ...collections.map((col) => ({ value: collName(col), label: collName(col) }))]}
        />
        <Select
          label="助手筛选"
          testId="filter-agent-select"
          value={agentFilter}
          onChange={(next) => {
            setAgentFilter(next);
            if (next === "all") onResetAgent?.();
          }}
          className="rounded-md border bg-card px-2 py-1.5 text-sm"
          options={[{ value: "all", label: "全部助手" }, ...agentOptions.map((agent) => ({ value: agent, label: nameOf(agent) }))]}
        />
        <AttentionOnlyToggle checked={attentionOnly} count={attentionSubjects.size} onChange={setAttentionOnly} />
        <div className="ml-auto flex items-center gap-2">
          {agentFilter === "all" && pendingFill.length > 0 && (
            <button
              type="button"
              data-testid="skills-fill-all"
              disabled={batchApply.executing}
              title="把在用的技能加到所有还缺它的助手；助手上停用的不动"
              onClick={() => void batchApply.applyBatch({ actionLabel: "补齐", rows: pendingFill })}
              className="inline-flex items-center gap-1 rounded-md border border-primary/30 bg-primary/10 px-3 py-1.5 text-sm text-primary hover:bg-primary/15 disabled:opacity-40"
            >
              全部补齐 {pendingFill.length}
            </button>
          )}
          {visibleCollections.length > 1 && !search.trim() && (
            <button
              type="button"
              data-testid="skills-toggle-all"
              onClick={() => setOpen(Object.fromEntries(visibleCollections.map((collection) => [collName(collection), !allOpen])))}
              className="inline-flex items-center gap-1 rounded-md px-2 py-1.5 text-sm text-muted-foreground hover:bg-accent hover:text-foreground"
            >
              {allOpen ? <ChevronsDownUp className="h-4 w-4" aria-hidden /> : <ChevronsUpDown className="h-4 w-4" aria-hidden />}
              {allOpen ? "全部收起" : "全部展开"}
            </button>
          )}
        </div>
      </div>

      {collections.length === 0 ? (
        <EmptyState
          icon={PackageOpen}
          testId="skills-collections-empty"
          title="还没有技能来源"
          description="技能来自 git 仓库或本机目录，在来源页添加。"
          actions={
            <button type="button" data-testid="skills-open-sources" onClick={() => onOpenSource?.(null)} className="rounded-md bg-primary px-3 py-1.5 text-sm text-primary-foreground hover:bg-primary/90">
              去来源页添加
            </button>
          }
        />
      ) : (search.trim() || attentionOnly) && shownSkillCount === 0 ? (
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
        <div role="region" aria-label="技能助手列表" tabIndex={0} className="table-scroll rounded-lg border bg-card" style={{ "--table-min-width": `${Math.max(704, 400 + groups.reduce((count, group) => count + group.columns.length * 64 + 16, 0))}px` } as CSSProperties}>
          <AccountHeader
            className="sticky top-0 z-10"
            groups={groups}
            left={<span data-testid="skills-counts" className="font-medium text-foreground">技能 · {shownSkillCount}</span>}
            below={(column) => {
              const rows = fillOf(column.id);
              return rows.length > 0 ? (
                <button
                  type="button"
                  data-testid={`skills-fill-${column.id}`}
                  disabled={batchApply.executing}
                  title={`把在用的 ${rows.length} 个技能加到 ${column.fullName}；助手上停用的不动`}
                  onClick={() => void batchApply.applyBatch({ actionLabel: "补齐", rows })}
                  className="text-primary hover:underline disabled:opacity-40"
                >
                  补齐 {rows.length}
                </button>
              ) : null;
            }}
          />
          {visibleCollections.map((collection) => {
            const name = collName(collection);
            const spec = collection.spec as SkillCollectionSpec;
            const observed = collection.observed as SkillCollectionObserved;
            const enabled = (collection.policy as { enabled?: boolean }).enabled !== false;
            const inboxCount = observed.discovery_inbox?.length ?? 0;
            const rows = skillsOf(collection);
            const open = isOpen(name, rows);
            const attention = rows.filter(needsAttention).length;
            const onCount = (agent: string) =>
              rows.filter((skill) => ACTIVE.has((skill.observed as SkillObserved).agents?.[agent]?.state ?? "")).length;
            return (
              <section key={collection.id} data-testid={`collection-card-${name}`} className="border-b last:border-b-0">
                <header className={cn("group flex items-center gap-3 bg-muted/40 py-2 pl-3 pr-4", open && "border-b")}>
                  <div className="flex min-w-0 flex-1 flex-wrap items-center gap-2 [&>span]:shrink-0">
                  <button
                    type="button"
                    data-testid={`collection-toggle-${name}`}
                    aria-expanded={open}
                    aria-label={`${open ? "收起" : "展开"}集合 ${name}`}
                    onClick={() => setOpen({ [name]: !open })}
                    className="flex min-w-0 max-w-full flex-wrap items-center gap-2 rounded text-left hover:text-primary"
                  >
                    {open ? <ChevronDown className="h-4 w-4 shrink-0 text-muted-foreground" aria-hidden /> : <ChevronRight className="h-4 w-4 shrink-0 text-muted-foreground" aria-hidden />}
                    <span className="min-w-0 break-words [overflow-wrap:anywhere] font-semibold">{name}</span>
                    <span className="shrink-0 rounded-full bg-muted px-2 py-0.5 text-[11px] text-muted-foreground">{rows.length} 个技能</span>
                  </button>
                  {attention > 0 && (
                    <span data-testid={`collection-attention-${name}`} className="shrink-0 rounded bg-warning/15 px-1.5 py-0.5 text-[11px] text-warning">
                      {attention} 个待处理
                    </span>
                  )}
                  <span className={cn("!shrink min-w-0 w-full break-words [overflow-wrap:anywhere] font-mono text-xs text-muted-foreground", !open && "hidden")} title={spec.source_path ?? spec.url ?? undefined}>
                    {spec.source_path ?? spec.url ?? "（未填来源）"}
                  </span>
                  {!enabled && <span className="rounded bg-muted px-1.5 py-0.5 text-[11px] text-muted-foreground">已停用</span>}
                  {!observed.repo_cached && (
                    <button type="button" data-testid={`collection-uncached-${name}`} onClick={() => onOpenSource?.(name)} title="到来源页处理" className="rounded bg-warning/15 px-1.5 py-0.5 text-[11px] text-warning hover:underline">
                      {spec.source_path ? "本机目录不存在" : "尚未下载"}
                    </button>
                  )}
                  {spec.url && observed.version?.current && (
                    <span
                      data-testid={`collection-version-${name}`}
                      className="max-w-full break-words [overflow-wrap:anywhere] rounded bg-muted px-1.5 py-0.5 font-mono text-[11px] text-muted-foreground"
                      title={`跟踪：${describeTrack(spec.track)}`}
                    >
                      {versionLabel(observed.version.current)}
                    </span>
                  )}
                  {collection.freshness.state === "behind" && (
                    <button type="button" data-testid={`collection-behind-${name}`} onClick={() => onOpenSource?.(name)} title={collection.freshness.upstream ? `到来源页预览并更新到 ${collection.freshness.upstream}` : "到来源页预览并更新"} className="rounded bg-info/10 px-1.5 py-0.5 text-[11px] text-info hover:underline">
                      可更新
                    </button>
                  )}
                  {observed.version?.track_error && (
                    <button type="button" data-testid={`collection-track-error-${name}`} onClick={() => onOpenSource?.(name)} className="rounded bg-warning/15 px-1.5 py-0.5 text-[11px] text-warning hover:underline" title={observed.version.track_error}>
                      跟踪方式有误
                    </button>
                  )}
                  {inboxCount > 0 && (
                    <span data-testid={`inbox-count-${name}`} className="rounded bg-info/10 px-1.5 py-0.5 text-[11px] text-info">
                      新发现 {inboxCount}
                    </span>
                  )}
                  </div>
                  <AccountCells
                    groups={groups}
                    cell={(column) => (
                      <span data-testid={`collection-count-${name}-${column.id}`} className="text-xs tabular-nums text-muted-foreground" title={`${column.fullName}：启用 ${onCount(column.id)} / ${rows.length}`}>
                        {onCount(column.id)}/{rows.length}
                      </span>
                    )}
                  />
                  {/* 与表头、技能行的行尾操作列同宽，助手列才对得齐 */}
                  <div className={TRAILING} />
                </header>
                {observed.discovery_error && (
                  <p data-testid={`discovery-error-${name}`} className="bg-warning/10 px-4 py-1.5 text-xs text-warning">
                    {observed.discovery_error.message ?? "技能发现失败"}
                  </p>
                )}
                {observed.blocked_reason && <p className="bg-warning/10 px-4 py-1.5 text-xs text-warning">{observed.blocked_reason}</p>}
                {open && rows.length === 0 && !search.trim() && (
                  <p className="px-4 py-2 text-xs text-muted-foreground">
                    {observed.repo_cached ? "没有找到技能" : "到来源页点「更新」取回后，技能会出现在这里"}
                  </p>
                )}
                {open && (
                <ul className="divide-y">
                  {rows.map((skill) => {
                    const cells = cellsIn(skill);
                    const summary = summarize(skill, Object.fromEntries(cells), nameOf);
                    const addRows = summary.inbox ? addToAllRows(skill) : [];
                    const skillName = (skill.spec as SkillSpec).name ?? skill.id;
                    const chips = cells.map(([agent, cell]) => ({ agent, cell, chip: skillChip(cell) }));
                    const menu = [
                      ...chips.flatMap(({ agent, cell, chip }) =>
                        chip.secondary.map((item) => ({
                          label: `${item.label} · ${nameOf(agent)}`,
                          testId: `action-${item.action}-${skill.id}-${agent}`,
                          danger: item.action === "remove",
                          onSelect: () => handleSkillAction(skill, agent, item.action, cell),
                        })),
                      ),
                      ...(summary.inbox
                        ? []
                        : [{ label: "改成我的版本…", testId: `fork-${skill.id}`, onSelect: () => setForking(skill) }]),
                    ];
                    const forkedFrom = (skill.observed as SkillObserved).forked_from;
                    return (
                      <li key={skill.id} data-testid={`skill-row-${skill.id}`} className="group flex items-center gap-3 py-1.5 pl-10 pr-4 hover:bg-accent/40">
                        <div className="flex min-w-0 flex-1 flex-wrap items-center gap-2">
                          <span className="min-w-0 truncate text-sm">{skillName}</span>
                          {summary.inbox && <span className="rounded bg-info/10 px-1.5 py-0.5 text-[11px] text-info">新发现</span>}
                          {forkedFrom && (
                            <span data-testid={`forked-from-${skill.id}`} className="rounded bg-muted px-1.5 py-0.5 text-[11px] text-muted-foreground" title="你的版本，原集合更新不会改它">
                              基于 {forkedFrom.collection}
                            </span>
                          )}
                          <DependencyChips chips={dependenciesOf(programs, "skill", skillName)} onOpen={onOpenProgram} />
                          {forkedFrom?.upstream_changed && (
                            <span data-testid={`upstream-changed-${skill.id}`} className="rounded bg-info/10 px-1.5 py-0.5 text-[11px] text-info" title={`${forkedFrom.collection} 里的这个技能在你复制之后更新过；要不要合进来由你决定`}>
                              原版有更新
                            </span>
                          )}
                          {summary.badges.map((badge) => (
                            <StateBadge key={badge.text} state={badge.state}>{badge.text}</StateBadge>
                          ))}
                          {addRows.length > 0 && (
                            <button
                              type="button"
                              data-testid={`skill-add-all-${skill.id}`}
                              disabled={batchApply.executing}
                              onClick={() => void batchApply.applyBatch({ actionLabel: "添加", rows: addRows })}
                              className="rounded-md border border-primary/30 bg-primary/10 px-2 py-0.5 text-xs text-primary hover:bg-primary/15 disabled:opacity-40"
                            >
                              添加到全部助手
                            </button>
                          )}
                        </div>
                        <AccountCells
                          groups={groups}
                          cell={(column) => {
                            const found = chips.find((item) => item.agent === column.id);
                            if (!found) return null;
                            const { cell, chip } = found;
                            return (
                              <AccountSwitch
                                tone={chip.tone}
                                checked={chip.checked}
                                title={`${column.fullName}：${chip.hint}`}
                                testId={chip.primary ? `action-${chip.primary.action}-${skill.id}-${column.id}` : `chip-${skill.id}-${column.id}`}
                                disabled={directApply.executing || batchApply.executing}
                                onClick={chip.primary ? () => handleSkillAction(skill, column.id, chip.primary!.action, cell) : undefined}
                              />
                            );
                          }}
                        />
                        <RowActions>
                          <ActionMenu label={`${skillName} 的更多操作`} items={menu} />
                        </RowActions>
                      </li>
                    );
                  })}
                </ul>
                )}
              </section>
            );
          })}
        </div>
      )}


      {unmanagedCount > 0 && (
        <button
          type="button"
          data-testid="skills-unmanaged-link"
          onClick={() => onOpenSource?.("unmanaged")}
          className="flex w-full items-center gap-2 rounded-lg border border-info/30 bg-info/5 px-4 py-2 text-left text-sm text-info hover:bg-info/10"
        >
          <Library className="h-4 w-4 shrink-0" aria-hidden />
          各助手里有 {unmanagedCount} 个技能不来自任何来源，到来源页「未纳管」决定是否纳入
        </button>
      )}

      <details className="border-t pt-3">
        <summary className="w-fit cursor-pointer text-xs text-muted-foreground hover:text-foreground">各助手技能目录里的实际内容</summary>
        <div className="mt-3 rounded-lg border bg-card p-3">
          <SkillInstalledView entries={installed} agents={agentOptions} />
        </div>
      </details>

      {forking && (
        <ForkSkillDialog
          skill={(forking.spec as SkillSpec).name ?? forking.id}
          collection={(forking.spec as SkillSpec).collection ?? ""}
          collectionPaths={Object.fromEntries(collections
            .filter((item) => Boolean((item.spec as SkillCollectionSpec).source_path))
            .map((item) => [collName(item), (item.spec as SkillCollectionSpec).source_path!]))}
          accounts={Object.entries((forking.observed as SkillObserved).agents ?? {})
            .filter(([, cell]) => cell.state === "linked" || cell.state === "missing-link" || cell.state === "dead-link")
            .map(([id]) => ({ id, name: nameOf(id) }))}
          executing={forkBusy}
          onConfirm={(input) => void runFork(forking, input)}
          onClose={() => !forkBusy && setForking(null)}
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
          }。不影响其他助手。`}
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
