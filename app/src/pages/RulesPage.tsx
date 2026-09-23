import { useState, type ReactNode } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { ChevronDown, ChevronRight, Plus } from "lucide-react";
import { ConfirmModal } from "../components/ConfirmModal";
import { ErrorPanel } from "../components/ErrorPanel";
import { PageError, PageHeader, PageLoading } from "../components/PageState";
import type { EngineElement, RuleSourceRule, RuleSuggestedSource, RuleTemplate, StatusData } from "../lib/contract";
import type { RulesObserved, RulesSpec } from "../lib/observed";
import { useDialogFocus } from "../lib/dialogFocus";
import { engineErrorMessage, runEngine } from "../lib/engine";
import { engineKeys, useEngineQuery } from "../lib/queries";
import { newRunId } from "../lib/runId";
import { diffLines, hasChanges } from "../lib/textDiff";
import { useDirectApply } from "../lib/useDirectApply";
import { cn } from "../lib/utils";

const PAGE_NOTE =
  "以各助手的规则文件为准：这里显示并直接编辑实际文件。模板只保存在本机库里；推荐规则来自技能来源（如 agent-skills 的 rules/），不随程序发布。应用模板、来源规则或从另一处复制之前，目标现有内容若对不上任何已知内容，会先另存为一份备份模板。";

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

function stateLabel(element: EngineElement): string {
  switch (element.state) {
    case "managed-ok":
      return "已有文件";
    case "missing":
      return "还没有文件";
    case "disabled":
      return "已停用";
    default:
      return "不可用";
  }
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
  return `来源规则「${rule.name}」（${rule.collection}）`;
}

