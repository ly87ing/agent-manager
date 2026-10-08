import { useState, type ReactNode } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { ArrowUpDown, Copy, Eye, Library, Pencil, Plus, Trash2 } from "lucide-react";
import { AgentIcon } from "../components/AgentIcon";
import { StateBadge } from "../components/StateBadge";
import { COL_ACTION, COL_ICONS, COL_MIDDLE, PILL, ROW, RowIcon, TableHeader } from "../components/DataTable";
import { ConfirmModal } from "../components/ConfirmModal";
import { useRuleSources } from "../components/RuleSources";
import { ErrorPanel } from "../components/ErrorPanel";
import { PageError, PageLoading } from "../components/PageState";
import type { EngineElement, RuleSourceRule, SuggestedSource, RuleTemplate, StatusData } from "../lib/contract";
import type { RulesObserved, RulesSpec } from "../lib/observed";
import { useDialogFocus } from "../lib/dialogFocus";
import { engineErrorMessage, runEngine } from "../lib/engine";
import { engineKeys, useEngineQuery } from "../lib/queries";
import { newRunId } from "../lib/runId";
import { diffLines, hasChanges } from "../lib/textDiff";
import { FileDiff } from "../components/FileDiff";
import { type RulesFillRow, rulesFillArgs, rulesFillRows } from "../lib/rulesFill";
import { useAccountNames } from "../lib/useAccountNames";
import { useDirectApply } from "../lib/useDirectApply";
import { cn } from "../lib/utils";

const PAGE_NOTE =
  "编辑直接写入规则文件。模板保存在本机，推荐规则来自技能来源；覆盖未知内容前会自动备份。";

/** 写入前被外部改过时引擎给出的固定说法。 */
const EXTERNAL_EDIT = "已在外部修改";

const ORIGIN_LABEL: Record<string, string> = {
  user: "自建",
  backup: "备份",
  legacy: "旧来源",
};

function specOf(element: EngineElement): RulesSpec {
  return element.spec as unknown as RulesSpec;
}

function observedOf(element: EngineElement): RulesObserved {
  return element.observed as unknown as RulesObserved;
}

function nameOf(element: EngineElement): string {
  return specOf(element).name ?? element.id;
}

function isWritable(element: EngineElement): boolean {
  return element.actions.some((action) => action.id === "write") && typeof observedOf(element).base === "string";
}

function lineCount(text: string): number {
  if (text === "") return 0;
  return text.endsWith("\n") ? text.split("\n").length - 1 : text.split("\n").length;
}

/** 执行失败时给弹窗内展示的说明；外部修改额外说明页面已刷新。 */
function failureText(message: string): string {
  if (message.includes(EXTERNAL_EDIT)) {
    return `${message}。页面已刷新为文件的最新内容，请关闭后重新打开再操作。`;
  }
  return message;
}

/** 一种「写入内容」的来源：已保存模板、来源规则或另一个目标的实际内容。 */
interface ContentSource {
  key: string;
  label: string;
  body: string;
  /** 追加到 --only/--base 之后的参数。 */
  args: string[];
}

type DialogState =
  | { kind: "edit"; element: EngineElement }
  | { kind: "apply"; title: string; targets: EngineElement[]; sources: ContentSource[] }
  | { kind: "template"; template: RuleTemplate | null }
  | { kind: "delete-template"; template: RuleTemplate };

function sourceRuleLabel(rule: RuleSourceRule): string {
  return `规则「${rule.name}」（来自 ${rule.collection}）`;
}

