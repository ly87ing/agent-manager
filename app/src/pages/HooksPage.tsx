import { useState, type ReactNode } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { ConfirmModal } from "../components/ConfirmModal";
import { ErrorPanel } from "../components/ErrorPanel";
import { HooksEditor } from "../components/HooksEditor";
import { PageError, PageHeader, PageLoading } from "../components/PageState";
import type { Diagnostic, EngineElement, StatusData } from "../lib/contract";
import type { HookCleanupSuggestion, HookLegacySuggestion, HooksObserved, HooksSpec } from "../lib/observed";
import { useDialogFocus } from "../lib/dialogFocus";
import { engineErrorMessage, runEngine } from "../lib/engine";
import { flattenHooks, hookSummary, hookType } from "../lib/hooksDefinition";
import { engineKeys, useEngineQuery } from "../lib/queries";
import { newRunId } from "../lib/runId";
import { useDirectApply } from "../lib/useDirectApply";
import { cn } from "../lib/utils";

const PAGE_NOTE =
  "以各助手的实际配置为准：这里显示并直接编辑实际文件里的 hooks。Claude Code 只改 settings.json 的 hooks 键，接入、运行参数与你自己的设置原样保留；打开编辑后文件若被别处改过，保存会被拒绝，不会覆盖。";

/** 写入前被外部改过时引擎给出的固定说法。 */
const EXTERNAL_EDIT = "已在外部修改";

function specOf(element: EngineElement): HooksSpec {
  return element.spec as unknown as HooksSpec;
}

