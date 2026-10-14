import { t } from "../lib/i18n";
import { useEffect, useRef, useState } from "react";
import { Link2, Plus, RefreshCw } from "lucide-react";
import { notifyError, toast } from "../lib/notifications";
import { useQueryClient } from "@tanstack/react-query";
import { ConfirmModal } from "../components/ConfirmModal";
import { ActionMenu } from "../components/AccountGrid";
import { COL_NARROW, DataTable, PILL, ROW } from "../components/DataTable";
import { EmptyState } from "../components/EmptyState";
import { PageError, PageLoading } from "../components/PageState";
import { RemoveCollectionDialog } from "../components/RemoveCollectionDialog";
import { RuleSourceFormDialog, RemoveSourceDialog, useRuleSources, type UserSource } from "../components/RuleSources";
import { SkillAdoptDialog } from "../components/SkillAdoptDialog";
import { SkillCollectionFormDialog } from "../components/SkillCollectionFormDialog";
import { SkillOtherSources } from "../components/SkillOtherSources";
import { SkillVersionsDialog } from "../components/SkillVersionsDialog";
import { DiscoverPanel } from "../components/DiscoverPanel";
import type { EngineElement, PolicyData, SkillRecommendation, StatusData } from "../lib/contract";
import { runEngine } from "../lib/engine";
import { useExecutions } from "../lib/execution";
import { type SkillCollectionObserved, type SkillCollectionSpec, type SkillSpec } from "../lib/observed";
import { engineKeys, useCheckingUpdates, useEngineQuery } from "../lib/queries";
import { dismissRecommendation, loadDismissed } from "../lib/recommendDismiss";
import { encodeShareLink, type SharePayload } from "../lib/shareLink";
import { availableSkills, fullSkillDefinition, isCollection } from "../lib/skillCollections";
import { addToAllRows, versionUpdateRows } from "../lib/skillFill";
import { describeTrack, versionLabel } from "../lib/skillTrack";
import { useAccountGroups, useResourceTargets, useAccountNames } from "../lib/useAccountNames";
import { useBatchApply } from "../lib/useBatchApply";
import { BatchResultPanel } from "../components/BatchResultPanel";
import { useDefinitionOps } from "../lib/useDefinitionOps";
import { useDirectApply } from "../lib/useDirectApply";
import { updatableSources, useSourceUpdate } from "../lib/useSourceUpdate";
import { shortRef } from "../lib/inbox";
import { cn } from "../lib/utils";

/** 来源表的列：「提供」只写几个技能几条规则，收窄；操作列放一个主按钮和「⋯」。 */
const COL_PROVIDES = "w-40 shrink-0";
const ACTIONS = "flex w-32 shrink-0 items-center justify-end gap-1";
const TEXT_BUTTON = "rounded px-2 py-1 text-xs text-muted-foreground hover:bg-accent hover:text-foreground disabled:opacity-40";
const TONE = { ok: "text-success", warn: "text-warning", muted: "text-muted-foreground", info: "text-info" } as const;

/**
 * 来源：内容从哪来、用哪个版本（docs/MULTI-ACCOUNT-DESIGN.md「管理体系」）。来源的整个生命周期只在这里：
 * 添加、推荐、编辑跟踪方式、检查与更新（git 先预览再换）、换版本、分享、停用、移除，以及把各助手里
 * 不来自任何来源的技能纳入进来。分配到哪些助手在技能、规则页，不在这里。
 */