export default function RulesPage({
  onOpenSource,
}: {
  /** 来源的添加、更新、移除都在来源页（名称；null 打开来源页）。 */
  onOpenSource?: (name: string | null) => void;
} = {}) {
  const query = useEngineQuery<StatusData>("rules:status");
  const [dialog, setDialog] = useState<DialogState | null>(null);
  const directApply = useDirectApply();
  const accountName = useAccountNames();

  if (query.isPending) return <PageLoading />;
  if (query.isError) return <PageError title="规则状态读取失败" error={query.error} />;
  if (!query.data.ok) {
    return (
      <div className="space-y-4">
        <h1 className="text-lg font-semibold tracking-tight">规则</h1>
        <ErrorPanel title="规则状态读取失败" errors={query.data.errors} />
      </div>
    );
  }

  // 与账号页同序：Claude 在前、Codex 在后；每个助手主号在前
  const elements = [...query.data.data.elements].sort(
    (a, b) =>
      Number(specOf(a).kind === "codex") - Number(specOf(b).kind === "codex") ||
      Number(Boolean(specOf(a).profile_id)) - Number(Boolean(specOf(b).profile_id)),
  );
  const templates = query.data.data.templates ?? [];
  const sourceRules = query.data.data.source_rules ?? [];
  const suggested = query.data.data.suggested_sources ?? [];
  const writable = elements.filter(isWritable);

  const templateSource = (template: RuleTemplate): ContentSource => ({
    key: `template:${template.id}`,
    label: `模板「${template.name}」`,
    body: template.body,
    args: ["--template", template.id],
  });
  const ruleSource = (rule: RuleSourceRule): ContentSource => ({
    key: `rule:${rule.id}`,
    label: sourceRuleLabel(rule),
    body: rule.body,
    args: ["--source-rule", rule.id],
  });

  /** 文件已与来源规则一致：只记下「跟随这条来源规则」，文件不动。 */
  const follow = (element: EngineElement, rule: RuleSourceRule) => {
    const base = observedOf(element).base;
    if (typeof base !== "string") return;
    void directApply
      .apply({
        command: "rules:apply",
        label: `${nameOf(element)} 跟随${sourceRuleLabel(rule)}`,
        args: ["--only", element.id, "--base", base, "--source-rule", rule.id],
      })
      .then((envelope) => {
        if (envelope?.ok) toast.info(`已跟随${sourceRuleLabel(rule)}：来源出新版本时，这里会提示更新`);
      });
  };

  const pendingFill = rulesFillRows(elements);
  /** 补齐一个账号：还没有规则文件时从同类主账号复制；引擎写前再核对文件仍不存在。 */
  const fillOne = (row: RulesFillRow) =>
    void directApply.apply({ command: "rules:apply", label: `补齐 ${row.name}`, args: rulesFillArgs(row) });

  /** 账号名：主账号用助手名，实例用账号名。 */
  const labelOf = (element: EngineElement) => {
    const spec = specOf(element);
    const account = spec.profile_id ? accountName(spec.profile_id) : spec.kind === "codex" ? accountName("codex") : accountName("claude-code");
    return `${account} · ${spec.kind === "codex" ? "AGENTS.md" : "CLAUDE.md"}`;
  };

  const followerCount = (match: (observed: RulesObserved) => boolean) => elements.filter((element) => match(observedOf(element))).length;

  return (
    <div className="space-y-3">
      <h1 className="text-lg font-semibold tracking-tight">规则</h1>
      {query.data.errors.length > 0 && <ErrorPanel title="规则状态读取失败" errors={query.data.errors} />}

      <section tabIndex={0} className="table-scroll rounded-lg border bg-card" aria-label="规则文件">
        <TableHeader title={`规则文件 · ${elements.length}`} middle="使用的规则" />
        {elements.length === 0 && <p className="px-4 py-3 text-xs text-muted-foreground">没有可显示的规则文件。</p>}
        <ul className="divide-y">
          {elements.map((element) => {
            const others = writable.filter((item) => item.id !== element.id);
            const content = observedOf(element).content;
            return (
              <TargetRow
                key={element.id}
                element={element}
                label={labelOf(element)}
                fill={pendingFill.find((row) => row.elementId === element.id)}
                onFill={fillOne}
                sourceRules={sourceRules}
                busy={directApply.executing}
                onEdit={() => setDialog({ kind: "edit", element })}
                onSwitch={
                  templates.length > 0 || sourceRules.length > 0
                    ? () =>
                        setDialog({
                          kind: "apply",
                          title: `更换 ${nameOf(element)} 的规则`,
                          targets: [element],
                          sources: [...sourceRules.map(ruleSource), ...templates.map(templateSource)],
                        })
                    : undefined
                }
                onApplyRule={(rule) =>
                  setDialog({ kind: "apply", title: `用${sourceRuleLabel(rule)}更新 ${nameOf(element)}`, targets: [element], sources: [ruleSource(rule)] })
                }
                onFollow={(rule) => follow(element, rule)}
                onCopy={
                  typeof content === "string" && others.length > 0
                    ? () =>
                        setDialog({
                          kind: "apply",
                          title: `复制 ${nameOf(element)} 的内容`,
                          targets: others,
                          sources: [{ key: `from:${element.id}`, label: nameOf(element), body: content, args: ["--from", element.id] }],
                        })
                    : undefined
                }
              />
            );
          })}
        </ul>
      </section>
      <p className="px-1 text-xs text-muted-foreground">{PAGE_NOTE}</p>

      <RuleLibrary
        sourceRules={sourceRules}
        templates={templates}
        suggested={suggested}
        usage={{
          rule: (rule) => followerCount((observed) => (observed.source_follow?.rule ?? observed.matched_source_rule?.id) === rule.id),
          template: (template) => followerCount((observed) => !observed.source_follow && observed.matched_template?.id === template.id),
        }}
        onApplyRule={
          writable.length > 0
            ? (rule) => setDialog({ kind: "apply", title: `应用${sourceRuleLabel(rule)}`, targets: writable, sources: [ruleSource(rule)] })
            : undefined
        }
        onApplyTemplate={
          writable.length > 0
            ? (template) => setDialog({ kind: "apply", title: `应用模板「${template.name}」`, targets: writable, sources: [templateSource(template)] })
            : undefined
        }
        onNewTemplate={() => setDialog({ kind: "template", template: null })}
        onEditTemplate={(template) => setDialog({ kind: "template", template })}
        onDeleteTemplate={(template) => setDialog({ kind: "delete-template", template })}
        onOpenSource={onOpenSource}
      />

      {dialog?.kind === "edit" && <EditDialog element={dialog.element} onClose={() => setDialog(null)} />}
      {dialog?.kind === "apply" && (
        <ApplyContentDialog title={dialog.title} targets={dialog.targets} sources={dialog.sources} onClose={() => setDialog(null)} />
      )}
      {dialog?.kind === "template" && <TemplateDialog template={dialog.template} onClose={() => setDialog(null)} />}
      {dialog?.kind === "delete-template" && <DeleteTemplateDialog template={dialog.template} onClose={() => setDialog(null)} />}
    </div>
  );
}