function observedOf(element: EngineElement): HooksObserved {
  return element.observed as unknown as HooksObserved;
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

/** 执行失败时给弹窗内展示的说明；外部修改额外说明页面已刷新。 */
function failureText(message: string): string {
  if (message.includes(EXTERNAL_EDIT)) {
    return `${message}。页面已刷新为文件的最新内容，请关闭后重新打开再操作。`;
  }
  return message;
}

function HookLine({ event, matcher, command, type }: { event: string; matcher: string | null; command: string; type?: string }) {
  return (
    <div className="min-w-0">
      <span className="font-mono text-xs">{event}</span>
      <span className="ml-2 text-[11px] text-muted-foreground">{matcher ? `匹配 ${matcher}` : "全部匹配"}</span>
      {type !== undefined && type !== "command" && (
        <span className="ml-2 rounded-full border px-2 py-0.5 text-[11px]">{type || "未标类型"}</span>
      )}
      <p className="font-mono text-xs break-all text-muted-foreground">{command}</p>
    </div>
  );
}

type DialogState =
  | { kind: "edit"; element: EngineElement }
  | { kind: "cleanup"; element: EngineElement; item: HookCleanupSuggestion }
  | { kind: "legacy-add"; element: EngineElement; item: HookLegacySuggestion }
  | { kind: "legacy-discard"; item: HookLegacySuggestion };

export default function HooksPage() {
  const query = useEngineQuery<StatusData>("hooks:status");
  const [dialog, setDialog] = useState<DialogState | null>(null);

  if (query.isPending) return <PageLoading />;
  if (query.isError) return <PageError title="Hooks 状态读取失败" error={query.error} />;
  if (!query.data.ok) {
    return (
      <div className="space-y-4">
        <PageHeader title="Hooks" description={PAGE_NOTE} />
        <ErrorPanel title="Hooks 状态读取失败" errors={query.data.errors} />
      </div>
    );
  }

  const elements = query.data.data.elements;

  return (
    <div className="space-y-4">
      <PageHeader title="Hooks" description={PAGE_NOTE} />
      {query.data.errors.length > 0 && <ErrorPanel title="Hooks 状态读取失败" errors={query.data.errors} />}

      <section className="space-y-3 rounded-lg border bg-card p-5 shadow-sm" aria-label="Hooks 配置">
        <h3 className="text-sm font-semibold">Hooks 配置</h3>
        {elements.length === 0 && <p className="text-xs text-muted-foreground">没有可显示的 Hooks 配置。</p>}
        {elements.map((element) => (
          <TargetCard
            key={element.id}
            element={element}
            onEdit={() => setDialog({ kind: "edit", element })}
            onCleanup={(item) => setDialog({ kind: "cleanup", element, item })}
            onAddLegacy={(item) => setDialog({ kind: "legacy-add", element, item })}
            onDiscardLegacy={(item) => setDialog({ kind: "legacy-discard", item })}
          />
        ))}
      </section>

      {dialog?.kind === "edit" && <EditDialog element={dialog.element} onClose={() => setDialog(null)} />}
      {dialog?.kind === "cleanup" && (
        <SuggestionApplyDialog
          element={dialog.element}
          item={dialog.item}
          title={`清理 ${nameOf(dialog.element)} 里的旧 hook`}
          description="只移除这一条 hook，其余 hooks 与文件里的其他内容不动。"
          confirmLabel="清理"
          args={["--cleanup", dialog.item.id]}
          onClose={() => setDialog(null)}
        />
      )}
      {dialog?.kind === "legacy-add" && (
        <SuggestionApplyDialog
          element={dialog.element}
          item={dialog.item}
          title={`加入到 ${nameOf(dialog.element)}`}
          description="把这条旧版本保存的 hook 写进实际配置；写入前重读文件，被别处改过就不写。"
          confirmLabel="加入实际配置"
          args={["--legacy", dialog.item.id]}
          onClose={() => setDialog(null)}
        />
      )}
      {dialog?.kind === "legacy-discard" && <DiscardLegacyDialog item={dialog.item} onClose={() => setDialog(null)} />}
    </div>
  );
}

function TargetCard({
  element,
  onEdit,
  onCleanup,
  onAddLegacy,
  onDiscardLegacy,
}: {
  element: EngineElement;
  onEdit: () => void;
  onCleanup: (item: HookCleanupSuggestion) => void;
  onAddLegacy: (item: HookLegacySuggestion) => void;
  onDiscardLegacy: (item: HookLegacySuggestion) => void;
}) {
  const spec = specOf(element);
  const observed = observedOf(element);
  const writable = isWritable(element);
  const rows = flattenHooks(observed.hooks);
  const cleanup = observed.cleanup ?? [];
  const legacy = observed.legacy ?? [];
  return (
    <div data-testid={`hooks-target-${element.id}`} className="rounded-md border p-3">
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
            {observed.hooks && <span className="text-[11px] text-muted-foreground">{observed.hook_count} 条命令</span>}
          </div>
          <p className="font-mono text-xs break-all text-muted-foreground">
            {spec.target ?? observed.target}
            {spec.scope === "hooks-key" && <span className="ml-1 font-sans">（只改 hooks 键）</span>}
          </p>
          {observed.blocked_reason && <p className="text-xs text-warning">{observed.blocked_reason}</p>}
        </div>
        {writable && (
          <button
            type="button"
            aria-label={`编辑 ${nameOf(element)}`}
            onClick={onEdit}
            className="rounded border border-primary/30 px-2.5 py-1 text-xs text-primary hover:bg-primary/10"
          >
            编辑
          </button>
        )}
      </div>

      {rows.length > 0 && (
        <ul data-testid={`hooks-rows-${element.id}`} className="mt-2 space-y-1.5 border-t pt-2">
          {rows.map((row) => (
            <li key={row.id}>
              <HookLine event={row.event} matcher={row.matcher || null} command={hookSummary(row)} type={hookType(row)} />
            </li>
          ))}
        </ul>
      )}

      {writable && cleanup.length > 0 && (
        <SuggestionList
          testId={`hooks-cleanup-${element.id}`}
          title="清理建议"
          note="旧版本本工具写过的 hook，现在已不再需要；逐条确认后才移除。"
        >
          {cleanup.map((item) => (
            <li key={item.id} className="flex flex-wrap items-start justify-between gap-2">
              <HookLine event={item.event} matcher={item.matcher} command={item.command} />
              <button
                type="button"
                aria-label={`清理 ${item.command}`}
                onClick={() => onCleanup(item)}
                className="shrink-0 rounded border border-warning/40 px-2.5 py-1 text-xs text-warning hover:bg-warning/10"
              >
                清理
              </button>
            </li>
          ))}
        </SuggestionList>
      )}

      {writable && legacy.length > 0 && (
        <SuggestionList
          testId={`hooks-legacy-${element.id}`}
          title="旧版已保存的 hooks（未生效）"
          note="旧版本保存在本机库里、但实际配置里没有的 hook。可以加入实际配置，或丢弃（只删库里这一条）。"
        >
          {legacy.map((item) => (
            <li key={item.id} className="flex flex-wrap items-start justify-between gap-2">
              <HookLine event={item.event} matcher={item.matcher} command={item.command} />
              <div className="flex shrink-0 gap-2">
                <button
                  type="button"
                  aria-label={`加入实际配置 ${item.command}`}
                  onClick={() => onAddLegacy(item)}
                  className="rounded border border-primary/30 px-2.5 py-1 text-xs text-primary hover:bg-primary/10"
                >
                  加入实际配置
                </button>
                <button
                  type="button"
                  aria-label={`丢弃 ${item.command}`}
                  onClick={() => onDiscardLegacy(item)}
                  className="rounded border border-destructive/30 px-2.5 py-1 text-xs text-destructive hover:bg-destructive/10"
                >
                  丢弃
                </button>
              </div>
            </li>
          ))}
        </SuggestionList>
      )}
    </div>
  );
}

function SuggestionList({ testId, title, note, children }: { testId: string; title: string; note: string; children: ReactNode }) {
  return (
    <div data-testid={testId} className="mt-3 rounded-md border border-dashed p-3">
      <h4 className="text-xs font-semibold">{title}</h4>
      <p className="mt-0.5 text-[11px] text-muted-foreground">{note}</p>
      <ul className="mt-2 space-y-2">{children}</ul>
    </div>
  );
}

function EditDialog({ element, onClose }: { element: EngineElement; onClose: () => void }) {
  const observed = observedOf(element);
  const [error, setError] = useState<string | null>(null);
  const directApply = useDirectApply();
  const name = nameOf(element);
  const { dialogRef, onDialogKeyDown } = useDialogFocus<HTMLElement>({
    onEscape: directApply.executing ? undefined : onClose,
  });

  const save = async (hooks: Record<string, unknown>) => {
    if (typeof observed.base !== "string") return;
    setError(null);
    const envelope = await directApply.apply({
      command: "hooks:apply",
      label: `保存 ${name}`,
      args: ["--only", element.id, "--base", observed.base, "--value", JSON.stringify(hooks)],
    });
    if (envelope?.ok) {
      onClose();
      return;
    }
    setError(failureText(envelope?.errors[0]?.message ?? "保存失败，详见任务记录"));
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-slate-900/40 p-4">
      <section
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-label={`编辑 ${name}`}
        tabIndex={-1}
        onKeyDown={onDialogKeyDown}
        className="max-h-[90vh] w-full max-w-3xl overflow-y-auto rounded-xl bg-card p-5 shadow-xl"
      >
        <h3 className="font-semibold">编辑 {name}</h3>
        <p className="my-2 text-xs text-muted-foreground">
          保存即写入 <span className="font-mono">{observed.target}</span>
          {specOf(element).scope === "hooks-key" ? "（只改 hooks 键）" : ""}；打开编辑后若文件被别处改过，保存会被拒绝，不会覆盖。
        </p>
        <HooksEditor initialHooks={observed.hooks ?? {}} busy={directApply.executing} onSave={(hooks) => void save(hooks)} onCancel={onClose} />
        {error && (
          <p role="alert" data-testid="hooks-save-error" className="mt-2 text-sm text-destructive">
            {error}
          </p>
        )}
      </section>
    </div>
  );
}

function SuggestionApplyDialog({
  element,
  item,
  title,
  description,
  confirmLabel,
  args,
  onClose,
}: {
  element: EngineElement;
  item: HookCleanupSuggestion;
  title: string;
  description: string;
  confirmLabel: string;
  args: string[];
  onClose: () => void;
}) {
  const observed = observedOf(element);
  const directApply = useDirectApply();
  const [errors, setErrors] = useState<Diagnostic[] | null>(null);

  const confirm = async () => {
    if (typeof observed.base !== "string") return;
    const envelope = await directApply.apply({
      command: "hooks:apply",
      label: title,
      args: ["--only", element.id, "--base", observed.base, ...args],
    });
    if (envelope?.ok) {
      onClose();
      return;
    }
    const message = failureText(envelope?.errors[0]?.message ?? "写入失败，详见任务记录");
    setErrors([{ code: envelope?.errors[0]?.code ?? "HOOK_ERROR", element: null, message }]);
  };

  return (
    <ConfirmModal
      title={title}
      description={description}
      risk="write-local"
      confirmLabel={confirmLabel}
      executing={directApply.executing}
      resultErrors={errors}
      onConfirm={() => void confirm()}
      onClose={onClose}
    >
      <div className="rounded border bg-muted/40 p-2">
        <HookLine event={item.event} matcher={item.matcher} command={item.command} />
      </div>
    </ConfirmModal>
  );
}

function DiscardLegacyDialog({ item, onClose }: { item: HookLegacySuggestion; onClose: () => void }) {
  const queryClient = useQueryClient();
  const [executing, setExecuting] = useState(false);
  const [errors, setErrors] = useState<Diagnostic[] | null>(null);

  const confirm = async () => {
    setExecuting(true);
    try {
      const envelope = await runEngine("hooks:discard-legacy", ["--legacy", item.id], { runId: newRunId() });
      if (!envelope.ok) {
        setErrors(envelope.errors.map((error) => ({ code: error.code, element: null, message: error.message })));
        return;
      }
      void queryClient.invalidateQueries({ queryKey: engineKeys.all });
      toast.success("已丢弃这条旧版 hook");
      onClose();
    } catch (err) {
      setErrors([{ code: "ERROR", element: null, message: engineErrorMessage(err) }]);
    } finally {
      setExecuting(false);
    }
  };

  return (
    <ConfirmModal
      title="丢弃旧版 hook"
      description="只删除本机库里保存的这一条，不改任何助手文件。"
      risk="write-local"
      confirmLabel="丢弃"
      executing={executing}
      resultErrors={errors}
      onConfirm={() => void confirm()}
      onClose={onClose}
    >
      <div className="rounded border bg-muted/40 p-2">
        <HookLine event={item.event} matcher={item.matcher} command={item.command} />
      </div>
    </ConfirmModal>
  );
}