export default function RulesPage({ onOpenSkills }: { onOpenSkills?: () => void } = {}) {
  const query = useEngineQuery<StatusData>("rules:status");
  const [dialog, setDialog] = useState<DialogState | null>(null);
  const directApply = useDirectApply();

  if (query.isPending) return <PageLoading />;
  if (query.isError) return <PageError title="规则状态读取失败" error={query.error} />;
  if (!query.data.ok) {
    return (
      <div className="space-y-4">
        <PageHeader title="规则" description={PAGE_NOTE} />
        <ErrorPanel title="规则状态读取失败" errors={query.data.errors} />
      </div>
    );
  }

  const elements = query.data.data.elements;
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

  /** git 来源：取回仓库新版本（与技能页的「更新资源库」同一个动作）。 */
  const fetchSource = (collection: string) => {
    void directApply.apply({
      command: "skills:apply",
      label: `更新来源 ${collection}`,
      args: ["--only", `skill:${collection}`, "--action", "update"],
    });
  };

  return (
    <div className="space-y-4">
      <PageHeader title="规则" description={PAGE_NOTE} />
      {query.data.errors.length > 0 && <ErrorPanel title="规则状态读取失败" errors={query.data.errors} />}

      <section className="space-y-3 rounded-lg border bg-card p-5 shadow-sm" aria-label="规则文件">
        <h3 className="text-sm font-semibold">规则文件</h3>
        {elements.length === 0 && <p className="text-xs text-muted-foreground">没有可显示的规则文件。</p>}
        {elements.map((element) => {
          const others = writable.filter((item) => item.id !== element.id);
          const content = observedOf(element).content;
          return (
            <TargetCard
              key={element.id}
              element={element}
              sourceRules={sourceRules}
              busy={directApply.executing}
              onEdit={() => setDialog({ kind: "edit", element })}
              onApplyTemplate={
                templates.length > 0 || sourceRules.length > 0
                  ? () =>
                      setDialog({
                        kind: "apply",
                        title: `应用模板到 ${nameOf(element)}`,
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
      </section>

      <section className="space-y-3 rounded-lg border bg-card p-5 shadow-sm" aria-label="规则模板">
        <div className="flex items-center justify-between gap-2">
          <div>
            <h3 className="text-sm font-semibold">规则模板</h3>
            <p className="mt-1 text-xs text-muted-foreground">只保存在本机库里，保存或删除都不改任何规则文件。</p>
          </div>
          <button
            type="button"
            onClick={() => setDialog({ kind: "template", template: null })}
            className="inline-flex items-center gap-1 rounded-md border border-primary/30 px-3 py-1.5 text-xs text-primary hover:bg-primary/10"
          >
            <Plus className="h-3.5 w-3.5" aria-hidden /> 新建模板
          </button>
        </div>
        {templates.length === 0 && <p className="text-xs text-muted-foreground">还没有模板。</p>}
        <ul className="divide-y">
          {templates.map((template) => (
            <li key={template.id} data-testid={`rule-template-${template.id}`} className="flex flex-wrap items-center justify-between gap-2 py-2">
              <div className="min-w-0">
                <span className="text-sm font-medium">{template.name}</span>
                <span className="ml-2 rounded-full border px-2 py-0.5 text-[11px] text-muted-foreground">
                  {ORIGIN_LABEL[template.origin] ?? template.origin}
                </span>
                <span className="ml-2 text-[11px] text-muted-foreground">{lineCount(template.body)} 行</span>
              </div>
              <div className="flex gap-2">
                {writable.length > 0 && (
                  <button
                    type="button"
                    onClick={() =>
                      setDialog({ kind: "apply", title: `应用模板「${template.name}」`, targets: writable, sources: [templateSource(template)] })
                    }
                    className="rounded border border-primary/30 px-2.5 py-1 text-xs text-primary hover:bg-primary/10"
                  >
                    应用到…
                  </button>
                )}
                <button
                  type="button"
                  aria-label={`编辑模板 ${template.name}`}
                  onClick={() => setDialog({ kind: "template", template })}
                  className="rounded border px-2.5 py-1 text-xs hover:bg-accent"
                >
                  编辑
                </button>
                <button
                  type="button"
                  aria-label={`删除模板 ${template.name}`}
                  onClick={() => setDialog({ kind: "delete-template", template })}
                  className="rounded border border-destructive/30 px-2.5 py-1 text-xs text-destructive hover:bg-destructive/10"
                >
                  删除
                </button>
              </div>
            </li>
          ))}
        </ul>
      </section>

      <SourceRulesSection
        rules={sourceRules}
        suggested={suggested}
        busy={directApply.executing}
        onApply={
          writable.length > 0
            ? (rule) => setDialog({ kind: "apply", title: `应用${sourceRuleLabel(rule)}`, targets: writable, sources: [ruleSource(rule)] })
            : undefined
        }
        onFetch={fetchSource}
        onOpenSkills={onOpenSkills}
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

function TargetCard({
  element,
  sourceRules,
  busy,
  onEdit,
  onApplyTemplate,
  onApplyRule,
  onFollow,
  onCopy,
}: {
  element: EngineElement;
  sourceRules: RuleSourceRule[];
  busy: boolean;
  onEdit: () => void;
  onApplyTemplate?: () => void;
  /** 用某条来源规则的当前版本覆盖（先看差异）。 */
  onApplyRule: (rule: RuleSourceRule) => void;
  /** 文件已与来源规则一致：记下跟随。 */
  onFollow: (rule: RuleSourceRule) => void;
  onCopy?: () => void;
}) {
  const [open, setOpen] = useState(false);
  const spec = specOf(element);
  const observed = observedOf(element);
  const writable = isWritable(element);
  const hasContent = typeof observed.content === "string";
  const follow = observed.source_follow;
  const followedRule = follow ? sourceRules.find((rule) => rule.id === follow.rule) : undefined;
  const matchedRule = observed.matched_source_rule
    ? sourceRules.find((rule) => rule.id === observed.matched_source_rule?.id)
    : undefined;
  // 来源出了新版，但这份文件在上次写入后被改过：只提示，要不要覆盖由用户看差异后决定
  const changedSinceFollow = Boolean(
    follow && !follow.file_unchanged && follow.current_revision && follow.current_revision !== follow.applied_revision,
  );
  return (
    <div data-testid={`rules-target-${element.id}`} className="rounded-md border p-3">
      <div className="flex flex-wrap items-start justify-between gap-2">
        <div className="min-w-0 space-y-1">
          <div className="flex flex-wrap items-center gap-2">
            <span className="text-sm font-medium">{nameOf(element)}</span>
            <span
              className={cn(
                "rounded-full border px-2 py-0.5 text-[11px]",
                element.state === "managed-ok" ? "border-success/30 bg-success/10 text-success" : "bg-muted text-muted-foreground",
              )}
            >
              {stateLabel(element)}
            </span>
            {observed.matched_template && (
              <span className="text-[11px] text-muted-foreground">与模板「{observed.matched_template.name}」一致</span>
            )}
            {!observed.matched_template && observed.matched_source_rule && (
              <span className="text-[11px] text-muted-foreground">
                与来源规则「{observed.matched_source_rule.name}」（{observed.matched_source_rule.collection}）一致
              </span>
            )}
            {follow?.update_available && (
              <span data-testid={`rules-update-available-${element.id}`} className="rounded-full border border-primary/30 bg-primary/10 px-2 py-0.5 text-[11px] text-primary">
                来源有新版本
              </span>
            )}
          </div>
          <p className="font-mono text-xs break-all text-muted-foreground">{spec.target ?? observed.target}</p>
          {observed.blocked_reason && <p className="text-xs text-warning">{observed.blocked_reason}</p>}
          {changedSinceFollow && (
            <p data-testid={`rules-changed-since-follow-${element.id}`} className="text-xs text-muted-foreground">
              这份文件在上次从来源「{follow?.rule}」写入后被改过；来源也有了新版本。要不要用新版覆盖，看过差异再决定（覆盖前会先备份现有内容）。
            </p>
          )}
          {follow && follow.current_revision === null && (
            <p className="text-xs text-muted-foreground">跟随的来源规则「{follow.rule}」已不在（来源已停用、删除或还没取回）。</p>
          )}
        </div>
        {writable && (
          <div className="flex flex-wrap gap-2">
            {followedRule && (follow?.update_available || changedSinceFollow) && (
              <button
                type="button"
                data-testid={`rules-update-from-source-${element.id}`}
                onClick={() => onApplyRule(followedRule)}
                className="rounded bg-primary px-2.5 py-1 text-xs text-primary-foreground hover:bg-primary/90"
              >
                {follow?.update_available ? "更新到新版本" : "查看新版本"}
              </button>
            )}
            {matchedRule && !follow && (
              <button
                type="button"
                data-testid={`rules-follow-${element.id}`}
                disabled={busy}
                title="文件不动，只记下它来自这条来源规则；来源出新版本时这里会提示更新"
                onClick={() => onFollow(matchedRule)}
                className="rounded border border-primary/30 px-2.5 py-1 text-xs text-primary hover:bg-primary/10 disabled:opacity-50"
              >
                跟随来源更新
              </button>
            )}
            <button
              type="button"
              aria-label={`编辑 ${nameOf(element)}`}
              onClick={onEdit}
              className="rounded border border-primary/30 px-2.5 py-1 text-xs text-primary hover:bg-primary/10"
            >
              编辑
            </button>
            {onApplyTemplate && (
              <button
                type="button"
                aria-label={`应用模板到 ${nameOf(element)}`}
                onClick={onApplyTemplate}
                className="rounded border px-2.5 py-1 text-xs hover:bg-accent"
              >
                应用模板
              </button>
            )}
            {onCopy && (
              <button
                type="button"
                aria-label={`复制 ${nameOf(element)} 到…`}
                onClick={onCopy}
                className="rounded border px-2.5 py-1 text-xs hover:bg-accent"
              >
                复制到…
              </button>
            )}
          </div>
        )}
      </div>
      {hasContent && (
        <div className="mt-2">
          <button
            type="button"
            aria-expanded={open}
            onClick={() => setOpen(!open)}
            className="inline-flex items-center gap-1 text-xs text-primary"
          >
            {open ? <ChevronDown className="h-3 w-3" aria-hidden /> : <ChevronRight className="h-3 w-3" aria-hidden />}
            {open ? "收起全文" : `查看全文（${lineCount(observed.content ?? "")} 行）`}
          </button>
          {open && (
            <pre
              data-testid={`rules-content-${element.id}`}
              className="mt-2 max-h-80 overflow-auto rounded border bg-muted/40 p-2 font-mono text-xs whitespace-pre-wrap"
            >
              {observed.content}
            </pre>
          )}
        </div>
      )}
    </div>
  );
}

function SourceRulesSection({
  rules,
  suggested,
  busy,
  onApply,
  onFetch,
  onOpenSkills,
}: {
  rules: RuleSourceRule[];
  suggested: RuleSuggestedSource[];
  busy: boolean;
  onApply?: (rule: RuleSourceRule) => void;
  onFetch: (collection: string) => void;
  onOpenSkills?: () => void;
}) {
  const [openId, setOpenId] = useState<string | null>(null);
  // 每个 git 来源一个「更新来源」：本地目录来源改了即时生效，不需要
  const gitCollections = Array.from(new Set(rules.filter((rule) => !rule.local).map((rule) => rule.collection)));
  return (
    <section className="space-y-3 rounded-lg border bg-card p-5 shadow-sm" aria-label="来源规则">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div>
          <h3 className="text-sm font-semibold">来源规则</h3>
          <p className="mt-1 text-xs text-muted-foreground">
            来自技能来源里的 rules/，只读；应用前会先展示与目标现有内容的差异。来源更新后，跟随它的规则文件会提示可更新。
          </p>
        </div>
        <div className="flex flex-wrap gap-2">
          {gitCollections.map((collection) => (
            <button
              key={collection}
              type="button"
              data-testid={`rules-fetch-source-${collection}`}
              disabled={busy}
              onClick={() => onFetch(collection)}
              className="rounded border px-2.5 py-1 text-xs hover:bg-accent disabled:opacity-50"
            >
              更新来源 {collection}
            </button>
          ))}
        </div>
      </div>
      {rules.length === 0 && (
        <div data-testid="rules-no-source" className="space-y-2">
          <p className="text-xs text-muted-foreground">还没有来源提供规则。规则内容不随程序发布，需要从技能来源取回。</p>
          {suggested.map((item) => (
            <div key={item.name} className="flex flex-wrap items-center justify-between gap-2 rounded-md border p-3">
              <div className="min-w-0">
                <p className="text-sm font-medium">{item.name}</p>
                <p className="text-xs text-muted-foreground">{item.description}</p>
              </div>
              {item.collection ? (
                <button
                  type="button"
                  data-testid={`rules-fetch-source-${item.collection}`}
                  disabled={busy}
                  onClick={() => onFetch(item.collection as string)}
                  className="rounded border border-primary/30 px-2.5 py-1 text-xs text-primary hover:bg-primary/10 disabled:opacity-50"
                >
                  取回 {item.collection}
                </button>
              ) : (
                <span className="text-xs text-muted-foreground">
                  还没添加：到
                  {onOpenSkills ? (
                    <button type="button" onClick={onOpenSkills} className="mx-1 text-primary underline-offset-2 hover:underline">
                      技能页
                    </button>
                  ) : (
                    "技能页"
                  )}
                  添加推荐来源 {item.name}，再回来取回。
                </span>
              )}
            </div>
          ))}
        </div>
      )}
      <ul className="divide-y">
        {rules.map((rule) => (
          <li key={rule.id} data-testid={`rules-source-rule-${rule.id}`} className="py-2">
            <div className="flex flex-wrap items-center justify-between gap-2">
              <div className="min-w-0">
                <span className="text-sm font-medium">{rule.name}</span>
                <span className="ml-2 rounded-full border px-2 py-0.5 text-[11px] text-muted-foreground">
                  {rule.collection} · {rule.local ? "本地目录" : "git 镜像"}
                </span>
                <span className="ml-2 text-[11px] text-muted-foreground">{lineCount(rule.body)} 行</span>
              </div>
              <div className="flex gap-2">
                <button
                  type="button"
                  aria-expanded={openId === rule.id}
                  onClick={() => setOpenId(openId === rule.id ? null : rule.id)}
                  className="rounded border px-2.5 py-1 text-xs hover:bg-accent"
                >
                  {openId === rule.id ? "收起" : "预览"}
                </button>
                {onApply && (
                  <button
                    type="button"
                    aria-label={`应用${sourceRuleLabel(rule)}到…`}
                    onClick={() => onApply(rule)}
                    className="rounded border border-primary/30 px-2.5 py-1 text-xs text-primary hover:bg-primary/10"
                  >
                    应用到…
                  </button>
                )}
              </div>
            </div>
            {openId === rule.id && (
              <pre data-testid={`rules-source-body-${rule.id}`} className="mt-2 max-h-80 overflow-auto rounded border bg-muted/40 p-2 font-mono text-xs whitespace-pre-wrap">
                {rule.body}
              </pre>
            )}
          </li>
        ))}
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
      <div className="mt-4 flex justify-end gap-2">
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
  return (
    <pre data-testid="rules-diff" className="max-h-80 overflow-auto rounded border p-2 font-mono text-xs whitespace-pre-wrap">
      {lines.map((line, index) => (
        <div
          key={index}
          data-kind={line.kind}
          className={cn(line.kind === "add" && "bg-success/10 text-success", line.kind === "del" && "bg-destructive/10 text-destructive")}
        >
          {line.kind === "add" ? "+ " : line.kind === "del" ? "- " : "  "}
          {line.text}
        </div>
      ))}
    </pre>
  );
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
        目标现有内容 → 将写入的内容（<span className="font-mono">{specOf(target).target ?? observed.target}</span>）。现有内容对不上任何已保存模板、来源规则或上次从来源写入的那一版时，会先另存为备份模板再写入。
      </p>
      <div className="mt-2">
        <DiffView current={observed.content} after={source.body} />
      </div>
      {error && (
        <p role="alert" className="mt-2 text-sm text-destructive">
          {error}
        </p>
      )}
      <div className="mt-4 flex justify-end gap-2">
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
      <div className="mt-4 flex justify-end gap-2">
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
