import { useState } from "react";
import { PlayCircle, Plus, Terminal, Trash2 } from "lucide-react";
import { AliasEditorDialog } from "../components/AliasEditorDialog";
import { ConfirmModal } from "../components/ConfirmModal";
import { ElementTable } from "../components/ElementTable";
import { EmptyState } from "../components/EmptyState";
import { ErrorPanel } from "../components/ErrorPanel";
import { PageError, PageHeader, PageLoading } from "../components/PageState";
import { SourceDefinitionsPanel } from "../components/SourceDefinitionsPanel";
import type { StatusData } from "../lib/contract";
import { LANG_OPTIONS, listTimeZones, renderAliasPreview } from "../lib/aliasCommand";
import { ALIAS_EFFECT_HINT, ALIAS_EFFECT_LABEL, aliasEffectOf, aliasElementsOf } from "../lib/aliasEffect";
import { toastWithUndo } from "../lib/undo";
import { useAliasSource } from "../lib/useAliasSource";
import { useEngineQuery } from "../lib/queries";
import { cn } from "../lib/utils";
import { useDirectApply } from "../lib/useDirectApply";

const PAGE_NOTE =
  "给终端里的命令设定固定的启动方式：时区、语言与每次都带上的参数。保存只写本地配置，点「立即同步」才写入 ~/.zshrc 与 ~/.zprofile 中由本工具维护的那一段，这段以外的内容不动。";

/**
 * 终端快捷命令页：按项配置每条命令的启动方式（时区 / 语言 / 扩展参数），
 * 并保留原文编辑作为兜底入口。同步状态与执行沿用 aliases 域既有 status/apply。
 */