/** 这个账号的规则文件用的是哪条、状态如何。 */
function ruleStatus(element: EngineElement, sourceRules: RuleSourceRule[]) {
  const observed = observedOf(element);
  const follow = observed.source_follow;
  if (element.state === "disabled") return { tone: "muted" as const, note: "已停用" };
  if (!isWritable(element)) return { tone: "muted" as const, note: "不可用", detail: observed.blocked_reason ?? undefined };
  if (observed.content === null || observed.base === "missing") return { tone: "muted" as const, note: "还没有规则文件" };
  if (follow) {
    const rule = sourceRules.find((item) => item.id === follow.rule);
    const name = rule?.name ?? follow.rule.split("/").pop();
    const source = rule?.collection ?? follow.rule.split("/")[0];
    const changedSinceFollow = !follow.file_unchanged && Boolean(follow.current_revision) && follow.current_revision !== follow.applied_revision;
    // 统一状态（lib/inbox.ts）：与待处理页同一套标签
    if (follow.update_available) return { name, source, tone: "warn" as const, state: "update" as const, note: "有新版本", testId: "rules-update-available" };
    if (changedSinceFollow) return { name, source, tone: "warn" as const, state: "drifted" as const, note: "被改过，来源有新版", testId: "rules-changed-since-follow" };
    if (follow.current_revision === null) return { name, source, tone: "muted" as const, note: "来源已不在" };
    if (!follow.file_unchanged) return { name, source, tone: "muted" as const, note: "改过" };
    return { name, source, tone: "ok" as const, note: "最新" };
  }
  if (observed.matched_source_rule) {
    return { name: observed.matched_source_rule.name, source: observed.matched_source_rule.collection, tone: "ok" as const, note: "一致" };
  }
  if (observed.matched_template) return { name: observed.matched_template.name, source: "本机模板", tone: "ok" as const, note: "一致" };
  return { tone: "muted" as const, note: "自定义内容" };
}

const DOT = { ok: "bg-success", warn: "bg-warning", muted: "bg-muted-foreground/50" } as const;