export default function SourcesPage({
  focus,
  importPayload,
  onImportConsumed,
  onImportLink,
}: {
  /** 打开「从分享链接导入」：技能来源回到这里的添加表单，MCP 服务去 MCP 页。 */
  onImportLink?: () => void;
  /** 从别处跳来：定位到这个来源（名称），有新版本的 git 来源直接打开更新预览；`unmanaged` 定位到未纳管。 */
  focus?: string | null;
  /** 从分享链接导入的技能来源：打开预填好的添加表单，用户检查后保存。 */
  importPayload?: Extract<SharePayload, { kind: "skills" }> | null;
  onImportConsumed?: () => void;
} = {}) {
  const skills = useEngineQuery<StatusData>("skills:status", ["--collections-only"]);
  const rules = useEngineQuery<StatusData>("rules:status");
  const policy = useEngineQuery<PolicyData>("policy:get");
  const userSources = useRuleSources();
  const directApply = useDirectApply();
  const batchApply = useBatchApply();
  const sourceUpdate = useSourceUpdate();
  const { execute } = useExecutions();
  const queryClient = useQueryClient();
  const nameOfAccount = useAccountNames();
  const targets = useResourceTargets();
  const groupsOf = useAccountGroups();
  const definitionOps = useDefinitionOps();
  const [adding, setAdding] = useState<"collection" | "rule" | null>(null);
  const [editing, setEditing] = useState<EngineElement | null>(null);
  const [editingRuleSource, setEditingRuleSource] = useState<UserSource | null>(null);
  // 检查新版本进行中（任何页面或命令面板发起）：先不更新、不换版本，免得检查按更新前的版本记下「可更新」
  const checking = useCheckingUpdates();
  const CHECKING_TITLE = t("正在检查新版本，完成后再更新");
  const [draft, setDraft] = useState<{ name: string; definition: Record<string, unknown> } | null>(null);
  const [removingCollection, setRemovingCollection] = useState<EngineElement | null>(null);
  const [removingSource, setRemovingSource] = useState<UserSource | null>(null);
  const [dismissed, setDismissed] = useState(loadDismissed);
  const [enabling, setEnabling] = useState<SkillRecommendation | null>(null);
  const [enablingBusy, setEnablingBusy] = useState(false);
  const [versionsOf, setVersionsOf] = useState<{ collection: EngineElement; openLatest: boolean } | null>(null);
  const [switching, setSwitching] = useState(false);
  const [adopting, setAdopting] = useState<string | null>(null);
  const focusHandled = useRef(false);

  useEffect(() => {
    if (!importPayload) return;
    setDraft({ name: importPayload.name, definition: importPayload.definition });
    onImportConsumed?.();
  }, [importPayload]);

  const elements = skills.data?.ok ? skills.data.data.elements ?? [] : [];
  const collections = elements.filter(isCollection);

  // 跳进来定位：滚到那一行；有新版本的 git 来源直接打开更新预览（仍要用户确认才换）
  useEffect(() => {
    if (!focus || focusHandled.current || !skills.data) return;
    focusHandled.current = true;
    const target = collections.find((collection) => collection.id === `skill:${focus}`);
    document.querySelector(`[data-source-anchor="${focus}"]`)?.scrollIntoView?.({ block: "center" });
    if (target && (target.spec as SkillCollectionSpec).url && target.freshness.state === "behind") {
      setVersionsOf({ collection: target, openLatest: true });
    }
  }, [focus, skills.data]);

  if (skills.isPending) return <PageLoading />;
  if (skills.isError) return <PageError title={t("来源读取失败")} error={skills.error} onRetry={() => void skills.refetch()} retrying={skills.isFetching} />;
  if (!skills.data.ok) return <PageError title={t("来源读取失败")} errors={skills.data.errors} onRetry={() => void skills.refetch()} retrying={skills.isFetching} className="break-all" />;
  if (policy.isPending) return <PageLoading />;
  if (policy.isError) return <PageError title={t("来源定义读取失败")} error={policy.error} onRetry={() => void policy.refetch()} retrying={policy.isFetching} />;
  if (!policy.data.ok) return <PageError title={t("来源定义读取失败")} errors={policy.data.errors} onRetry={() => void policy.refetch()} retrying={policy.isFetching} className="break-all" />;
  if (rules.isPending) return <PageLoading />;
  if (rules.isError) return <PageError title={t("规则内容读取失败")} error={rules.error} onRetry={() => void rules.refetch()} retrying={rules.isFetching} />;
  if (!rules.data.ok) return <PageError title={t("规则内容读取失败")} errors={rules.data.errors} onRetry={() => void rules.refetch()} retrying={rules.isFetching} className="break-all" />;

  const skillElements = elements.filter((element) => !isCollection(element));
  const recommendations = skills.data.ok ? skills.data.data.recommendations ?? [] : [];
  const unmanaged = skills.data.ok ? skills.data.data.unmanaged ?? null : null;
  const sourceRules = rules.data?.ok ? rules.data.data.source_rules ?? [] : [];
  const layers = policy.data?.ok ? policy.data.data.layers : null;
  const nameOf = (collection: EngineElement) => collection.id.replace(/^skill:/, "");
  const skillCount = (name: string) => {
    const observed = collections.find(collection => nameOf(collection) === name)?.observed as SkillCollectionObserved | undefined;
    return observed?.discovered?.length ?? skillElements.filter((element) => (element.spec as SkillSpec).collection === name).length;
  };
  const ruleCount = (name: string) => sourceRules.filter((rule) => rule.collection === name).length;
  const agentIds = Array.from(
    new Set([
      ...targets.map(target => target.id),
      ...(unmanaged?.skills ?? []).flatMap((skill) => skill.locations.flatMap((location) => (location.agent ? [location.agent] : []))),
      ...(unmanaged?.duplicates ?? []).map((dup) => dup.agent),
    ]),
  ).sort();
  const updatable = updatableSources(collections);
  const busy = directApply.executing || sourceUpdate.updating || batchApply.executing || switching;

  /** 停用或移除的是推荐来源：视为不想用了，之后不再推荐。 */
  const dismissIfRecommended = (collection: EngineElement) => {
    const spec = collection.spec as SkillCollectionSpec;
    const match = recommendations.find((item) => item.name === nameOf(collection) || (spec.url && item.url === spec.url));
    if (match) setDismissed(dismissRecommendation(match.name));
  };
  const suggested = recommendations.filter((item) => !item.added && !dismissed.includes(item.name));

  const refreshUpstream = (collection: EngineElement) =>
    runEngine("system:refresh-upstream", ["--only", collection.id])
      .catch(() => undefined)
      .finally(() => void queryClient.invalidateQueries({ queryKey: engineKeys.all }));

  /** 更新：git 来源先预览要换的内容，确认后才换；本机目录重新扫描。 */
  const updateOne = (collection: EngineElement) => {
    if ((collection.spec as SkillCollectionSpec).url) return setVersionsOf({ collection, openLatest: true });
    return void directApply
      .apply({ command: "skills:apply", label: t("重新扫描来源 {0}", nameOf(collection)), args: ["--only", collection.id, "--action", "update"] })
      .then((envelope) => {
        if (envelope?.ok) return refreshUpstream(collection);
      });
  };

  const toggle = (collection: EngineElement, enable: boolean) => {
    if (!enable) dismissIfRecommended(collection);
    void directApply.apply({
      command: "skills:apply",
      label: t("{0}来源 {1}", enable ? t("启用") : t("停用"), nameOf(collection)),
      args: ["--only", collection.id, "--action", enable ? "enable" : "disable"],
    });
  };

  /**
   * 把 git 来源换到某个版本（ref 为 null 时按跟踪方式到最新），再把各助手在用、内容有变化的
   * 这个来源的技能换过去；停用的不动。镜像切换失败时助手一个都不动。
   */
  const switchVersion = async (collection: EngineElement, ref: string | null, label: string): Promise<boolean> => {
    const name = nameOf(collection);
    setSwitching(true);
    try {
      const envelope = await execute({
        command: "skills:apply",
        label: ref ? t("把 {0} 切换到 {1}", name, label) : t("更新 {0}", name),
        args: ["--only", collection.id, "--action", "update", ...(ref ? ["--ref", ref] : [])],
        silent: true,
      });
      if (!envelope?.ok) {
        notifyError(t("来源切换失败"), envelope?.errors[0]?.message ?? t("详见任务记录"));
        return false;
      }
      await runEngine("system:refresh-upstream", ["--only", collection.id]).catch(() => undefined);
      const fresh = await runEngine<StatusData>("skills:status", []);
      const rows = fresh.ok
        ? versionUpdateRows(fresh.data.elements ?? []).filter((row) => row.elementId.startsWith(`${collection.id}/`))
        : [];
      const result = await batchApply.applyBatch({ actionLabel: t("更新"), rows, silent: true });
      const failed = result?.rows.filter((item) => item.status === "failed").length ?? 0;
      if (!fresh.ok || failed || (rows.length > 0 && !result)) {
        toast.warning(t("{0} 来源已{1}，但技能应用未全部完成，请查看批量结果", name, ref ? t("切换") : t("更新")));
        return false;
      }
      toast.success(t("{0} 已{1}到 {2}", name, ref ? t("切换") : t("更新"), label), {
        description: rows.length ? t("{0} 处技能换到了这个版本", rows.length) : t("各助手在用的技能内容没有变化"),
      });
      return true;
    } catch (error) {
      notifyError(t("来源切换失败"), error);
      return false;
    } finally {
      setSwitching(false);
      await queryClient.invalidateQueries({ queryKey: engineKeys.all });
    }
  };

  /** 复制来源的分享链接：只含 git 地址与技能、规则位置，不含本机路径。 */
  const share = async (name: string, spec: SkillCollectionSpec) => {
    try {
      const link = encodeShareLink({
        kind: "skills",
        name,
        definition: {
          url: spec.url as string,
          skill_paths: spec.skill_paths ?? null,
          rules_path: (spec as { rules_path?: string | null }).rules_path ?? null,
          ...(spec.track ? { track: spec.track } : {}),
        },
      });
      await navigator.clipboard.writeText(link);
      toast.success(t("已复制 {0} 的分享链接", name));
    } catch (reason) {
      notifyError(t("复制失败"), reason);
    }
  };

  /**
   * 启用推荐来源：保存定义 → 下载 → 把其中的技能链接到全部助手。助手里已有同名技能目录或链接的，
   * 引擎不给 add，跳过、不覆盖；规则不在这里写。任何一步失败就停在那一步，已做的如实保留。
   */
  const enableRecommended = async (item: SkillRecommendation) => {
    setEnablingBusy(true);
    try {
      if (!(await definitionOps.saveDefinition({ name: item.name, definition: item.definition, isNew: true, silent: true }))) return;
      const fetched = await execute({ command: "skills:apply", label: t("下载 {0}", item.name), args: ["--only", `skill:${item.name}`, "--action", "update"], silent: true });
      if (!fetched?.ok) {
        toast.warning(t("{0} 已添加，但下载失败：{1}。请在来源列表选择「更新」重试。", item.name, fetched?.errors[0]?.message ?? t("详见任务记录")));
        return;
      }
      const fresh = await runEngine<StatusData>("skills:status", []);
      if (!fresh.ok) {
        toast.warning(t("{0} 来源已添加，但读取技能状态失败，请刷新后应用", item.name));
        return;
      }
      const rows = (fresh.ok ? fresh.data.elements ?? [] : [])
        .filter((element) => (element.spec as SkillSpec).collection === item.name)
        .flatMap(addToAllRows);
      const result = await batchApply.applyBatch({ actionLabel: t("添加"), rows, silent: true });
      const failed = result?.rows.filter((item) => item.status === "failed").length ?? 0;
      if (failed || (rows.length > 0 && !result)) {
        toast.warning(t("{0} 来源已添加，但技能应用未全部完成，请查看批量结果", item.name));
      } else {
        toast.info(t("{0} 已添加并启用。推荐规则需在规则页预览后单独应用。", item.name));
      }
    } catch (error) {
      notifyError(t("来源操作失败"), error);
    } finally {
      setEnablingBusy(false);
      setEnabling(null);
      await queryClient.invalidateQueries({ queryKey: engineKeys.all });
    }
  };

  /** 纳入管理：执行后提示里给「撤销」（按备份清单放回原目录）。 */
  const runAdopt = async (name: string, args: string[], label: string, runId: string): Promise<boolean> => {
    const envelope = await execute({ command: "skills:adopt", label, args, silent: true, runId });
    await queryClient.invalidateQueries({ queryKey: engineKeys.all });
    if (!envelope?.ok) {
      notifyError(t("来源操作失败"), envelope?.errors[0]?.message ?? t("详见任务记录"));
      return false;
    }
    const data = envelope.data as { backup?: string; removed?: string[]; linked?: string[] };
    // 用户可能已「在后台继续」并打开了别的纳入：只关这一个
    setAdopting((current) => (current === name ? null : current));
    toast.success(t("{0}：完成", label), {
      description: t("移走 {0} 处{1}", data.removed?.length ?? 0, data.linked?.length ? t("，链接到 {0}", data.linked.map(nameOfAccount).join("、")) : ""),
      action: data.backup
        ? {
            label: t("撤销"),
            onClick: () =>
              void execute({ command: "skills:adopt", label: t("撤回：{0}", label), args: ["--undo", data.backup!] }).then(() =>
                queryClient.invalidateQueries({ queryKey: engineKeys.all }),
              ),
          }
        : undefined,
    });
    return true;
  };

  const collectionStatus = (collection: EngineElement): { text: string; tone: keyof typeof TONE } => {
    const observed = collection.observed as SkillCollectionObserved;
    const spec = collection.spec as SkillCollectionSpec;
    if ((collection.policy as { enabled?: boolean }).enabled === false) return { text: t("已停用"), tone: "muted" };
    if (!observed.repo_cached) return { text: spec.source_path ? t("目录不存在") : t("尚未下载"), tone: "warn" };
    if (observed.discovery_error) return { text: t("读取失败"), tone: "warn" };
    if (observed.version?.track_error) return { text: t("跟踪方式有误"), tone: "warn" };
    if (collection.freshness.state === "behind") return { text: t("可更新"), tone: "info" };
    return { text: t("正常"), tone: "ok" };
  };
  const total = collections.length + userSources.sources.length;
  const highlight = (name: string) => focus === name && "ring-2 ring-inset ring-primary/40";

  return (
    <div className="space-y-3">
      {batchApply.result && <BatchResultPanel batch={batchApply.result} busy={batchApply.executing} onRetry={() => void batchApply.retryFailed(batchApply.result!)} />}
      <h1 className="text-lg font-semibold tracking-tight">{t("来源")}</h1>
      <div className="flex flex-wrap items-center justify-end gap-2">
        {updatable.length > 0 && (
          <button
            type="button"
            data-testid="sources-update-all"
            disabled={busy || checking}
            title={checking ? CHECKING_TITLE : t("逐个取回来源的新内容，并把各助手在用的技能换到新版本；新发现的技能不会自动添加，助手上停用的不动")}
            onClick={() => void sourceUpdate.update(updatable)}
            className="inline-flex items-center gap-1 rounded-md border px-3 py-1.5 text-sm hover:bg-accent disabled:opacity-40"
          >
            <RefreshCw className={cn("h-4 w-4", sourceUpdate.updating && "animate-spin")} aria-hidden /> {sourceUpdate.updating ? t("正在更新…") : t("全部更新")}
          </button>
        )}
        <button type="button" data-testid="source-add-collection" onClick={() => setAdding("collection")} className="inline-flex shrink-0 items-center gap-1 whitespace-nowrap rounded-md border px-3 py-1.5 text-sm hover:bg-accent disabled:opacity-40">
          <Plus className="h-4 w-4" aria-hidden />{(" " + t("添加 Git 仓库或目录"))}</button>
        <button type="button" data-testid="source-add-rule" onClick={() => setAdding("rule")} className="inline-flex shrink-0 items-center gap-1 whitespace-nowrap rounded-md border px-3 py-1.5 text-sm hover:bg-accent disabled:opacity-40">
          <Plus className="h-4 w-4" aria-hidden />{(" " + t("添加规则文件或目录"))}</button>
        {onImportLink && (
          <button type="button" data-testid="source-import-link" onClick={onImportLink} className="inline-flex shrink-0 items-center gap-1 whitespace-nowrap rounded-md border px-3 py-1.5 text-sm hover:bg-accent disabled:opacity-40">
            <Link2 className="h-4 w-4" aria-hidden />{(" " + t("从链接导入"))}</button>
        )}
      </div>
      {checking && (
        <p data-testid="sources-checking" className="rounded-md border border-info/30 bg-info/5 px-3 py-2 text-xs text-info">{t("正在检查新版本：完成前先不能更新或换版本，免得按旧的检查结果操作。")}</p>
      )}
      <DataTable label={t("来源")} testId="sources-page">
        <div className="flex items-center gap-4 border-b bg-muted px-4 py-2 text-xs text-muted-foreground">
          <span className="min-w-0 flex-1 font-medium text-foreground">{(t("来源 ·") + " ")}{total}</span>
          <span className={COL_PROVIDES}>{t("提供")}</span>
          <span className={COL_NARROW}>{t("状态")}</span>
          <span className={ACTIONS} />
        </div>
        {suggested.map((item) => (
          <div key={item.name} data-testid={`source-suggest-${item.name}`} className={cn(ROW, "border-b bg-primary/5")}>
            <div className="min-w-0 flex-1">
              <div className="flex items-center gap-2 text-sm font-medium">
                {item.name}
                <span className="rounded bg-primary/15 px-1.5 py-0.5 text-[11px] font-normal text-primary">{t("推荐")}</span>
              </div>
              <div className="break-words text-xs text-muted-foreground">{item.description === "提供代码审查、方案设计、改动验证、文档写作等通用技能。推荐规则可在规则页预览后单独应用。"
                ? t("提供代码审查、方案设计、改动验证、文档写作等通用技能。推荐规则可在规则页预览后单独应用。") : item.description}</div>
            </div>
            <div className={ACTIONS}>
              <button type="button" data-testid={`source-suggest-enable-${item.name}`} onClick={() => setEnabling(item)} className="shrink-0 whitespace-nowrap rounded-md bg-primary px-2.5 py-1 text-xs text-primary-foreground hover:bg-primary/90">{t("添加并启用")}</button>
              <ActionMenu
                label={t("{0} 的更多操作", item.name)}
                items={[
                  { label: t("仅添加来源（不下载）"), testId: `source-suggest-add-${item.name}`, onSelect: () => setDraft({ name: item.name, definition: item.definition }) },
                  { label: t("不再推荐"), testId: `source-suggest-dismiss-${item.name}`, onSelect: () => setDismissed(dismissRecommendation(item.name)) },
                ]}
              />
            </div>
          </div>
        ))}
        {total === 0 ? (
          <EmptyState compact title={t("尚未添加来源（可选）")} description={t("不添加也可管理本机扩展。需要更多技能或规则时，再选择仓库、本机目录或规则文件。")} testId="sources-empty" />
        ) : (
          <ul className="divide-y">
            {collections.map((collection) => {
              const name = nameOf(collection);
              const spec = collection.spec as SkillCollectionSpec;
              const observed = collection.observed as SkillCollectionObserved;
              const actions = observed.actions ?? [];
              const enabled = (collection.policy as { enabled?: boolean }).enabled !== false;
              const status = collectionStatus(collection);
              const attention = status.tone === "info" || status.tone === "warn";
              return (
                <li key={collection.id} data-testid={`source-row-${name}`} data-source-anchor={name} className={cn(ROW, highlight(name))}>
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2 text-sm font-medium">
                      {name}
                      {spec.url && observed.version?.current && (
                        <span data-testid={`source-version-${name}`} className="rounded bg-muted px-1.5 py-0.5 font-mono text-[11px] font-normal text-muted-foreground" title={t("跟踪：{0}", describeTrack(spec.track))}>
                          {versionLabel(observed.version.current)}
                        </span>
                      )}
                    </div>
                    <div className="break-all font-mono text-xs text-muted-foreground">
                      {spec.source_path ? t("本机目录 · {0}", spec.source_path) : spec.url ? `git · ${spec.url}` : t("来源定义缺失")}
                    </div>
                    {(observed.discovery_error || observed.version?.track_error || observed.blocked_reason) && (
                      <div data-testid={`source-problem-${name}`} className="break-words text-xs text-warning">
                        {observed.discovery_error?.message ?? observed.version?.track_error ?? observed.blocked_reason}
                      </div>
                    )}
                  </div>
                  <span className={cn(COL_PROVIDES, "text-sm text-muted-foreground")}>
                    {skillCount(name)}{(" " + t("个技能 ·") + " ")}{ruleCount(name)}{(" " + t("条规则"))}</span>
                  <span className={cn(COL_NARROW, "text-xs", TONE[status.tone])} title={collection.freshness.upstream ? t("可更新到 {0}", shortRef(collection.freshness.upstream)) : undefined}>
                    {status.text}
                  </span>
                  {/* 一个主操作常驻，其余收进「⋯」（设计规则：每行一个主操作） */}
                  <div className={ACTIONS}>
                    {actions.includes("update") && enabled && (
                      <button type="button" data-testid={`source-update-${name}`} disabled={busy || checking} title={checking ? CHECKING_TITLE : undefined} onClick={() => updateOne(collection)} className={attention ? PILL : TEXT_BUTTON}>
                        {spec.source_path ? t("重新扫描") : t("更新")}
                      </button>
                    )}
                    <ActionMenu
                      label={t("{0} 的更多操作", name)}
                      items={[
                        ...(spec.url
                          ? [
                              { label: t("切换版本"), testId: `source-versions-${name}`, disabled: busy || checking, title: checking ? CHECKING_TITLE : undefined, onSelect: () => setVersionsOf({ collection, openLatest: false }) },
                              { label: t("复制分享链接"), testId: `source-share-${name}`, onSelect: () => void share(name, spec) },
                            ]
                          : []),
                        ...(spec.source_path || spec.url ? [{ label: t("编辑来源"), testId: `source-edit-${name}`, onSelect: () => setEditing(collection) }] : []),
                        ...(actions.includes(enabled ? "disable" : "enable")
                          ? [
                              {
                                label: enabled ? t("停用来源") : t("启用来源"),
                                testId: `source-toggle-${name}`,
                                disabled: busy,
                                title: enabled ? t("停用：各助手里它的技能链接先移除，规则暂不出现；重新启用即恢复") : t("启用：按之前的分配重新链接"),
                                onSelect: () => toggle(collection, !enabled),
                              },
                            ]
                          : []),
                        ...(actions.includes("remove") ? [{ label: t("移除来源"), testId: `source-remove-${name}`, danger: true, disabled: busy, onSelect: () => setRemovingCollection(collection) }] : []),
                      ]}
                    />
                  </div>
                </li>
              );
            })}
            {userSources.sources.map((source) => {
              const count = ruleCount(source.name);
              return (
                <li key={`rule:${source.name}`} data-testid={`source-row-${source.name}`} data-source-anchor={source.name} className={cn(ROW, highlight(source.name))}>
                  <div className="min-w-0 flex-1">
                    <div className="text-sm font-medium">{source.name}</div>
                    <div className="break-all font-mono text-xs text-muted-foreground">{(t("规则 ·") + " ")}{source.path}</div>
                  </div>
                  <span className={cn(COL_PROVIDES, "text-sm text-muted-foreground")}>{source.enabled ? t("{0} 条规则", count) : "—"}</span>
                  <span className={cn(COL_NARROW, "text-xs", !source.enabled ? TONE.muted : count ? TONE.ok : TONE.warn)}>
                    {!source.enabled ? t("已停用") : count ? t("正常") : t("未发现规则")}
                  </span>
                  <div className={ACTIONS}>
                    {source.enabled && count === 0 && (
                      <button type="button" data-testid={`source-edit-${source.name}`} disabled={userSources.busy} onClick={() => setEditingRuleSource(source)} className={PILL}>{t("修改路径")}</button>
                    )}
                    <ActionMenu
                      label={t("{0} 的更多操作", source.name)}
                      items={[
                        ...(source.enabled && count === 0 ? [] : [{ label: t("修改路径"), testId: `source-edit-${source.name}`, disabled: userSources.busy, onSelect: () => setEditingRuleSource(source) }]),
                        {
                          label: source.enabled ? t("停用来源") : t("启用来源"),
                          testId: `source-toggle-${source.name}`,
                          disabled: userSources.busy,
                          title: source.enabled ? t("停用：它的规则暂不出现，已写进助手的规则文件不动") : t("启用"),
                          onSelect: () => void userSources.setEnabled(source, !source.enabled),
                        },
                        { label: t("移除来源"), testId: `source-remove-${source.name}`, danger: true, disabled: userSources.busy, onSelect: () => setRemovingSource(source) },
                      ]}
                    />
                  </div>
                </li>
              );
            })}
          </ul>
        )}
      </DataTable>
      <p className="px-1 text-xs text-muted-foreground">{t("在此管理技能与规则的来源，以及更新与版本切换。")}</p>

      {/* 未纳管：在助手里、但不来自任何来源的技能——识别出来源的纳入（只留一份源），外部目录登记为本机目录来源，残留链接清理 */}
      <div data-source-anchor="unmanaged" className={cn("space-y-3 rounded-lg", focus === "unmanaged" && "ring-2 ring-primary/40")}>
        {unmanaged && <SkillOtherSources view={unmanaged} groups={groupsOf(agentIds)} search="" agentFilter="all" onAdopt={setAdopting} />}
        <DiscoverPanel />
      </div>
      {adopting && (
        <SkillAdoptDialog
          name={adopting}
          nameOf={nameOfAccount}
          onConfirm={(args, label, runId) => runAdopt(adopting, args, label, runId)}
          onClose={() => setAdopting(null)}
        />
      )}

      {versionsOf && (
        <SkillVersionsDialog
          collectionId={versionsOf.collection.id}
          name={nameOf(versionsOf.collection)}
          busy={switching || batchApply.executing}
          openLatest={versionsOf.openLatest}
          nameOf={nameOfAccount}
          onSwitch={(ref, label) => switchVersion(versionsOf.collection, ref, label)}
          onClose={() => setVersionsOf(null)}
        />
      )}
      {(adding === "collection" || editing || draft) && (
        <SkillCollectionFormDialog
          agentIds={agentIds}
          draft={draft ?? undefined}
          initial={editing ? { name: nameOf(editing), definition: editing.spec } : undefined}
          fullDefinition={editing ? fullSkillDefinition(layers, nameOf(editing)) : null}
          available={editing ? availableSkills(editing.observed as SkillCollectionObserved) : undefined}
          repoCached={editing ? (editing.observed as SkillCollectionObserved).repo_cached : undefined}
          onClose={() => {
            setAdding(null);
            setEditing(null);
            setDraft(null);
          }}
          onSubmit={(name, definition, complete) => {
            const isNew = !editing;
            setAdding(null);
            setEditing(null);
            setDraft(null);
            void definitionOps.saveDefinition({ name, definition, isNew, complete });
          }}
        />
      )}
      {(adding === "rule" || editingRuleSource) && (
        <RuleSourceFormDialog
          initial={editingRuleSource ?? undefined}
          taken={userSources.taken}
          busy={userSources.busy}
          onClose={() => { setAdding(null); setEditingRuleSource(null); }}
          onSubmit={async (name, path) => {
            const saved = editingRuleSource
              ? await userSources.updatePath(editingRuleSource, path)
              : await userSources.add(name, path);
            if (saved) { setAdding(null); setEditingRuleSource(null); }
            return saved;
          }}
        />
      )}
      {removingCollection && (
        <RemoveCollectionDialog
          name={nameOf(removingCollection)}
          local={Boolean((removingCollection.spec as SkillCollectionSpec).source_path)}
          executing={directApply.executing}
          onConfirm={(purge) => {
            const collection = removingCollection;
            setRemovingCollection(null);
            dismissIfRecommended(collection);
            void directApply.apply({
              command: "skills:apply",
              label: t("移除来源 {0}", nameOf(collection)),
              args: ["--only", collection.id, "--action", "remove", ...(purge ? ["--purge"] : [])],
            });
          }}
          onClose={() => setRemovingCollection(null)}
        />
      )}
      {removingSource && (
        <RemoveSourceDialog
          source={removingSource}
          busy={userSources.busy}
          onConfirm={async () => {
            if (await userSources.remove(removingSource)) setRemovingSource(null);
          }}
          onClose={() => setRemovingSource(null)}
        />
      )}
      {enabling && (
        <ConfirmModal
          title={t("添加并启用 {0}", enabling.name)}
          description={t("确认后将添加来源、下载仓库，并应用其中的技能。")}
          risk="write-local"
          confirmLabel={t("添加并启用")}
          executing={enablingBusy}
          onConfirm={() => void enableRecommended(enabling)}
          onClose={() => !enablingBusy && setEnabling(null)}
        >
          <ol data-testid="recommend-enable-steps" className="mt-3 list-decimal space-y-1.5 pl-5 text-sm">
            <li>{(t("添加来源") + " ")}{enabling.name}（{enabling.url}）</li>
            <li>{t("将仓库下载到本工具的数据目录")}</li>
            <li>{t("将技能链接到所有已有扩展目标；已有同名目录或链接时跳过，不覆盖")}</li>
            <li className="text-muted-foreground">{t("规则、MCP 和启动命令保持不变。推荐规则可在规则页预览后单独应用。")}</li>
          </ol>
          <p className="mt-2 text-xs text-muted-foreground">{t("停用或移除来源时，将移除对应的技能链接。")}</p>
        </ConfirmModal>
      )}
    </div>
  );
}
