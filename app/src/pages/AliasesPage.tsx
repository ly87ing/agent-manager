import { useState } from "react";
import { ChevronRight, Copy, Info, Pencil, Plus, Trash2 } from "lucide-react";
import { AgentIcon } from "../components/AgentIcon";
import { COL_ACTION, COL_ICONS, COL_MIDDLE, COL_NARROW, DataTable, PILL, ROW, RowIcon, TableHeader } from "../components/DataTable";
import { AliasApplyConfirm } from "../components/AliasApplyConfirm";
import { AliasEditorDialog } from "../components/AliasEditorDialog";
import { ConfirmModal } from "../components/ConfirmModal";
import { EmptyState } from "../components/EmptyState";
import { ErrorPanel } from "../components/ErrorPanel";
import { PageError, PageHeader, PageLoading } from "../components/PageState";
import { useCommandAccounts } from "../lib/useAccountNames";
import type { AliasCurrent, AliasLegacySuggestion, AliasRecommendation } from "../lib/contract";
import { DEFAULT_LANG, DEFAULT_TIMEZONE, useAliasSource, type AliasTarget } from "../lib/useAliasSource";
import { cn } from "../lib/utils";

const PAGE_NOTE =
  "以 ~/.zshrc 与 ~/.zprofile 里由本工具管理的那几行为准：这里显示并直接编辑实际的快捷命令，保存写入 ~/.zshrc（~/.zprofile 已有这几行时一并同步，没有就不新建），新开终端窗口生效。你自己另写的 alias 只读列出、不会改动；打开后文件若被别处改过，保存会被拒绝，不会覆盖。";

function fileLabel(targets: AliasTarget[], id: string): string {
  return targets.find((target) => target.id === id)?.target ?? id;
}

