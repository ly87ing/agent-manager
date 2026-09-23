import { useState, type ReactNode } from "react";
import { Copy, Plus, Terminal, Trash2 } from "lucide-react";
import { AliasEditorDialog } from "../components/AliasEditorDialog";
import { ConfirmModal } from "../components/ConfirmModal";
import { EmptyState } from "../components/EmptyState";
import { ErrorPanel } from "../components/ErrorPanel";
import { PageError, PageHeader, PageLoading } from "../components/PageState";
import type { AliasCurrent, AliasLegacySuggestion, AliasRecommendation } from "../lib/contract";
import { DEFAULT_LANG, DEFAULT_TIMEZONE, useAliasSource, type AliasTarget } from "../lib/useAliasSource";
import { cn } from "../lib/utils";

const PAGE_NOTE =
  "以 ~/.zshrc 与 ~/.zprofile 里本工具维护的那一段为准：这里显示并直接编辑实际的快捷命令，保存即写入两个文件，新开终端窗口生效。这段以外你自己写的 alias 只读列出、不会改动；打开后文件若被别处改过，保存会被拒绝，不会覆盖。";

function fileLabel(targets: AliasTarget[], id: string): string {
  return targets.find((target) => target.id === id)?.target ?? id;
}

function targetStateLabel(target: AliasTarget): string {
  if (target.blockedReason) return "不能读写";
  if (target.state === "managed-ok") return "有本工具那一段";
  return "还没有本工具那一段";
}

type Dialog =
  | { kind: "edit"; name?: string; value?: string; presetName?: string }
  | { kind: "remove"; name: string }
  | { kind: "recommend"; item: AliasRecommendation }
  | { kind: "legacy-add"; item: AliasLegacySuggestion }
  | { kind: "legacy-discard"; item: AliasLegacySuggestion };

/**
 * 终端快捷命令页：逐条显示 rc 文件受管区块里的实际命令并直接编辑；内置命令只作推荐，
 * 旧版本保存、还没写进文件的命令作为建议，可加入或丢弃。
 */