function TargetRow({
  element,
  label,
  sourceRules,
  busy,
  onEdit,
  onSwitch,
  onApplyRule,
  onFollow,
  onCopy,
  fill,
  onFill,
}: {
  element: EngineElement;
  label: string;
  fill?: RulesFillRow;
  onFill: (row: RulesFillRow) => void;
  sourceRules: RuleSourceRule[];
  busy: boolean;
  onEdit: () => void;
  /** 先看差异，再换成规则库里的另一条。 */
  onSwitch?: () => void;
  onApplyRule: (rule: RuleSourceRule) => void;
  onFollow: (rule: RuleSourceRule) => void;
  onCopy?: () => void;
}) {
  const [open, setOpen] = useState(false);
  const spec = specOf(element);
  const observed = observedOf(element);
  const writable = isWritable(element);
  const status = ruleStatus(element, sourceRules);
  const follow = observed.source_follow;
  const followedRule = follow ? sourceRules.find((rule) => rule.id === follow.rule) : undefined;
  const matchedRule = observed.matched_source_rule ? sourceRules.find((rule) => rule.id === observed.matched_source_rule?.id) : undefined;
  const content = observed.content;
  // 需要处理时的那一个按钮：补齐 > 更新 > 跟随
  const action = !writable
    ? null
    : fill
      ? { testId: `rules-fill-${element.id}`, label: "补齐", title: "还没有规则文件：从同一助手的主号复制过来", run: () => onFill(fill) }
      : followedRule && status.testId
        ? {
            testId: `rules-update-from-source-${element.id}`,
            label: follow?.update_available ? "更新" : "查看新版本",
            title: "先看差异，确认后写入；覆盖前先备份现有内容",
            run: () => onApplyRule(followedRule),
          }
        : matchedRule && !follow
          ? { testId: `rules-follow-${element.id}`, label: "跟随更新", title: "文件不动，只记下它来自这条规则；来源出新版本时这里会提示", run: () => onFollow(matchedRule) }
          : null;
  return (
    <li data-testid={`rules-target-${element.id}`}>
      <div className={ROW}>
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-1.5 text-sm font-medium" title={nameOf(element)}>
            <AgentIcon kind={spec.kind === "codex" ? "codex" : "claude"} />
            <span className="truncate">{label}</span>
          </div>
          <div className="truncate font-mono text-xs text-muted-foreground" title={spec.target ?? observed.target}>
            {spec.target ?? observed.target}
            {typeof content === "string" && <span className="font-sans"> · {lineCount(content)} 行</span>}
          </div>
        </div>
        <div className={cn(COL_MIDDLE, "min-w-0 text-sm")}>
          <div className="flex items-center gap-1.5">
            <span className={cn("h-1.5 w-1.5 shrink-0 rounded-full", DOT[status.tone])} aria-hidden />
            {status.name && <span className="font-medium">{status.name}</span>}
            {status.source && <span className="truncate text-muted-foreground">· {status.source}</span>}
            {"state" in status && status.state ? (
              <StateBadge state={status.state} testId={status.testId ? `${status.testId}-${element.id}` : undefined}>{status.note}</StateBadge>
            ) : (
              <span className="shrink-0 text-muted-foreground">{status.name ? `· ${status.note}` : status.note}</span>
            )}
          </div>
          {status.detail && <p className="truncate text-xs text-warning" title={status.detail}>{status.detail}</p>}
        </div>
        <div className={COL_ACTION}>
          {action && (
            <button type="button" data-testid={action.testId} disabled={busy} title={action.title} onClick={action.run} className={PILL}>
              {action.label}
            </button>
          )}
        </div>
        <div className={COL_ICONS}>
          {writable && (
            <>
              {onSwitch && (
                <RowIcon label={`更换 ${nameOf(element)} 的规则`} onClick={onSwitch}>
                  <ArrowUpDown className="h-3.5 w-3.5" aria-hidden />
                </RowIcon>
              )}
              {typeof content === "string" && (
                <RowIcon label={open ? `收起 ${nameOf(element)} 全文` : `查看 ${nameOf(element)} 全文`} onClick={() => setOpen(!open)}>
                  <Eye className="h-3.5 w-3.5" aria-hidden />
                </RowIcon>
              )}
              <RowIcon label={`编辑 ${nameOf(element)}`} onClick={onEdit}>
                <Pencil className="h-3.5 w-3.5" aria-hidden />
              </RowIcon>
              {onCopy && (
                <RowIcon label={`复制 ${nameOf(element)} 到…`} onClick={onCopy}>
                  <Copy className="h-3.5 w-3.5" aria-hidden />
                </RowIcon>
              )}
            </>
          )}
        </div>
      </div>
      {open && typeof content === "string" && (
        <pre data-testid={`rules-content-${element.id}`} className="mx-4 mb-2 max-h-80 overflow-auto whitespace-pre-wrap rounded border bg-muted/40 p-2 font-mono text-xs">
          {content}
        </pre>
      )}
    </li>
  );
}