export default function AliasesPage() {
  const { query: sourceQuery, source, save } = useAliasSource();
  const statusQuery = useEngineQuery<StatusData>("aliases:status");
  const directApply = useDirectApply();
  const [editing, setEditing] = useState<{ name?: string; value?: string } | null>(null);
  const [removing, setRemoving] = useState<string | null>(null);
  const [showRaw, setShowRaw] = useState(false);

  if (sourceQuery.isPending) return <PageLoading text="正在读取终端快捷命令…" />;
  if (sourceQuery.isError) return <PageError title="读取终端快捷命令失败" error={sourceQuery.error} />;
  if (sourceQuery.data && !sourceQuery.data.ok) {
    return (
      <div className="space-y-4">
        <PageHeader title="终端快捷命令" description={PAGE_NOTE} />
        <ErrorPanel title="读取配置失败" errors={sourceQuery.data.errors} />
      </div>
    );
  }
  if (!source) return <PageError title="读取终端快捷命令失败" error={new Error("没有可用配置")} />;

  const globals = { timezone: source.timezone, lang: source.lang };
  const names = Object.keys(source.aliases).sort();
  const timeZones = listTimeZones();
  const aliasElements = aliasElementsOf(statusQuery.data);

  const saveAlias = async (name: string, value: string) => {
    const ok = await save({ aliases: { ...source.aliases, [name]: value } }, `已保存 ${name}`);
    if (ok) setEditing(null);
  };

  const removeAlias = async (name: string) => {
    const removed = source.aliases[name];
    const next = { ...source.aliases };
    delete next[name];
    const ok = await save({ aliases: next }, `已移除 ${name}`, { silent: true });
    if (!ok) return;
    setRemoving(null);
    toastWithUndo(`已移除 ${name}`, {
      description: "还没同步到终端配置；撤销会把它加回来。",
      onUndo: () => void save({ aliases: { ...next, [name]: removed } }, `已恢复 ${name}`),
    });
  };

  return (
    <div className="space-y-4">
      <PageHeader title="终端快捷命令" description={PAGE_NOTE} />

      <section className="rounded-lg border bg-card p-5 shadow-sm">
        <h3 className="text-sm font-semibold text-foreground">全局默认</h3>
        <p className="mt-1 text-xs text-muted-foreground">设为「跟随全局」的命令都用这里的值；改这里会一起改到那些命令。</p>
        <div className="mt-3 grid grid-cols-1 gap-3 sm:grid-cols-2">
          <div>
            <label className="mb-1 block text-xs font-medium text-muted-foreground" htmlFor="global-timezone">
              时区
            </label>
            <select
              id="global-timezone"
              value={source.timezone}
              data-testid="global-timezone-select"
              onChange={(event) => void save({ timezone: event.target.value }, "已保存全局时区")}
              className="w-full rounded border border-input bg-background px-2 py-1.5 text-xs"
            >
              {(timeZones.includes(source.timezone) ? timeZones : [source.timezone, ...timeZones]).map((zone) => (
                <option key={zone} value={zone}>
                  {zone}
                </option>
              ))}
            </select>
          </div>
          <div>
            <label className="mb-1 block text-xs font-medium text-muted-foreground" htmlFor="global-lang">
              语言
            </label>
            <select
              id="global-lang"
              value={source.lang}
              data-testid="global-lang-select"
              onChange={(event) => void save({ lang: event.target.value }, "已保存全局语言")}
              className="w-full rounded border border-input bg-background px-2 py-1.5 text-xs"
            >
              {(LANG_OPTIONS.includes(source.lang) ? LANG_OPTIONS : [source.lang, ...LANG_OPTIONS]).map((item) => (
                <option key={item} value={item}>
                  {item}
                </option>
              ))}
            </select>
          </div>
        </div>
      </section>

      <section className="rounded-lg border bg-card p-5 shadow-sm">
        <div className="flex items-start justify-between gap-3">
          <div>
            <h3 className="flex items-center gap-2 text-sm font-semibold text-foreground">
              <Terminal className="h-4 w-4 text-primary" aria-hidden />
              快捷命令
            </h3>
            <p className="mt-1 text-xs text-muted-foreground">每条命令对应终端里的一个名字，展示的是同步后实际生效的那一行。</p>
          </div>
          <button
            type="button"
            data-testid="add-alias-button"
            onClick={() => setEditing({})}
            className="inline-flex shrink-0 items-center gap-1 rounded border border-primary/30 bg-primary/10 px-2.5 py-1 text-xs text-primary hover:bg-primary/15"
          >
            <Plus className="h-3.5 w-3.5" aria-hidden /> 新增
          </button>
        </div>

        {names.length === 0 ? (
          <EmptyState compact title="还没有快捷命令" description="新增一条后即可在终端里用短名字带固定参数启动。" testId="aliases-empty" />
        ) : (
          <ul className="mt-3 space-y-2">
            {names.map((name) => (
              <li key={name} data-testid={`alias-row-${name}`} className="flex flex-wrap items-center justify-between gap-3 rounded-md border px-3 py-2">
                <div className="min-w-0">
                  <div className="flex flex-wrap items-center gap-1.5">
                    <span className="font-mono text-xs font-semibold text-foreground">{name}</span>
                    {(() => {
                      const effect = aliasEffectOf(aliasElements, name);
                      const tone =
                        effect === "applied"
                          ? "border-success/30 bg-success/10 text-success"
                          : effect === "unknown"
                            ? "border bg-muted text-muted-foreground"
                            : "border-warning/30 bg-warning/10 text-warning";
                      return (
                        <span
                          data-testid={`alias-effect-${name}`}
                          title={ALIAS_EFFECT_HINT[effect]}
                          className={cn("rounded border px-1.5 py-0.5 text-[10px]", tone)}
                        >
                          {ALIAS_EFFECT_LABEL[effect]}
                        </span>
                      );
                    })()}
                  </div>
                  <p className="mt-1 break-all font-mono text-[11px] text-muted-foreground">
                    {renderAliasPreview(name, source.aliases[name], globals)}
                  </p>
                </div>
                <div className="flex shrink-0 items-center gap-2">
                  <button
                    type="button"
                    data-testid={`edit-alias-${name}`}
                    onClick={() => setEditing({ name, value: source.aliases[name] })}
                    className="rounded border px-2.5 py-1 text-xs text-muted-foreground hover:bg-accent"
                  >
                    编辑
                  </button>
                  <button
                    type="button"
                    data-testid={`remove-alias-${name}`}
                    onClick={() => setRemoving(name)}
                    className="inline-flex items-center gap-1 rounded border border-destructive/30 px-2.5 py-1 text-xs text-destructive hover:bg-destructive/10"
                  >
                    <Trash2 className="h-3.5 w-3.5" aria-hidden /> 移除
                  </button>
                </div>
              </li>
            ))}
          </ul>
        )}
      </section>

      <div className="flex items-center justify-end gap-2">
        <button
          type="button"
          onClick={() => setShowRaw((prev) => !prev)}
          data-testid="toggle-alias-raw-panel"
          className="rounded border px-2.5 py-1 text-xs text-muted-foreground hover:bg-accent"
        >
          {showRaw ? "收起原文编辑" : "编辑配置原文"}
        </button>
        <button
          type="button"
          data-testid="apply-aliases-button"
          disabled={directApply.executing}
          onClick={() => void directApply.apply({ command: "aliases:apply", label: "同步终端快捷命令" })}
          className="inline-flex items-center gap-2 rounded-md bg-primary px-4 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-60"
        >
          <PlayCircle className="h-4 w-4" aria-hidden /> 立即同步
        </button>
      </div>
      <p className="text-right text-xs text-muted-foreground">同步后新开一个终端窗口即可生效。</p>

      {showRaw && <SourceDefinitionsPanel domain="aliases" />}

      {statusQuery.data?.ok && (
        <section className="rounded-lg border bg-card p-5 shadow-sm">
          <h3 className="text-sm font-semibold text-foreground">同步状态</h3>
          <div className="mt-3">
            <ElementTable elements={statusQuery.data.data.elements} />
          </div>
        </section>
      )}
      {statusQuery.data && !statusQuery.data.ok && <ErrorPanel title="同步状态读取失败" errors={statusQuery.data.errors} />}

      {editing && (
        <AliasEditorDialog
          initialName={editing.name}
          initialValue={editing.value}
          globals={globals}
          existingNames={names}
          onSubmit={(name, value) => void saveAlias(name, value)}
          onClose={() => setEditing(null)}
        />
      )}

      {removing && (
        <ConfirmModal
          title={`移除 ${removing}`}
          description="移除后这条快捷命令不再由本工具维护；下次同步会把它从终端配置里本工具维护的那一段中删掉。"
          risk="write-local"
          confirmLabel="确认移除"
          executing={false}
          onConfirm={() => void removeAlias(removing)}
          onClose={() => setRemoving(null)}
        />
      )}
    </div>
  );
}
