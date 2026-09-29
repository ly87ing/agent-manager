import { useEffect, useState } from "react";
import { ChevronDown, ChevronRight, ChevronsDownUp, ChevronsUpDown, PackageOpen, Pencil, Power, PowerOff, Plus, RefreshCw, SearchX, Share2, Sparkles, Trash2 } from "lucide-react";
import { toast } from "sonner";
import { useQueryClient } from "@tanstack/react-query";
import { ConfirmModal } from "../components/ConfirmModal";
import { EmptyState } from "../components/EmptyState";
import { ErrorPanel } from "../components/ErrorPanel";
import { PageError, PageLoading } from "../components/PageState";
import { SearchInput, matchesKeyword } from "../components/SearchInput";
import { AccountCells, AccountHeader, AccountSwitch, ActionMenu, IconAction, RowActions } from "../components/AccountGrid";
import { skillChip, skillNeedsUpdate } from "../lib/skillChip";
import { SkillCollectionFormDialog } from "../components/SkillCollectionFormDialog";
import { SkillInstalledView } from "../components/SkillInstalledView";
import { SkillRecommendations } from "../components/SkillRecommendations";
import type { EngineElement, PolicyData, SkillRecommendation, StatusData } from "../lib/contract";
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
import { addToAllRows, fillRows, versionUpdateRows } from "../lib/skillFill";
import { useExecutions } from "../lib/execution";
import { encodeShareLink, type SharePayload } from "../lib/shareLink";
import { availableSkills, fullSkillDefinition, isCollection } from "../lib/skillCollections";
import { dismissRecommendation as dismissRecommendationFor, loadDismissed } from "../lib/recommendDismiss";
import { RemoveCollectionDialog } from "../components/RemoveCollectionDialog";
import { ForkSkillDialog } from "../components/ForkSkillDialog";
import { useBatchApply } from "../lib/useBatchApply";
import { useAccountGroups, useAccountNames } from "../lib/useAccountNames";
import { useDefinitionOps } from "../lib/useDefinitionOps";
import { useDirectApply } from "../lib/useDirectApply";
import { cn } from "../lib/utils";

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
  if (entries.some(([, cell]) => skillNeedsUpdate(cell))) badges.push({ text: "有新版本", tone: "info" });
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
  importPayload,
  onImportConsumed,
}: {
  /** 从分享链接导入的技能集合：打开预填好的「新增集合」表单，用户检查后保存。 */
  importPayload?: Extract<SharePayload, { kind: "skills" }> | null;
  onImportConsumed?: () => void;
  /** 从账号页进来：只看这个账号。 */
  initialAgent?: string | null;
  /** 从总览「去处理」进来时直接搜到那个技能。 */
  initialSearch?: string | null;
  onResetAgent?: () => void;
} = {}) {
  const query = useEngineQuery<StatusData>("skills:status");
  const nameOf = useAccountNames();
  const groupsOf = useAccountGroups();
  const policyQuery = useEngineQuery<PolicyData>("policy:get");
  const [collectionFilter, setCollectionFilter] = useState<string>("all");
  const [agentFilter, setAgentFilter] = useState<string>(() => initialAgent ?? "all");
  const [search, setSearch] = useState(initialSearch ?? "");
  const [expanded, setExpanded] = useState<Record<string, boolean>>(loadExpanded);
  const [showAddForm, setShowAddForm] = useState(false);
  const [editing, setEditing] = useState<EngineElement | null>(null);
  const [pendingRemove, setPendingRemove] = useState<EngineElement | null>(null);
  const [pendingRecommendation, setPendingRecommendation] = useState<SkillRecommendation | null>(null);
  /** 推荐卡片上的「启用…」：确认后一次做完登记、下载、链接到全部账号。 */
  const [enabling, setEnabling] = useState<SkillRecommendation | null>(null);
  const [enablingBusy, setEnablingBusy] = useState(false);
  const [dismissed, setDismissed] = useState<string[]>(loadDismissed);
  const [pendingTakeover, setPendingTakeover] = useState<{ element: EngineElement; agent: string; action: "add" | "remove" } | null>(null);
  /** 正在「改成我的版本」的技能。 */
  const [forking, setForking] = useState<EngineElement | null>(null);
  const [forkBusy, setForkBusy] = useState(false);
  const [pendingDropped, setPendingDropped] = useState<{ element: EngineElement; agent: string; cell: SkillAgentCell } | null>(null);

  const directApply = useDirectApply();
  const batchApply = useBatchApply();
  const { execute } = useExecutions();
  const [updatingAll, setUpdatingAll] = useState(false);
  const queryClient = useQueryClient();
  const definitionOps = useDefinitionOps({ section: "skills", domain: "skills" });

  useEffect(() => {
    setAgentFilter(initialAgent ?? "all");
  }, [initialAgent]);

  const [importDraft, setImportDraft] = useState<{ name: string; definition: Record<string, unknown> } | null>(null);
  useEffect(() => {
    if (!importPayload) return;
    setImportDraft({ name: importPayload.name, definition: importPayload.definition });
    setShowAddForm(true);
    onImportConsumed?.();
  }, [importPayload]);

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

  const runCollectionAction = (collection: EngineElement, action: "update" | "enable" | "disable", title: string) => {
    if (action === "disable") dismissIfRecommended(collection);
    return void directApply
      .apply({ command: "skills:apply", label: title, args: ["--only", collection.id, "--action", action] })
      .then((envelope) => {
        // 拉完代码顺手重查一次最新提交，否则还挂着「可更新」
        if (action === "update" && envelope?.ok) return refreshUpstream(collection);
      });
  };

  /**
   * 全部更新：逐个取回启用中的集合（git 拉到最新、本机目录重新扫描），再把各账号在用、
   * 已有新版本的技能换到新版本。新发现的技能不自动添加，账号上停用的不动；单个失败不拦其余。
   */
  const updateAll = async () => {
    if (updatingAll) return;
    setUpdatingAll(true);
    let fetched = 0;
    let failed = 0;
    try {
      const targets = collections.filter(
        (collection) =>
          (collection.policy as { enabled?: boolean }).enabled !== false &&
          ((collection.observed as SkillCollectionObserved).actions ?? []).includes("update"),
      );
      for (const collection of targets) {
        const envelope = await execute({
          command: "skills:apply",
          label: `更新集合 ${collName(collection)}`,
          args: ["--only", collection.id, "--action", "update"],
          silent: true,
        });
        if (envelope?.ok) {
          fetched += 1;
          await runEngine("system:refresh-upstream", ["--only", collection.id]).catch(() => undefined);
        } else {
          failed += 1;
        }
      }
      // 取回之后按最新状态算哪些技能有新版本
      const fresh = await runEngine<StatusData>("skills:status", []);
      const rows = fresh.ok ? versionUpdateRows(fresh.data.elements ?? []) : [];
      (failed ? toast.error : toast.success)(
        `已取回 ${fetched} 个集合${failed ? `，${failed} 个失败（详见任务记录）` : ""}；${rows.length ? `${rows.length} 处技能换到新版本` : "各账号的技能都已是最新"}`,
      );
      await batchApply.applyBatch({ actionLabel: "更新", rows });
    } finally {
      setUpdatingAll(false);
      await queryClient.invalidateQueries({ queryKey: engineKeys.all });
    }
  };

  /** 复制技能集合的分享链接：只含 git 地址与技能、规则位置，不含本机路径。 */
  const shareCollection = async (name: string, spec: SkillCollectionSpec) => {
    try {
      const link = encodeShareLink({
        kind: "skills",
        name,
        definition: { url: spec.url as string, skill_paths: spec.skill_paths ?? null, rules_path: (spec as { rules_path?: string | null }).rules_path ?? null },
      });
      await navigator.clipboard.writeText(link);
      toast.success(`已复制 ${name} 的分享链接`);
    } catch (reason) {
      toast.error(`没能复制：${reason instanceof Error ? reason.message : String(reason)}`);
    }
  };

  /**
   * 改成我的版本：先复制并登记（skills:fork），成功后把勾选的账号逐个换过去——
   * 先取消原集合的分配，再加上用户的版本；每一步照常核对，某个账号失败不影响其余。
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
        const removed = await execute({ command: "skills:apply", label: `换掉原版本 ${name} @ ${nameOf(agent)}`, args: ["--only", skill.id, "--action", "remove", "--agent", agent], silent: true });
        const added = removed?.ok
          ? await execute({ command: "skills:apply", label: `换成我的版本 ${name} @ ${nameOf(agent)}`, args: ["--only", `skill:${input.target}/${name}`, "--action", "add", "--agent", agent], silent: true })
          : null;
        if (!added?.ok) failed += 1;
      }
      const path = (forked.data as { path?: string }).path ?? input.dest;
      (failed ? toast.error : toast.success)(
        failed ? `已复制到 ${path}，但 ${failed} 个账号没换过去，详见任务记录` : `已复制到 ${path}，${input.switchAccounts.length} 个账号换成了你的版本`,
      );
      setForking(null);
    } finally {
      setForkBusy(false);
      await queryClient.invalidateQueries({ queryKey: engineKeys.all });
    }
  };

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

  const confirmRemove = (purge: boolean) => {
    if (!pendingRemove) return;
    dismissIfRecommended(pendingRemove);
    void directApply.apply({
      command: "skills:apply",
      label: `移除来源 ${collName(pendingRemove)}`,
      args: ["--only", pendingRemove.id, "--action", "remove", ...(purge ? ["--purge"] : [])],
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

  /**
   * 启用推荐来源：保存集合定义 → 下载 → 把其中的技能链接到全部账号。账号里已有同名技能目录或链接的，
   * 引擎不给 add，跳过、不覆盖；规则不在这里写。任何一步失败就停在那一步，已做的如实保留。
   */
  const enableRecommended = async (item: SkillRecommendation) => {
    setEnablingBusy(true);
    try {
      if (!(await definitionOps.saveDefinition({ name: item.name, definition: item.definition, isNew: true }))) return;
      const fetched = await execute({ command: "skills:apply", label: `下载 ${item.name}`, args: ["--only", `skill:${item.name}`, "--action", "update"], silent: true });
      if (!fetched?.ok) {
        toast.error(`${item.name} 已登记，但下载失败：${fetched?.errors[0]?.message ?? "详见任务记录"}。稍后在集合上点「更新」重试`);
        return;
      }
      const fresh = await runEngine<StatusData>("skills:status", []);
      const rows = (fresh.ok ? fresh.data.elements ?? [] : [])
        .filter((element) => (element.spec as SkillSpec).collection === item.name)
        .flatMap(addToAllRows);
      await batchApply.applyBatch({ actionLabel: "添加", rows });
      toast.info(`${item.name} 已启用。规则没有改动：想用它的规则，到规则页看过差异再应用。`);
    } finally {
      setEnablingBusy(false);
      setEnabling(null);
      await queryClient.invalidateQueries({ queryKey: engineKeys.all });
    }
  };

  const dismissRecommendation = (name: string) => setDismissed(dismissRecommendationFor(name));
  /** 停用或移除的是推荐来源（同名或同地址）：视为不想用了，不再推荐。 */
  const dismissIfRecommended = (collection: EngineElement) => {
    const spec = collection.spec as SkillCollectionSpec;
    const match = recommendations.find((item) => item.name === collName(collection) || (spec.url && item.url === spec.url));
    if (match) dismissRecommendation(match.name);
  };
  const featured = recommendations.find((item) => !item.added && !dismissed.includes(item.name));

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
  /** 需要处理：有黄色开关（损坏、占用、来源已删、外部链接）或新发现待添加。 */
  const needsAttention = (skill: EngineElement) =>
    (skill.observed as SkillObserved).discovery_inbox === true || cellsIn(skill).some(([, cell]) => skillChip(cell).tone === "warn");
  /** 搜索或只看一个集合时展开命中的；否则按用户上次的选择，没选过的有待处理或还没有技能时才展开。 */
  const isOpen = (name: string, rows: EngineElement[]) =>
    Boolean(search.trim()) || collectionFilter === name || (expanded[name] ?? (rows.length === 0 || rows.some(needsAttention)));
  const setOpen = (next: Record<string, boolean>) => {
    const merged = { ...expanded, ...next };
    setExpanded(merged);
    saveExpanded(merged);
  };
  const allOpen = visibleCollections.every((collection) => isOpen(collName(collection), skillsOf(collection)));
  const shownAgents = agentFilter === "all" ? agentOptions : [agentFilter];
  const groups = groupsOf(shownAgents);
  /** 每个账号还缺哪些在用的技能：表头「补齐」逐个账号做。 */
  const fillOf = (agent: string) => pendingFill.filter((row) => row.agent === agent);

  return (
    <div className="space-y-3">
      <h1 className="sr-only">技能</h1>
      {query.data.errors.length > 0 && <ErrorPanel title="技能状态读取失败" errors={query.data.errors} />}

      {featured && (
        <div data-testid={`recommend-banner-${featured.name}`} className="flex items-center gap-3 rounded-lg border border-primary/30 bg-primary/5 px-4 py-3">
          <Sparkles className="h-5 w-5 shrink-0 text-primary" aria-hidden />
          <div className="min-w-0 flex-1">
            <p className="text-sm font-medium">推荐启用 {featured.name}</p>
            <p className="text-xs text-muted-foreground">{featured.description}</p>
          </div>
          <button
            type="button"
            data-testid={`recommend-dismiss-${featured.name}`}
            onClick={() => dismissRecommendation(featured.name)}
            className="shrink-0 rounded-md px-3 py-1.5 text-sm text-muted-foreground hover:bg-accent hover:text-foreground"
          >
            不用了
          </button>
          <button
            type="button"
            data-testid={`recommend-enable-${featured.name}`}
            onClick={() => setEnabling(featured)}
            className="shrink-0 rounded-md bg-primary px-3 py-1.5 text-sm text-primary-foreground hover:bg-primary/90"
          >
            启用…
          </button>
        </div>
      )}

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
          {collections.length > 0 && (
            <button
              type="button"
              data-testid="skills-update-all"
              disabled={updatingAll || batchApply.executing}
              title="取回所有集合的新版本，并把各账号在用的技能换到新版本；新发现的技能不会自动添加，账号上停用的不动"
              onClick={() => void updateAll()}
              className="inline-flex items-center gap-1 rounded-md border px-3 py-1.5 text-sm hover:bg-accent disabled:opacity-40"
            >
              <RefreshCw className={cn("h-4 w-4", updatingAll && "animate-spin")} aria-hidden /> {updatingAll ? "正在更新…" : "全部更新"}
            </button>
          )}
          {agentFilter === "all" && pendingFill.length > 0 && (
            <button
              type="button"
              data-testid="skills-fill-all"
              disabled={updatingAll || batchApply.executing}
              title="把在用的技能加到所有还缺它的账号；账号上停用的不动"
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
          <button
            type="button"
            data-testid="add-collection-button"
            onClick={() => setShowAddForm(true)}
            className="inline-flex items-center gap-1 rounded-md bg-primary px-3 py-1.5 text-sm text-primary-foreground hover:bg-primary/90"
          >
            <Plus className="h-4 w-4" aria-hidden /> 添加来源
          </button>
        </div>
      </div>

      {collections.length === 0 ? (
        <EmptyState
          icon={PackageOpen}
          testId="skills-collections-empty"
          title="还没有技能来源"
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
        <div className="overflow-clip rounded-lg border bg-card">
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
                  title={`把在用的 ${rows.length} 个技能加到 ${column.fullName}；账号上停用的不动`}
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
            const actions = observed.actions ?? [];
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
                  <div className="flex min-w-0 flex-1 items-center gap-2 whitespace-nowrap [&>span]:shrink-0">
                  <button
                    type="button"
                    data-testid={`collection-toggle-${name}`}
                    aria-expanded={open}
                    aria-label={`${open ? "收起" : "展开"}集合 ${name}`}
                    onClick={() => setOpen({ [name]: !open })}
                    className="flex shrink-0 items-center gap-2 rounded text-left hover:text-primary"
                  >
                    {open ? <ChevronDown className="h-4 w-4 shrink-0 text-muted-foreground" aria-hidden /> : <ChevronRight className="h-4 w-4 shrink-0 text-muted-foreground" aria-hidden />}
                    <span className="font-semibold">{name}</span>
                    <span className="shrink-0 rounded-full bg-muted px-2 py-0.5 text-[11px] text-muted-foreground">{rows.length} 个技能</span>
                  </button>
                  {attention > 0 && (
                    <span data-testid={`collection-attention-${name}`} className="shrink-0 rounded bg-warning/15 px-1.5 py-0.5 text-[11px] text-warning">
                      {attention} 个待处理
                    </span>
                  )}
                  <span className={cn("!shrink min-w-0 truncate font-mono text-xs text-muted-foreground", !open && "hidden")} title={spec.source_path ?? spec.url ?? undefined}>
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
                  </div>
                  <AccountCells
                    groups={groups}
                    cell={(column) => (
                      <span data-testid={`collection-count-${name}-${column.id}`} className="text-xs tabular-nums text-muted-foreground" title={`${column.fullName}：启用 ${onCount(column.id)} / ${rows.length}`}>
                        {onCount(column.id)}/{rows.length}
                      </span>
                    )}
                  />
                  {/* 集合（来源）的操作常驻显示：移除来源等要一眼找得到 */}
                  <RowActions
                    always={
                      <>
                    {actions.includes("update") && (collection.freshness.state === "behind" || !observed.repo_cached) && (
                        <IconAction tone="primary" label={`更新集合 ${name}`} testId={`update-collection-${name}`} onClick={() => runCollectionAction(collection, "update", `更新集合 ${name}`)}>
                          <RefreshCw className="h-3.5 w-3.5" aria-hidden />
                        </IconAction>
                    )}
                    {spec.url && (
                      <IconAction label={`复制 ${name} 的分享链接`} testId={`share-collection-${name}`} onClick={() => void shareCollection(name, spec)}>
                        <Share2 className="h-3.5 w-3.5" aria-hidden />
                      </IconAction>
                    )}
                    <IconAction label={`编辑集合 ${name}`} testId={`edit-collection-${name}`} onClick={() => setEditing(collection)}>
                      <Pencil className="h-3.5 w-3.5" aria-hidden />
                    </IconAction>
                    {actions.includes("enable") && !enabled && (
                      <IconAction label={`启用集合 ${name}`} testId={`enable-collection-${name}`} onClick={() => runCollectionAction(collection, "enable", `启用集合 ${name}`)}>
                        <Power className="h-3.5 w-3.5" aria-hidden />
                      </IconAction>
                    )}
                    {actions.includes("disable") && enabled && (
                      <IconAction label={`停用集合 ${name}`} testId={`disable-collection-${name}`} onClick={() => runCollectionAction(collection, "disable", `停用集合 ${name}`)}>
                        <PowerOff className="h-3.5 w-3.5" aria-hidden />
                      </IconAction>
                    )}
                    {actions.includes("remove") && (
                      <IconAction danger label={`移除来源 ${name}`} testId={`remove-collection-${name}`} onClick={() => setPendingRemove(collection)}>
                        <Trash2 className="h-3.5 w-3.5" aria-hidden />
                      </IconAction>
                    )}
                      </>
                    }
                  />
                </header>
                {observed.discovery_error && (
                  <p data-testid={`discovery-error-${name}`} className="bg-warning/10 px-4 py-1.5 text-xs text-warning">
                    {observed.discovery_error.message ?? "技能发现失败"}
                  </p>
                )}
                {observed.blocked_reason && <p className="bg-warning/10 px-4 py-1.5 text-xs text-warning">{observed.blocked_reason}</p>}
                {open && rows.length === 0 && !search.trim() && (
                  <p className="px-4 py-2 text-xs text-muted-foreground">
                    {observed.repo_cached ? "没有找到技能" : "点「更新」取回后技能会出现在这里"}
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
                          {forkedFrom?.upstream_changed && (
                            <span data-testid={`upstream-changed-${skill.id}`} className="rounded bg-info/10 px-1.5 py-0.5 text-[11px] text-info" title={`${forkedFrom.collection} 里的这个技能在你复制之后更新过；要不要合进来由你决定`}>
                              原版有更新
                            </span>
                          )}
                          {summary.badges.map((badge) => (
                            <span key={badge.text} className={cn("rounded px-1.5 py-0.5 text-[11px]", TONE[badge.tone])}>{badge.text}</span>
                          ))}
                          {addRows.length > 0 && (
                            <button
                              type="button"
                              data-testid={`skill-add-all-${skill.id}`}
                              disabled={batchApply.executing}
                              onClick={() => void batchApply.applyBatch({ actionLabel: "添加", rows: addRows })}
                              className="rounded-md border border-primary/30 bg-primary/10 px-2 py-0.5 text-xs text-primary hover:bg-primary/15 disabled:opacity-40"
                            >
                              添加到全部账号
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
          draft={editing ? undefined : importDraft ?? undefined}
          fullDefinition={editing ? fullSkillDefinition(layers, collName(editing)) : null}
          available={editing ? availableSkills(editing.observed as SkillCollectionObserved) : undefined}
          repoCached={editing ? (editing.observed as SkillCollectionObserved).repo_cached : undefined}
          onClose={() => {
            setShowAddForm(false);
            setEditing(null);
            setImportDraft(null);
          }}
          onSubmit={(name, definition, complete) => {
            setShowAddForm(false);
            setEditing(null);
            setImportDraft(null);
            const isNew = !editing;
            void definitionOps.saveDefinition({ name, definition, isNew, complete }).then(() => {
              if (isNew) toast.info(`集合 ${name} 已保存，点「更新」取回技能。`);
            });
          }}
        />
      )}
      {definitionOps.modals}
      {pendingRemove && (
        <RemoveCollectionDialog
          name={collName(pendingRemove)}
          local={Boolean((pendingRemove.spec as SkillCollectionSpec).source_path)}
          executing={directApply.executing}
          onConfirm={confirmRemove}
          onClose={() => setPendingRemove(null)}
        />
      )}
      {enabling && (
        <ConfirmModal
          title={`启用 ${enabling.name}`}
          description="确认后依次执行，只做下面这些，不改动其他任何配置："
          risk="write-local"
          confirmLabel="启用"
          executing={enablingBusy}
          onConfirm={() => void enableRecommended(enabling)}
          onClose={() => !enablingBusy && setEnabling(null)}
        >
          <ol data-testid="recommend-enable-steps" className="mt-3 list-decimal space-y-1.5 pl-5 text-sm">
            <li>登记来源 {enabling.name}（{enabling.url}）</li>
            <li>把仓库下载到本工具的数据目录</li>
            <li>把其中的技能链接到全部账号；账号里已有同名技能目录或链接的跳过，不覆盖</li>
            <li className="text-muted-foreground">规则、MCP、启动命令都不动；它带的规则可以到规则页看过差异再决定</li>
          </ol>
          <p className="mt-2 text-xs text-muted-foreground">以后不想用了：在集合上停用或删除，账号里的链接随之移除。</p>
        </ConfirmModal>
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
      {forking && (
        <ForkSkillDialog
          skill={(forking.spec as SkillSpec).name ?? forking.id}
          collection={(forking.spec as SkillSpec).collection ?? ""}
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