/** 规则库：来源规则（技能集合、我的规则来源）与本机模板，一行一条。 */
function RuleLibrary({
  sourceRules,
  templates,
  suggested,
  usage,
  onApplyRule,
  onApplyTemplate,
  onNewTemplate,
  onEditTemplate,
  onDeleteTemplate,
  onOpenSource,
}: {
  sourceRules: RuleSourceRule[];
  templates: RuleTemplate[];
  suggested: SuggestedSource[];
  usage: { rule: (rule: RuleSourceRule) => number; template: (template: RuleTemplate) => number };
  onApplyRule?: (rule: RuleSourceRule) => void;
  onApplyTemplate?: (template: RuleTemplate) => void;
  onNewTemplate: () => void;
  onEditTemplate: (template: RuleTemplate) => void;
  onDeleteTemplate: (template: RuleTemplate) => void;
  /** 来源的更新、移除在来源页：行上的来源名点过去。 */
  onOpenSource?: (name: string | null) => void;
}) {
  const [openId, setOpenId] = useState<string | null>(null);
  const userSources = useRuleSources();
  const userSourceOf = (collection: string) => userSources.sources.find((source) => source.name === collection);
  const emptySources = userSources.sources.filter((source) => !sourceRules.some((rule) => rule.collection === source.name));
  const count = sourceRules.length + templates.length;
  const used = (n: number) => (n > 0 ? `${n} 个账号在用` : "未使用");
  const body = (id: string, text: string, testId: string) =>
    openId === id && (
      <pre data-testid={testId} className="mx-4 mb-2 max-h-80 overflow-auto whitespace-pre-wrap rounded border bg-muted/40 p-2 font-mono text-xs">
        {text}
      </pre>
    );

  return (
    <section tabIndex={0} className="table-scroll rounded-lg border bg-card" aria-label="规则库">
      <TableHeader title={`规则库 · ${count}`} middle="来源">
        <button type="button" data-testid="rule-source-add" onClick={() => onOpenSource?.(null)} className="inline-flex items-center gap-1 text-primary hover:underline">
          <Library className="h-3.5 w-3.5" aria-hidden /> 管理来源
        </button>
        <button type="button" onClick={onNewTemplate} className="inline-flex items-center gap-1 text-primary hover:underline">
          <Plus className="h-3.5 w-3.5" aria-hidden /> 新建模板
        </button>
      </TableHeader>
      <ul className="divide-y">
        {sourceRules.map((rule) => {
          const mine = userSourceOf(rule.collection);
          return (
            <li key={rule.id} data-testid={`rules-source-rule-${rule.id}`}>
              <div className={ROW}>
                <div className="min-w-0 flex-1">
                  <div className="truncate text-sm font-medium">{rule.name}</div>
                  <div className="text-xs text-muted-foreground">{used(usage.rule(rule))} · {lineCount(rule.body)} 行</div>
                </div>
                <button
                  type="button"
                  data-testid={`rules-open-source-${rule.id}`}
                  onClick={() => onOpenSource?.(rule.collection)}
                  className={cn(COL_MIDDLE, "truncate text-left text-sm text-muted-foreground hover:text-primary hover:underline")}
                  title={`到来源页管理 ${rule.collection}${mine ? `（${mine.path}）` : ""}`}
                >
                  {mine ? `规则文件 · ${mine.path}` : `${rule.collection} · ${rule.local ? "本机目录" : "git 仓库"}`}
                </button>
                <div className={COL_ACTION}>
                  {onApplyRule && (
                    <button
                      type="button"
                      aria-label={`应用${sourceRuleLabel(rule)}到…`}
                      onClick={() => onApplyRule(rule)}
                      className={PILL}
                    >
                      应用到…
                    </button>
                  )}
                </div>
                <div className={COL_ICONS}>
                  <RowIcon label={openId === rule.id ? `收起 ${rule.name}` : `查看 ${rule.name}`} onClick={() => setOpenId(openId === rule.id ? null : rule.id)}>
                    <Eye className="h-3.5 w-3.5" aria-hidden />
                  </RowIcon>
                </div>
              </div>
              {body(rule.id, rule.body, `rules-source-body-${rule.id}`)}
            </li>
          );
        })}
        {emptySources.map((source) => (
          <li key={`source:${source.name}`} data-testid={`rule-source-${source.name}`} className={ROW}>
            <div className="min-w-0 flex-1">
              <div className="truncate text-sm font-medium">{source.name}</div>
              <div className="text-xs text-warning">没读到规则</div>
            </div>
            <div className={cn(COL_MIDDLE, "truncate text-sm text-muted-foreground")} title={source.path}>规则文件 · {source.path}</div>
            <div className={COL_ACTION} />
            <div className={COL_ICONS}>
              <RowIcon label={`到来源页管理 ${source.name}`} onClick={() => onOpenSource?.(source.name)}>
                <Library className="h-3.5 w-3.5" aria-hidden />
              </RowIcon>
            </div>
          </li>
        ))}
        {templates.map((template) => (
          <li key={template.id} data-testid={`rule-template-${template.id}`}>
            <div className={ROW}>
              <div className="min-w-0 flex-1">
                <div className="truncate text-sm font-medium">{template.name}</div>
                <div className="text-xs text-muted-foreground">{used(usage.template(template))} · {lineCount(template.body)} 行</div>
              </div>
              <div className={cn(COL_MIDDLE, "text-sm text-muted-foreground")}>本机模板 · {ORIGIN_LABEL[template.origin] ?? template.origin}</div>
              <div className={COL_ACTION}>
                {onApplyTemplate && (
                  <button
                    type="button"
                    onClick={() => onApplyTemplate(template)}
                    className={PILL}
                  >
                    应用到…
                  </button>
                )}
              </div>
              <div className={COL_ICONS}>
                <RowIcon label={openId === template.id ? `收起 ${template.name}` : `查看 ${template.name}`} onClick={() => setOpenId(openId === template.id ? null : template.id)}>
                  <Eye className="h-3.5 w-3.5" aria-hidden />
                </RowIcon>
                <RowIcon label={`编辑模板 ${template.name}`} onClick={() => onEditTemplate(template)}>
                  <Pencil className="h-3.5 w-3.5" aria-hidden />
                </RowIcon>
                <RowIcon danger label={`删除模板 ${template.name}`} onClick={() => onDeleteTemplate(template)}>
                  <Trash2 className="h-3.5 w-3.5" aria-hidden />
                </RowIcon>
              </div>
            </div>
            {body(template.id, template.body, `rules-template-body-${template.id}`)}
          </li>
        ))}
        {sourceRules.length === 0 && (
          <li data-testid="rules-no-source" className="space-y-2 px-4 py-3 text-xs text-muted-foreground">
            <p>
              还没有来源提供规则：到
              <button type="button" data-testid="rules-open-sources" onClick={() => onOpenSource?.(null)} className="mx-1 text-primary underline-offset-2 hover:underline">
                来源页
              </button>
              添加规则文件，或带规则的仓库（如 agent-skills）。
            </p>
            {suggested.map((item) => (
              <div key={item.name} className="flex items-center justify-between gap-3">
                <span className="min-w-0">
                  <span className="font-medium text-foreground">{item.name}</span> · {item.description}
                </span>
                <button
                  type="button"
                  data-testid={`rules-open-source-${item.collection ?? item.name}`}
                  onClick={() => onOpenSource?.(item.collection ?? null)}
                  className={PILL}
                >
                  {item.collection ? `到来源页取回 ${item.collection}` : `到来源页添加 ${item.name}`}
                </button>
              </div>
            ))}
          </li>
        )}
      </ul>
    </section>
  );
}