function targetStateLabel(target: AliasTarget): string {
  if (target.blockedReason) return "不能读写";
  if (target.state === "managed-ok") return "正常";
  return "还没有本工具管理的命令";
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
  const { query, source, busy, setAlias, previewSetAlias, previewApply, removeAlias, addRecommended, addLegacy, discardLegacy, fetchSource } = useAliasSource();
  const [dialog, setDialog] = useState<Dialog | null>(null);
  const [showMore, setShowMore] = useState(false);
  const accountOf = useCommandAccounts(
    source?.aliases ?? {},
    (source?.targets ?? []).flatMap((target) => target.external),
  );

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
  const previewBase = JSON.stringify(source.targets.map((target) => [target.id, target.base]));
  const external = source.targets.flatMap((target) => target.external.map((line) => ({ target, line })));
  /** 有文件读写不了：终端配置文件直接摊开，不藏在「更多」里。 */
  const problem = source.targets.some((target) => target.blockedReason);
  const close = () => setDialog(null);
  const refreshCurrent = () => {
    close();
    void query.refetch();
  };
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

  /** 文件列放不下两个文件名：两个都有写「都有」，只在一个里有写那个文件名，完整路径在悬停提示里。 */
  const fileNames = (ids: string[]) =>
    ids.length > 1 && ids.length === source.targets.length
      ? "都有"
      : ids.map((id) => fileLabel(source.targets, id).replace(/^~\/\./, "")).join("、");

  return (
    <div className="space-y-3">
      <h1 className="sr-only">终端快捷命令</h1>

      <DataTable label="启动命令" testId="aliases-current">
        <TableHeader title={`启动命令 · ${source.current.length}`} middle="启动的账号" narrow="文件">
          <span className="text-muted-foreground" title={PAGE_NOTE}>
            <Info className="inline h-3.5 w-3.5 align-[-2px]" aria-hidden /> 说明
          </span>
          <button
            type="button"
            data-testid="add-alias-button"
            disabled={!source.writable || busy}
            onClick={() => setDialog({ kind: "edit" })}
            className="inline-flex items-center gap-1 text-primary hover:underline disabled:opacity-50"
          >
            <Plus className="h-3.5 w-3.5" aria-hidden /> 新增
          </button>
        </TableHeader>
        {source.current.length === 0 ? (
          <EmptyState
            compact
            title="还没有快捷命令"
            description="新增一条，或从下面的推荐里添加，之后即可在终端里用短名字带固定参数启动。"
            testId="aliases-empty"
          />
        ) : (
          <ul className="divide-y">
            {source.current.map((item) => {
              const missingIn = source.targets.filter((target) => target.base !== null && !item.targets.includes(target.id));
              const account = accountOf(item.name);
              return (
                <li key={item.name} data-testid={`alias-row-${item.name}`} className={ROW}>
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2">
                      <span className="font-mono text-sm font-medium">{item.name}</span>
                      {!item.consistent && (
                        <span data-testid={`alias-inconsistent-${item.name}`} className="text-xs text-warning">⚠ 两个文件写法不同</span>
                      )}
                      {missingIn.length > 0 && (
                        <span data-testid={`alias-missing-in-${item.name}`} className="text-xs text-warning">
                          ⚠ {missingIn.map((target) => target.target).join("、")} 里没有
                        </span>
                      )}
                    </div>
                    <div className="truncate font-mono text-xs text-muted-foreground" title={item.line}>{item.value ?? item.line}</div>
                    {item.value === null && <div className="text-xs text-muted-foreground">不是本工具的写法，只能删除，或在文件里手改</div>}
                  </div>
                  <div className={cn(COL_MIDDLE, "flex min-w-0 items-center gap-1.5 text-sm")}>
                    {account ? (
                      <>
                        <AgentIcon kind={account.kind} />
                        <span className="truncate">{account.label}</span>
                      </>
                    ) : (
                      <span className="text-muted-foreground">不是启动助手的命令</span>
                    )}
                  </div>
                  <span className={cn(COL_NARROW, "text-xs text-muted-foreground")} title={item.targets.map((id) => fileLabel(source.targets, id)).join("、")}>
                    {fileNames(item.targets)}
                  </span>
                  <div className={COL_ACTION} />
                  <div className={COL_ICONS}>
                    {item.value !== null && (
                      <>
                        <RowIcon label={`编辑 ${item.name}`} testId={`edit-alias-${item.name}`} disabled={busy} onClick={() => setDialog({ kind: "edit", name: item.name, value: item.value ?? undefined })}>
                          <Pencil className="h-3.5 w-3.5" aria-hidden />
                        </RowIcon>
                        <RowIcon label={`照着 ${item.name} 新建一条`} testId={`duplicate-alias-${item.name}`} disabled={busy} onClick={() => duplicate(item)}>
                          <Copy className="h-3.5 w-3.5" aria-hidden />
                        </RowIcon>
                      </>
                    )}
                    <RowIcon danger label={`移除 ${item.name}`} testId={`remove-alias-${item.name}`} disabled={busy} onClick={() => setDialog({ kind: "remove", name: item.name })}>
                      <Trash2 className="h-3.5 w-3.5" aria-hidden />
                    </RowIcon>
                  </div>
                </li>
              );
            })}
          </ul>
        )}
      </DataTable>

      {source.recommendations.length > 0 && (
        <DataTable label="推荐" testId="alias-recommendations">
          <TableHeader title={`推荐 · ${source.recommendations.length}`} middle="来源" narrow="">
            <span className="text-muted-foreground">逐条确认后才写入</span>
          </TableHeader>
          <ul className="divide-y">
            {source.recommendations.map((item) => (
              <li key={item.name} data-testid={`alias-recommend-${item.name}`} className={ROW}>
                <LineCell name={item.name} line={item.line} value={item.value} />
                <span className={cn(COL_MIDDLE, "truncate text-sm text-muted-foreground")}>来自 {item.source}</span>
                <span className={COL_NARROW} />
                <div className={COL_ACTION}>
                  <button
                    type="button"
                    data-testid={`add-recommended-${item.name}`}
                    disabled={!source.writable || busy}
                    onClick={() => setDialog({ kind: "recommend", item })}
                    className={PILL}
                  >
                    添加
                  </button>
                </div>
                <div className={COL_ICONS} />
              </li>
            ))}
          </ul>
        </DataTable>
      )}

      {source.recommendations.length === 0 && source.suggestedSources.length > 0 && (
        <DataTable label="推荐" testId="alias-suggested-sources">
          <TableHeader title="推荐" middle="" narrow="">
            <span className="text-muted-foreground">推荐的快捷命令不随程序发布，要从技能来源取回</span>
          </TableHeader>
          <ul className="divide-y">
            {source.suggestedSources.map((item) => (
              <li key={item.name} className={ROW}>
                <div className="min-w-0 flex-1">
                  <div className="text-sm font-medium">{item.name}</div>
                  <div className="truncate text-xs text-muted-foreground">{item.description}</div>
                </div>
                {item.collection ? (
                  <button
                    type="button"
                    data-testid={`alias-fetch-source-${item.collection}`}
                    disabled={busy}
                    onClick={() => void fetchSource(item.collection as string)}
                    className={PILL}
                  >
                    取回 {item.collection}
                  </button>
                ) : (
                  <span className="shrink-0 text-xs text-muted-foreground">
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
          </ul>
        </DataTable>
      )}

      {source.legacy.length > 0 && (
        <DataTable label="旧版存着、还没用上的" testId="alias-legacy">
          <TableHeader title={`旧版存着、还没用上的 · ${source.legacy.length}`} middle="说明" narrow="">
            <span className="text-muted-foreground">旧版本存在库里、文件里没有</span>
          </TableHeader>
          <ul className="divide-y">
            {source.legacy.map((item) => (
              <li key={item.id} data-testid={`alias-legacy-${item.name}`} className={ROW}>
                <LineCell name={item.name} line={item.line} value={item.value} />
                <span className={cn(COL_MIDDLE, "truncate text-sm", item.actual ? "text-warning" : "text-muted-foreground")} title={item.actual ?? undefined}>
                  {item.actual ? `文件里现在是另一条写法，加入会替换它` : "加入后写进两个文件"}
                </span>
                <span className={COL_NARROW} />
                <div className={COL_ACTION}>
                  <button
                    type="button"
                    data-testid={`add-legacy-${item.name}`}
                    disabled={!source.writable || busy}
                    onClick={() => setDialog({ kind: "legacy-add", item })}
                    className={PILL}
                  >
                    加入
                  </button>
                </div>
                <div className={COL_ICONS}>
                  <RowIcon danger label={`丢弃 ${item.name}`} testId={`discard-legacy-${item.name}`} disabled={busy} onClick={() => setDialog({ kind: "legacy-discard", item })}>
                    <Trash2 className="h-3.5 w-3.5" aria-hidden />
                  </RowIcon>
                </div>
              </li>
            ))}
          </ul>
        </DataTable>
      )}

      {!problem && (
        <button
          type="button"
          data-testid="aliases-more"
          aria-expanded={showMore}
          onClick={() => setShowMore((value) => !value)}
          className="inline-flex items-center gap-1 px-1 text-xs text-muted-foreground hover:text-foreground"
        >
          <ChevronRight className={cn("h-3.5 w-3.5 transition-transform", showMore && "rotate-90")} aria-hidden />
          更多：{external.length > 0 ? `你自己写的 ${external.length} 条、` : ""}终端配置文件
        </button>
      )}

      {showMore && external.length > 0 && (
        <DataTable label="你自己写的" testId="alias-external">
          <TableHeader title={`你自己写的 · ${external.length}`} middle="启动的账号" narrow="文件">
            <span className="text-muted-foreground">你自己另写的，只读</span>
          </TableHeader>
          <ul className="divide-y">
            {external.map(({ target, line }) => {
              const account = accountOf(line.name);
              return (
                <li key={`${target.id}:${line.name}`} data-testid={`alias-external-${target.id}-${line.name}`} className={ROW}>
                  <LineCell name={line.name} line={line.line} value={line.value} />
                  <div className={cn(COL_MIDDLE, "flex min-w-0 items-center gap-1.5 text-sm")}>
                    {account ? (
                      <>
                        <AgentIcon kind={account.kind} />
                        <span className="truncate">{account.label}</span>
                      </>
                    ) : (
                      <span className="text-muted-foreground">不是启动助手的命令</span>
                    )}
                  </div>
                  <span className={cn(COL_NARROW, "text-xs text-muted-foreground")}>{fileNames([target.id])}</span>
                  <div className={COL_ACTION} />
                  <div className={COL_ICONS} />
                </li>
              );
            })}
          </ul>
        </DataTable>
      )}

      {(showMore || problem) && (
        <DataTable label="终端配置文件">
          <TableHeader title={`终端配置文件 · ${source.targets.length}`} middle="状态" narrow="" />
          <ul className="divide-y">
            {source.targets.map((target) => (
              <li key={target.id} data-testid={`alias-target-${target.id}`} className={ROW}>
                <span className="min-w-0 flex-1 truncate font-mono text-sm">{target.target}</span>
                <div className={cn(COL_MIDDLE, "min-w-0 text-sm", target.blockedReason ? "text-warning" : "text-muted-foreground")}>
                  <div>{targetStateLabel(target)}</div>
                  {target.blockedReason && <div className="truncate text-xs" title={target.blockedReason}>{target.blockedReason}</div>}
                </div>
                <span className={COL_NARROW} />
                <div className={COL_ACTION} />
                <div className={COL_ICONS} />
              </li>
            ))}
          </ul>
        </DataTable>
      )}

      {dialog?.kind === "edit" && (
        <AliasEditorDialog
          initialName={dialog.name}
          initialValue={dialog.value}
          presetName={dialog.presetName}
          defaults={{ timezone: DEFAULT_TIMEZONE, lang: DEFAULT_LANG }}
          existingNames={names}
          previewBase={previewBase}
          onPreview={previewSetAlias}
          onRefresh={refreshCurrent}
          onSubmit={(name, value) => void setAlias(name, value, { renameFrom: dialog.name }).then(closeOn)}
          onClose={close}
        />
      )}
      {dialog?.kind === "remove" && (
        <AliasApplyConfirm
          title={`移除 ${dialog.name}`}
          description="确认后直接从由本工具管理的那几行中去掉这一条；你自己写的内容不动。"
          confirmLabel="确认移除"
          previewBase={previewBase}
          op={["--remove", dialog.name]}
          onPreview={previewApply}
          onRefresh={refreshCurrent}
          executing={busy}
          onConfirm={() => void removeAlias(dialog.name).then(closeOn)}
          onClose={close}
        />
      )}
      {dialog?.kind === "recommend" && (
        <AliasApplyConfirm
          title={`添加推荐的 ${dialog.item.name}`}
          description="把这一行写进 ~/.zshrc 里由本工具管理的那几行；写入前重读文件，被别处改过就不写。"
          confirmLabel="添加"
          previewBase={previewBase}
          op={["--recommended", dialog.item.name]}
          onPreview={previewApply}
          onRefresh={refreshCurrent}
          executing={busy}
          onConfirm={() => void addRecommended(dialog.item.name).then(closeOn)}
          onClose={close}
        >
          <AliasLineBox line={dialog.item.line} />
        </AliasApplyConfirm>
      )}
      {dialog?.kind === "legacy-add" && (
        <AliasApplyConfirm
          title={`加入 ${dialog.item.name}`}
          description={
            dialog.item.actual
              ? "用旧版本保存的这一行替换终端配置里现在的同名命令；写入前重读文件，被别处改过就不写。"
              : "把旧版本保存的这一行写进终端配置；写入前重读文件，被别处改过就不写。"
          }
          confirmLabel="加入终端配置"
          previewBase={previewBase}
          op={["--legacy", dialog.item.id]}
          onPreview={previewApply}
          onRefresh={refreshCurrent}
          executing={busy}
          onConfirm={() => void addLegacy(dialog.item).then(closeOn)}
          onClose={close}
        >
          <AliasLineBox line={dialog.item.line} />
        </AliasApplyConfirm>
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

/** 名字列：命令名，下面一行是它的取值（完整一行在悬停提示里）。 */
function LineCell({ name, line, value }: { name: string; line: string; value: string | null }) {
  return (
    <div className="min-w-0 flex-1">
      <div className="font-mono text-sm font-medium">{name}</div>
      <div className="truncate font-mono text-xs text-muted-foreground" title={line}>{value ?? line}</div>
    </div>
  );
}

function AliasLineBox({ line }: { line: string }) {
  return <pre className="overflow-x-auto whitespace-pre-wrap break-all rounded border bg-muted/40 p-2 font-mono text-[11px]">{line}</pre>;
}
