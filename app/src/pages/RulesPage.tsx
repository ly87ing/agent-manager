import { useState, type ReactNode } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { ChevronDown, ChevronRight, Plus } from "lucide-react";
import { ConfirmModal } from "../components/ConfirmModal";
import { ErrorPanel } from "../components/ErrorPanel";
import { PageError, PageHeader, PageLoading } from "../components/PageState";
import type { EngineElement, RuleRecommended, RuleTemplate, StatusData } from "../lib/contract";
import type { RulesObserved, RulesSpec } from "../lib/observed";
import { useDialogFocus } from "../lib/dialogFocus";
import { engineErrorMessage, runEngine } from "../lib/engine";
import { engineKeys, useEngineQuery } from "../lib/queries";
import { newRunId } from "../lib/runId";
import { diffLines, hasChanges } from "../lib/textDiff";
import { useDirectApply } from "../lib/useDirectApply";
import { cn } from "../lib/utils";

const PAGE_NOTE =
  "以各助手的规则文件为准：这里显示并直接编辑实际文件。模板只保存在本机库里；应用模板、内置推荐或从另一处复制之前，目标现有内容若对不上任何已保存模板，会先另存为一份备份模板。";

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

/** 一种「写入内容」的来源：已保存模板、内置推荐或另一个目标的实际内容。 */
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

export default function RulesPage() {
  const query = useEngineQuery<StatusData>("rules:status");
  const [dialog, setDialog] = useState<DialogState | null>(null);

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
  const recommended = query.data.data.recommended ?? null;
  const writable = elements.filter(isWritable);

  const templateSource = (template: RuleTemplate): ContentSource => ({
    key: `template:${template.id}`,
    label: `模板「${template.name}」`,
    body: template.body,
    args: ["--template", template.id],
  });
  const recommendedSource = (rec: RuleRecommended): ContentSource => ({
    key: "recommended",
    label: rec.name,
    body: rec.body,
    args: ["--recommended"],
  });

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
              onEdit={() => setDialog({ kind: "edit", element })}
              onApplyTemplate={
                templates.length > 0 || recommended
                  ? () =>
                      setDialog({
                        kind: "apply",
                        title: `应用模板到 ${nameOf(element)}`,
                        targets: [element],
                        sources: [...templates.map(templateSource), ...(recommended ? [recommendedSource(recommended)] : [])],
                      })
                  : undefined
              }
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

      {recommended && (
        <RecommendedSection
          recommended={recommended}
          onApply={
            writable.length > 0
              ? () => setDialog({ kind: "apply", title: `应用${recommended.name}`, targets: writable, sources: [recommendedSource(recommended)] })
              : undefined
          }
        />
      )}

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
  onEdit,
  onApplyTemplate,
  onCopy,
}: {
  element: EngineElement;
  onEdit: () => void;
  onApplyTemplate?: () => void;
  onCopy?: () => void;
}) {
  const [open, setOpen] = useState(false);
  const spec = specOf(element);
  const observed = observedOf(element);
  const writable = isWritable(element);
  const hasContent = typeof observed.content === "string";
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
            {!observed.matched_template && observed.matches_recommended && (
              <span className="text-[11px] text-muted-foreground">与内置推荐一致</span>
            )}
          </div>
          <p className="font-mono text-xs break-all text-muted-foreground">{spec.target ?? observed.target}</p>
          {observed.blocked_reason && <p className="text-xs text-warning">{observed.blocked_reason}</p>}
        </div>
        {writable && (
          <div className="flex flex-wrap gap-2">
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

function RecommendedSection({ recommended, onApply }: { recommended: RuleRecommended; onApply?: () => void }) {
  const [open, setOpen] = useState(false);
  return (
    <section className="space-y-2 rounded-lg border bg-card p-5 shadow-sm" aria-label="内置推荐">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div>
          <h3 className="text-sm font-semibold">内置推荐</h3>
          <p className="mt-1 text-xs text-muted-foreground">
            {recommended.name}（{lineCount(recommended.body)} 行）随程序提供，只读；应用前会先展示与目标现有内容的差异。
          </p>
        </div>
        {onApply && (
          <button
            type="button"
            aria-label={`应用${recommended.name}到…`}
            onClick={onApply}
            className="rounded border border-primary/30 px-2.5 py-1 text-xs text-primary hover:bg-primary/10"
          >
            应用到…
          </button>
        )}
      </div>
      <button type="button" aria-expanded={open} onClick={() => setOpen(!open)} className="inline-flex items-center gap-1 text-xs text-primary">
        {open ? <ChevronDown className="h-3 w-3" aria-hidden /> : <ChevronRight className="h-3 w-3" aria-hidden />}
        {open ? "收起" : "预览正文"}
      </button>
      {open && (
        <pre data-testid="rules-recommended-body" className="max-h-80 overflow-auto rounded border bg-muted/40 p-2 font-mono text-xs whitespace-pre-wrap">
          {recommended.body}
        </pre>
      )}
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
        目标现有内容 → 将写入的内容（<span className="font-mono">{specOf(target).target ?? observed.target}</span>）。现有内容对不上任何已保存模板、也不同于内置推荐时，会先另存为备份模板再写入。
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