function DialogFrame({
  label,
  busy,
  onClose,
  initialFocusSelector,
  children,
}: {
  label: string;
  busy: boolean;
  onClose: () => void;
  initialFocusSelector?: string;
  children: ReactNode;
}) {
  const { dialogRef, onDialogKeyDown } = useDialogFocus<HTMLElement>({
    onEscape: busy ? undefined : onClose,
    initialFocusSelector,
  });
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-slate-900/40 p-4">
      <section
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-label={label}
        tabIndex={-1}
        onKeyDown={onDialogKeyDown}
        className="max-h-[90vh] w-full max-w-3xl overflow-y-auto rounded-xl bg-card p-5 shadow-xl"
      >
        <h3 className="font-semibold">{label}</h3>
        {children}
      </section>
    </div>
  );
}

function EditDialog({ element, onClose }: { element: EngineElement; onClose: () => void }) {
  const observed = observedOf(element);
  const [text, setText] = useState(observed.content ?? "");
  const [error, setError] = useState<string | null>(null);
  const directApply = useDirectApply();
  const name = nameOf(element);

  const save = async () => {
    if (typeof observed.base !== "string") return;
    setError(null);
    const envelope = await directApply.apply({
      command: "rules:apply",
      label: `保存 ${name}`,
      args: ["--only", element.id, "--base", observed.base, "--value", text],
    });
    if (envelope?.ok) {
      onClose();
      return;
    }
    setError(failureText(envelope?.errors[0]?.message ?? "保存失败，详见任务记录"));
  };

  return (
    <DialogFrame label={`编辑 ${name}`} busy={directApply.executing} onClose={onClose} initialFocusSelector='textarea[aria-label="规则全文"]'>
      <p className="my-2 text-xs text-muted-foreground">
        保存即写入 <span className="font-mono">{observed.target}</span>；打开编辑后若文件被别处改过，保存会被拒绝，不会覆盖。
      </p>
      <textarea
        aria-label="规则全文"
        value={text}
        onChange={(event) => setText(event.target.value)}
        rows={18}
        disabled={directApply.executing}
        spellCheck={false}
        autoCorrect="off"
        autoCapitalize="off"
        className="w-full rounded border p-2 font-mono text-sm"
      />
      {error && (
        <p role="alert" className="mt-2 text-sm text-destructive">
          {error}
        </p>
      )}
      <div className="sticky -bottom-5 z-10 -mx-5 -mb-5 border-t bg-card px-5 py-3 mt-4 flex justify-end gap-2">
        <button type="button" disabled={directApply.executing} onClick={onClose} className="rounded border px-4 py-2 text-sm">
          取消
        </button>
        <button
          type="button"
          disabled={directApply.executing}
          onClick={() => void save()}
          className="rounded bg-primary px-4 py-2 text-sm text-primary-foreground disabled:opacity-60"
        >
          保存到文件
        </button>
      </div>
    </DialogFrame>
  );
}