export default function AliasesPage({ onOpenSkills }: { onOpenSkills?: () => void } = {}) {
  const { query, source, busy, setAlias, removeAlias, addRecommended, addLegacy, discardLegacy, fetchSource } = useAliasSource();
  const [dialog, setDialog] = useState<Dialog | null>(null);

  if (query.isPending) return <PageLoading text="正在读取终端快捷命令…" />;
  if (query.isError) return <PageError title="读取终端快捷命令失败" error={query.error} />;
  if (!query.data.ok || !source) {
    return (
      <div className="space-y-4">
        <PageHeader title="终端快捷命令" description={PAGE_NOTE} />
        <ErrorPanel title="读取终端配置失败" errors={query.data.errors} />
      </div>
    );
  }

  const names = source.current.map((item) => item.name);
  const external = source.targets.flatMap((target) => target.external.map((line) => ({ target, line })));
  const close = () => setDialog(null);
  const closeOn = (ok: boolean) => {
    if (ok) setDialog(null);
  };

  /** 复制一条：照着现有的改一点是最常见的新建方式，不用从空白开始。 */
  const duplicate = (item: AliasCurrent) => {
    let candidate = `${item.name}-copy`;
    let n = 2;
    while (names.includes(candidate)) {
      candidate = `${item.name}-copy${n}`;
      n += 1;
    }
    setDialog({ kind: "edit", value: item.value ?? undefined, presetName: candidate });
  };

  return (
    <div className="space-y-4">
      <PageHeader title="终端快捷命令" description={PAGE_NOTE} />

      <section className="rounded-lg border bg-card p-5 shadow-sm">
        <div className="flex items-start justify-between gap-3">
          <div>
            <h3 className="flex items-center gap-2 text-sm font-semibold text-foreground">
              <Terminal className="h-4 w-4 text-primary" aria-hidden />
              快捷命令
            </h3>
            <p className="mt-1 text-xs text-muted-foreground">每条命令对应终端里的一个名字，展示的就是终端配置文件里现在的那一行。</p>
          </div>
          <button
            type="button"
            data-testid="add-alias-button"
            disabled={!source.writable || busy}
            onClick={() => setDialog({ kind: "edit" })}
            className="inline-flex shrink-0 items-center gap-1 rounded border border-primary/30 bg-primary/10 px-2.5 py-1 text-xs text-primary hover:bg-primary/15 disabled:opacity-50"
          >
            <Plus className="h-3.5 w-3.5" aria-hidden /> 新增
          </button>
        </div>

        {source.current.length === 0 ? (
          <EmptyState
            compact
            title="还没有快捷命令"
            description="新增一条，或从下面的推荐里添加，之后即可在终端里用短名字带固定参数启动。"
            testId="aliases-empty"
          />
        ) : (
          <ul className="mt-3 space-y-2">
            {source.current.map((item) => {
              const missingIn = source.targets.filter((target) => target.base !== null && !item.targets.includes(target.id));
              return (
                <li key={item.name} data-testid={`alias-row-${item.name}`} className="flex flex-wrap items-center justify-between gap-3 rounded-md border px-3 py-2">
                  <div className="min-w-0">
                    <div className="flex flex-wrap items-center gap-1.5">
                      <span className="font-mono text-xs font-semibold text-foreground">{item.name}</span>
                      {item.targets.map((id) => (
                        <span key={id} className="rounded border bg-muted px-1.5 py-0.5 text-[10px] text-muted-foreground">
                          {fileLabel(source.targets, id)}
                        </span>
                      ))}
                      {!item.consistent && (
                        <span data-testid={`alias-inconsistent-${item.name}`} className="rounded border border-warning/30 bg-warning/10 px-1.5 py-0.5 text-[10px] text-warning">
                          两个文件写法不同
                        </span>
                      )}
                      {missingIn.length > 0 && (
                        <span data-testid={`alias-missing-in-${item.name}`} className="rounded border border-warning/30 bg-warning/10 px-1.5 py-0.5 text-[10px] text-warning">
                          {missingIn.map((target) => target.target).join("、")} 里没有
                        </span>
                      )}
                    </div>
                    <p className="mt-1 break-all font-mono text-[11px] text-muted-foreground">{item.line}</p>
                    {item.value === null && (
                      <p className="mt-0.5 text-[11px] text-muted-foreground">不是本工具的写法，只能删除，或在文件里手改。</p>
                    )}
                  </div>
                  <div className="flex shrink-0 items-center gap-2">
                    {item.value !== null && (
                      <>
                        <button
                          type="button"
                          data-testid={`edit-alias-${item.name}`}
                          disabled={busy}
                          onClick={() => setDialog({ kind: "edit", name: item.name, value: item.value ?? undefined })}
                          className="rounded border px-2.5 py-1 text-xs text-muted-foreground hover:bg-accent disabled:opacity-50"
                        >
                          编辑
                        </button>
                        <button
                          type="button"
                          data-testid={`duplicate-alias-${item.name}`}
                          disabled={busy}
                          onClick={() => duplicate(item)}
                          className="inline-flex items-center gap-1 rounded border px-2.5 py-1 text-xs text-muted-foreground hover:bg-accent disabled:opacity-50"
                        >
                          <Copy className="h-3.5 w-3.5" aria-hidden /> 复制
                        </button>
                      </>
                    )}
                    <button
                      type="button"
                      data-testid={`remove-alias-${item.name}`}
                      disabled={busy}
                      onClick={() => setDialog({ kind: "remove", name: item.name })}
                      className="inline-flex items-center gap-1 rounded border border-destructive/30 px-2.5 py-1 text-xs text-destructive hover:bg-destructive/10 disabled:opacity-50"
                    >
                      <Trash2 className="h-3.5 w-3.5" aria-hidden /> 移除
                    </button>
                  </div>
                </li>
              );
            })}
          </ul>
        )}
      </section>

      {source.recommendations.length > 0 && (
        <SuggestionSection
          testId="alias-recommendations"
          title="推荐"
          note="来自技能来源（如 agent-skills 的 aliases/）的常用快捷命令，不随程序发布，不会自动写入；逐条确认后才添加。已有同名命令的不再推荐。"
        >
          {source.recommendations.map((item) => (
            <li key={item.name} data-testid={`alias-recommend-${item.name}`} className="flex flex-wrap items-start justify-between gap-2">
              <div className="min-w-0">
                <AliasLineView name={item.name} line={item.line} />
                <p className="mt-0.5 text-[11px] text-muted-foreground">来自 {item.source}</p>
              </div>
              <button
                type="button"
                data-testid={`add-recommended-${item.name}`}
                disabled={!source.writable || busy}
                onClick={() => setDialog({ kind: "recommend", item })}
                className="shrink-0 rounded border border-primary/30 px-2.5 py-1 text-xs text-primary hover:bg-primary/10 disabled:opacity-50"
              >
                添加
              </button>
            </li>
          ))}
        </SuggestionSection>
      )}

      {source.recommendations.length === 0 && source.suggestedSources.length > 0 && (
        <SuggestionSection
          testId="alias-suggested-sources"
          title="推荐"
          note="推荐的快捷命令不随程序发布，需要从技能来源取回。"
        >
          {source.suggestedSources.map((item) => (
            <li key={item.name} className="flex flex-wrap items-center justify-between gap-2">
              <div className="min-w-0">
                <p className="text-sm font-medium">{item.name}</p>
                <p className="text-xs text-muted-foreground">{item.description}</p>
              </div>
              {item.collection ? (
                <button
                  type="button"
                  data-testid={`alias-fetch-source-${item.collection}`}
                  disabled={busy}
                  onClick={() => void fetchSource(item.collection as string)}
                  className="shrink-0 rounded border border-primary/30 px-2.5 py-1 text-xs text-primary hover:bg-primary/10 disabled:opacity-50"
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
            </li>
          ))}
        </SuggestionSection>
      )}

      {source.legacy.length > 0 && (
        <SuggestionSection
          testId="alias-legacy"
          title="已保存未生效"
          note="旧版本保存在本机库里、但终端配置文件里没有的快捷命令。可以加入终端配置，或丢弃（只删库里这一条）。"
        >
          {source.legacy.map((item) => (
            <li key={item.id} data-testid={`alias-legacy-${item.name}`} className="flex flex-wrap items-start justify-between gap-2">
              <div className="min-w-0">
                <AliasLineView name={item.name} line={item.line} />
                {item.actual && (
                  <p className="mt-0.5 break-all text-[11px] text-warning">
                    终端配置里现在是：<span className="font-mono">{item.actual}</span>；加入会替换它。
                  </p>
                )}
              </div>
              <div className="flex shrink-0 gap-2">
                <button
                  type="button"
                  data-testid={`add-legacy-${item.name}`}
                  disabled={!source.writable || busy}
                  onClick={() => setDialog({ kind: "legacy-add", item })}
                  className="rounded border border-primary/30 px-2.5 py-1 text-xs text-primary hover:bg-primary/10 disabled:opacity-50"
                >
                  加入
                </button>
                <button
                  type="button"
                  data-testid={`discard-legacy-${item.name}`}
                  disabled={busy}
                  onClick={() => setDialog({ kind: "legacy-discard", item })}
                  className="rounded border border-destructive/30 px-2.5 py-1 text-xs text-destructive hover:bg-destructive/10 disabled:opacity-50"
                >
                  丢弃
                </button>
              </div>
            </li>
          ))}
        </SuggestionSection>
      )}

      {external.length > 0 && (
        <SuggestionSection
          testId="alias-external"
          title="其他来源（不由本工具写入）"
          note="写在本工具那一段以外的 alias，由你自己或其他工具维护，这里只读列出，不会改动。"
        >
          {external.map(({ target, line }) => (
            <li key={`${target.id}:${line.name}`} data-testid={`alias-external-${target.id}-${line.name}`} className="flex flex-wrap items-start justify-between gap-2">
              <AliasLineView name={line.name} line={line.line} />
              <span className="shrink-0 rounded border bg-muted px-1.5 py-0.5 text-[10px] text-muted-foreground">{target.target}</span>
            </li>
          ))}
        </SuggestionSection>
      )}

      <section className="rounded-lg border bg-card p-5 shadow-sm" aria-label="终端配置文件">
        <h3 className="text-sm font-semibold text-foreground">终端配置文件</h3>
        <ul className="mt-3 space-y-2">
          {source.targets.map((target) => (
            <li key={target.id} data-testid={`alias-target-${target.id}`} className="rounded-md border px-3 py-2">
              <div className="flex flex-wrap items-center gap-2">
                <span className="font-mono text-xs">{target.target}</span>
                <span
                  className={cn(
                    "rounded-full border px-2 py-0.5 text-[11px]",
                    target.blockedReason ? "border-warning/30 bg-warning/10 text-warning" : "bg-muted text-muted-foreground",
                  )}
                >
                  {targetStateLabel(target)}
                </span>
              </div>
              {target.blockedReason && <p className="mt-1 text-xs text-warning">{target.blockedReason}</p>}
            </li>
          ))}
        </ul>
      </section>

      {dialog?.kind === "edit" && (
        <AliasEditorDialog
          initialName={dialog.name}
          initialValue={dialog.value}
          presetName={dialog.presetName}
          defaults={{ timezone: DEFAULT_TIMEZONE, lang: DEFAULT_LANG }}
          existingNames={names}
          onSubmit={(name, value) => void setAlias(name, value, { renameFrom: dialog.name }).then(closeOn)}
          onClose={close}
        />
      )}
      {dialog?.kind === "remove" && (
        <ConfirmModal
          title={`移除 ${dialog.name}`}
          description="确认后直接从 ~/.zshrc 与 ~/.zprofile 里本工具维护的那一段中去掉这一条；这段空了会连同两行注释一起移除，段外的内容不动。"
          risk="write-local"
          confirmLabel="确认移除"
          executing={busy}
          onConfirm={() => void removeAlias(dialog.name).then(closeOn)}
          onClose={close}
        />
      )}
      {dialog?.kind === "recommend" && (
        <ConfirmModal
          title={`添加推荐的 ${dialog.item.name}`}
          description="把这一行写进 ~/.zshrc 与 ~/.zprofile 里本工具维护的那一段；写入前重读文件，被别处改过就不写。"
          risk="write-local"
          confirmLabel="添加"
          executing={busy}
          onConfirm={() => void addRecommended(dialog.item.name).then(closeOn)}
          onClose={close}
        >
          <AliasLineBox line={dialog.item.line} />
        </ConfirmModal>
      )}
      {dialog?.kind === "legacy-add" && (
        <ConfirmModal
          title={`加入 ${dialog.item.name}`}
          description={
            dialog.item.actual
              ? "用旧版本保存的这一行替换终端配置里现在的同名命令；写入前重读文件，被别处改过就不写。"
              : "把旧版本保存的这一行写进终端配置；写入前重读文件，被别处改过就不写。"
          }
          risk="write-local"
          confirmLabel="加入终端配置"
          executing={busy}
          onConfirm={() => void addLegacy(dialog.item).then(closeOn)}
          onClose={close}
        >
          <AliasLineBox line={dialog.item.line} />
        </ConfirmModal>
      )}
      {dialog?.kind === "legacy-discard" && (
        <ConfirmModal
          title={`丢弃 ${dialog.item.name}`}
          description="只删除本机库里保存的这一条，不改终端配置文件。"
          risk="write-local"
          confirmLabel="丢弃"
          executing={busy}
          onConfirm={() => void discardLegacy(dialog.item).then(closeOn)}
          onClose={close}
        >
          <AliasLineBox line={dialog.item.line} />
        </ConfirmModal>
      )}
    </div>
  );
}

function SuggestionSection({ testId, title, note, children }: { testId: string; title: string; note: string; children: ReactNode }) {
  return (
    <section data-testid={testId} className="rounded-lg border border-dashed bg-card p-5">
      <h3 className="text-sm font-semibold text-foreground">{title}</h3>
      <p className="mt-1 text-xs text-muted-foreground">{note}</p>
      <ul className="mt-3 space-y-2">{children}</ul>
    </section>
  );
}

function AliasLineView({ name, line }: { name: string; line: string }) {
  return (
    <div className="min-w-0">
      <span className="font-mono text-xs font-semibold text-foreground">{name}</span>
      <p className="break-all font-mono text-[11px] text-muted-foreground">{line}</p>
    </div>
  );
}

function AliasLineBox({ line }: { line: string }) {
  return <pre className="overflow-x-auto whitespace-pre-wrap break-all rounded border bg-muted/40 p-2 font-mono text-[11px]">{line}</pre>;
}