function DiffView({ current, after }: { current: string | null; after: string }) {
  if (current === null && after === "") {
    return (
      <p data-testid="rules-diff-empty-new" className="rounded border bg-muted px-3 py-2 text-xs text-muted-foreground">
        目标还没有文件，将新建一个空的规则文件。
      </p>
    );
  }
  const before = current ?? "";
  if (before === after) {
    return (
      <p data-testid="rules-diff-same" className="rounded border bg-muted px-3 py-2 text-xs text-muted-foreground">
        与现有内容相同，不会写入。
      </p>
    );
  }
  const lines = diffLines(before, after);
  if (!hasChanges(lines)) {
    // 逐行比较看不出来的差别（只差文件末尾换行）：仍会写入，照实说
    return (
      <p data-testid="rules-diff-eol" className="rounded border bg-muted px-3 py-2 text-xs text-muted-foreground">
        各行内容相同，只有文件末尾换行不同。
      </p>
    );
  }
  // 只显示变化的行与前后几行，没变的折叠（与 MCP、快捷命令等预览同一写法）
  return <FileDiff original={current ?? null} content={after} testId="rules-diff" maxHeight="max-h-80" />;
}

function ApplyContentDialog({
  title,
  targets,
  sources,
  onClose,
}: {
  title: string;
  targets: EngineElement[];
  sources: ContentSource[];
  onClose: () => void;
}) {
  const [targetId, setTargetId] = useState(targets[0]?.id ?? "");
  const [sourceKey, setSourceKey] = useState(sources[0]?.key ?? "");
  const [error, setError] = useState<string | null>(null);
  const directApply = useDirectApply();
  const target = targets.find((item) => item.id === targetId) ?? targets[0];
  const source = sources.find((item) => item.key === sourceKey) ?? sources[0];
  if (!target || !source) return null;
  const observed = observedOf(target);
  const before = observed.content ?? "";
  // 以字节为准判断会不会写：文件不存在时即使正文为空也会新建
  const changed = typeof observed.content !== "string" || before !== source.body;

  const confirm = async () => {
    if (typeof observed.base !== "string") return;
    setError(null);
    const envelope = await directApply.apply({
      command: "rules:apply",
      label: `把${source.label}写入 ${nameOf(target)}`,
      args: ["--only", target.id, "--base", observed.base, ...source.args],
    });
    if (envelope?.ok) {
      const backedUp = (envelope.data as { steps?: { name?: string }[] }).steps?.some((step) => step.name?.startsWith("rules:backup"));
      if (backedUp) toast.info("原内容已另存为备份模板");
      onClose();
      return;
    }
    setError(failureText(envelope?.errors[0]?.message ?? "写入失败，详见任务记录"));
  };

  return (
    <DialogFrame label={title} busy={directApply.executing} onClose={onClose}>
      <div className="mt-3 grid gap-3 sm:grid-cols-2">
        <label className="space-y-1 text-xs">
          <span className="text-muted-foreground">内容来源</span>
          {sources.length > 1 ? (
            <select
              aria-label="内容来源"
              value={source.key}
              onChange={(event) => setSourceKey(event.target.value)}
              disabled={directApply.executing}
              className="w-full rounded border bg-background px-2 py-1.5 text-sm"
            >
              {sources.map((item) => (
                <option key={item.key} value={item.key}>
                  {item.label}
                </option>
              ))}
            </select>
          ) : (
            <p className="text-sm font-medium">{source.label}</p>
          )}
        </label>
        <label className="space-y-1 text-xs">
          <span className="text-muted-foreground">写入目标</span>
          {targets.length > 1 ? (
            <select
              aria-label="写入目标"
              value={target.id}
              onChange={(event) => setTargetId(event.target.value)}
              disabled={directApply.executing}
              className="w-full rounded border bg-background px-2 py-1.5 text-sm"
            >
              {targets.map((item) => (
                <option key={item.id} value={item.id}>
                  {nameOf(item)}
                </option>
              ))}
            </select>
          ) : (
            <p className="text-sm font-medium">{nameOf(target)}</p>
          )}
        </label>
      </div>
      <p className="mt-3 text-xs text-muted-foreground">
        目标现有内容 → 将写入的内容（<span className="font-mono">{specOf(target).target ?? observed.target}</span>）。现有内容对不上任何已保存模板、来源里的规则或上次从来源写入的那一版时，会先另存为备份模板再写入。
      </p>
      <div className="mt-2">
        <DiffView current={observed.content} after={source.body} />
      </div>
      {error && (
        <p role="alert" className="mt-2 text-sm text-destructive">
          {error}
        </p>
      )}
      <div className="sticky -bottom-5 z-10 -mx-5 -mb-5 border-t bg-card px-5 py-3 mt-4 flex justify-end gap-2">
        <button type="button" disabled={directApply.executing} onClick={onClose} className="rounded border px-4 py-2 text-sm">
          取消
        </button>
        <button
          type="button"
          disabled={directApply.executing || !changed}
          onClick={() => void confirm()}
          className="rounded bg-primary px-4 py-2 text-sm text-primary-foreground disabled:opacity-60"
        >
          写入
        </button>
      </div>
    </DialogFrame>
  );
}

function TemplateDialog({ template, onClose }: { template: RuleTemplate | null; onClose: () => void }) {
  const queryClient = useQueryClient();
  const [name, setName] = useState(template?.name ?? "");
  const [body, setBody] = useState(template?.body ?? "");
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  const save = async () => {
    setSaving(true);
    setError(null);
    try {
      const args = template
        ? ["--template", template.id, "--revision", template.revision, "--name", name, "--value", body]
        : ["--name", name, "--value", body];
      const envelope = await runEngine("rules:save-template", args, { runId: newRunId() });
      if (!envelope.ok) {
        setError(envelope.errors[0]?.message ?? "保存失败");
        return;
      }
      void queryClient.invalidateQueries({ queryKey: engineKeys.all });
      toast.success("模板已保存");
      onClose();
    } catch (err) {
      setError(engineErrorMessage(err));
    } finally {
      setSaving(false);
    }
  };

  return (
    <DialogFrame label={template ? `编辑模板 ${template.name}` : "新建模板"} busy={saving} onClose={onClose} initialFocusSelector='input[aria-label="模板名称"]'>
      <p className="my-2 text-xs text-muted-foreground">模板只保存在本机库里，不改任何规则文件。</p>
      <input
        aria-label="模板名称"
        value={name}
        onChange={(event) => setName(event.target.value)}
        disabled={saving}
        className="mb-2 w-full rounded border px-2 py-1.5 text-sm"
        placeholder="名称"
      />
      <textarea
        aria-label="模板正文"
        value={body}
        onChange={(event) => setBody(event.target.value)}
        rows={16}
        disabled={saving}
        spellCheck={false}
        autoCorrect="off"
        autoCapitalize="off"
        className="w-full rounded border p-2 font-mono text-sm"
      />
      {error && (
        <p role="alert" className="mt-2 text-sm text-destructive">
          {error}
        </p>
      )}
      <div className="sticky -bottom-5 z-10 -mx-5 -mb-5 border-t bg-card px-5 py-3 mt-4 flex justify-end gap-2">
        <button type="button" disabled={saving} onClick={onClose} className="rounded border px-4 py-2 text-sm">
          取消
        </button>
        <button
          type="button"
          disabled={saving || name.trim() === ""}
          onClick={() => void save()}
          className="rounded bg-primary px-4 py-2 text-sm text-primary-foreground disabled:opacity-60"
        >
          保存模板
        </button>
      </div>
    </DialogFrame>
  );
}

function DeleteTemplateDialog({ template, onClose }: { template: RuleTemplate; onClose: () => void }) {
  const queryClient = useQueryClient();
  const [executing, setExecuting] = useState(false);
  const [errors, setErrors] = useState<{ code: string; element: null; message: string }[] | null>(null);

  const confirm = async () => {
    setExecuting(true);
    try {
      const envelope = await runEngine("rules:delete-template", ["--template", template.id], { runId: newRunId() });
      if (!envelope.ok) {
        setErrors(envelope.errors.map((item) => ({ code: item.code, element: null, message: item.message })));
        return;
      }
      void queryClient.invalidateQueries({ queryKey: engineKeys.all });
      toast.success("模板已删除");
      onClose();
    } catch (err) {
      setErrors([{ code: "ERROR", element: null, message: engineErrorMessage(err) }]);
    } finally {
      setExecuting(false);
    }
  };

  return (
    <ConfirmModal
      title={`删除模板 ${template.name}`}
      description="只删除本机库里的这份模板，不改任何规则文件。"
      risk="write-local"
      confirmLabel="删除"
      executing={executing}
      resultErrors={errors}
      onConfirm={() => void confirm()}
      onClose={onClose}
    />
  );
}
