import { useMemo, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { Plus, Trash2 } from "lucide-react";
import { engineErrorMessage, runEngine } from "../lib/engine";
import { HOOK_EVENTS, buildHooks, flattenHooks, hookRowError, type HookRow } from "../lib/hooksDefinition";
import { engineKeys } from "../lib/queries";
import { PageError, PageLoading } from "./PageState";

interface Source {
  key: string;
  label: string;
  value: Record<string, unknown>;
  revision: string;
  format: "text" | "json";
}

/**
 * Hooks 结构化编辑：按「事件 + 匹配器 + 命令」逐行填，不用照着 JSON 结构手写。
 *
 * 保存把行回填成落盘结构，不认识的字段（timeout 等）原样带回；保存只写本地资源库，
 * 要落到助手配置得另点同步。写入用读到的版本号比对，期间被别处改过就失败，不覆盖他人改动。
 */
export function HooksEditor() {
  const queryClient = useQueryClient();
  const query = useQuery({
    queryKey: [...engineKeys.command("sources:get"), "hooks"],
    queryFn: () => runEngine<{ sources: Source[] }>("sources:get", ["--key", "hooks"]),
  });
  const [drafts, setDrafts] = useState<Record<string, HookRow[]>>({});
  const [saving, setSaving] = useState<string | null>(null);

  const sources = query.data?.ok ? query.data.data.sources : [];
  const rowsOf = useMemo(() => {
    const map: Record<string, HookRow[]> = {};
    for (const source of sources) {
      map[source.key] = drafts[source.key] ?? flattenHooks(source.value.hooks as Record<string, unknown>);
    }
    return map;
  }, [sources, drafts]);

  if (query.isPending) return <PageLoading text="正在读取 Hooks 定义…" />;
  if (query.isError) return <PageError title="读取 Hooks 定义失败" error={query.error} />;
  if (query.data && !query.data.ok) {
    return <PageError title="读取 Hooks 定义失败" error={new Error(query.data.errors[0]?.message ?? "未知错误")} />;
  }

  const update = (key: string, next: HookRow[]) => setDrafts((prev) => ({ ...prev, [key]: next }));

  const save = async (source: Source) => {
    const rows = rowsOf[source.key] ?? [];
    for (const row of rows) {
      const error = hookRowError(row);
      if (error) {
        toast.error(`保存失败:${error}`);
        return;
      }
    }
    setSaving(source.key);
    try {
      // 以读到的原值为基底：本工具不展示的字段（如 removed_signatures）原样保留
      const value = { ...source.value, hooks: buildHooks(rows) };
      const envelope = await runEngine("sources:set", [
        "--key",
        source.key,
        "--value",
        JSON.stringify({ revision: source.revision, value }),
      ]);
      if (!envelope.ok) {
        toast.error(`保存被拒绝:${envelope.errors[0]?.message ?? "未知错误"}`);
        return;
      }
      setDrafts((prev) => {
        const next = { ...prev };
        delete next[source.key];
        return next;
      });
      void queryClient.invalidateQueries({ queryKey: engineKeys.all });
      toast.success(`已保存 ${source.label}`, { description: "还没写入助手配置，回到上面点同步才生效。" });
    } catch (err) {
      toast.error(`保存失败:${engineErrorMessage(err)}`);
    } finally {
      setSaving(null);
    }
  };

  return (
    <div className="space-y-4">
      <datalist id="hook-event-options">
        {HOOK_EVENTS.map((event) => (
          <option key={event} value={event} />
        ))}
      </datalist>
      {sources.map((source) => {
        const rows = rowsOf[source.key] ?? [];
        const dirty = drafts[source.key] !== undefined;
        return (
          <section key={source.key} data-testid={`hooks-editor-${source.key}`} className="rounded-lg border bg-card p-5 shadow-sm">
            <div className="flex items-start justify-between gap-3">
              <div>
                <h3 className="text-sm font-semibold text-foreground">{source.label}</h3>
                <p className="mt-1 text-xs text-muted-foreground">
                  按事件填写要执行的命令；匹配器留空表示该事件下都执行。保存只写本地配置。
                </p>
              </div>
              <button
                type="button"
                data-testid={`hooks-add-${source.key}`}
                onClick={() =>
                  update(source.key, [
                    ...rows,
                    { id: `new-${Date.now()}-${rows.length}`, event: "", matcher: "", command: "", extra: {}, entryExtra: {} },
                  ])
                }
                className="inline-flex shrink-0 items-center gap-1 rounded border border-primary/30 bg-primary/10 px-2.5 py-1 text-xs text-primary hover:bg-primary/15"
              >
                <Plus className="h-3.5 w-3.5" aria-hidden /> 添加
              </button>
            </div>

            {rows.length === 0 ? (
              <p data-testid={`hooks-empty-${source.key}`} className="mt-3 text-sm text-muted-foreground">
                还没有配置任何命令。
              </p>
            ) : (
              <ul className="mt-3 space-y-2">
                {rows.map((row, index) => (
                  <li key={row.id} data-testid={`hooks-row-${source.key}-${index}`} className="rounded-md border px-3 py-2">
                    <div className="flex flex-wrap items-center gap-1.5">
                      <input
                        value={row.event}
                        list="hook-event-options"
                        onChange={(event) =>
                          update(
                            source.key,
                            rows.map((item, i) => (i === index ? { ...item, event: event.target.value } : item)),
                          )
                        }
                        data-testid={`hooks-event-${source.key}-${index}`}
                        placeholder="事件，如 PreToolUse"
                        className="w-44 rounded border border-input px-2 py-1.5 font-mono text-xs"
                      />
                      <input
                        value={row.matcher}
                        onChange={(event) =>
                          update(
                            source.key,
                            rows.map((item, i) => (i === index ? { ...item, matcher: event.target.value } : item)),
                          )
                        }
                        data-testid={`hooks-matcher-${source.key}-${index}`}
                        placeholder="匹配器（可留空）"
                        className="w-40 rounded border border-input px-2 py-1.5 font-mono text-xs"
                      />
                      <button
                        type="button"
                        data-testid={`hooks-remove-${source.key}-${index}`}
                        onClick={() => update(source.key, rows.filter((_, i) => i !== index))}
                        aria-label="移除这条命令"
                        className="ml-auto shrink-0 rounded border px-2 py-1.5 text-xs text-destructive hover:bg-destructive/10"
                      >
                        <Trash2 className="h-3.5 w-3.5" aria-hidden />
                      </button>
                    </div>
                    <input
                      value={row.command}
                      onChange={(event) =>
                        update(
                          source.key,
                          rows.map((item, i) => (i === index ? { ...item, command: event.target.value } : item)),
                        )
                      }
                      data-testid={`hooks-command-${source.key}-${index}`}
                      placeholder="要执行的命令"
                      className="mt-1.5 w-full rounded border border-input px-2 py-1.5 font-mono text-xs"
                    />
                    {Object.keys(row.extra).length > 0 && (
                      <p className="mt-1 text-[11px] text-muted-foreground">
                        保留原有字段：{Object.keys(row.extra).join("、")}
                      </p>
                    )}
                  </li>
                ))}
              </ul>
            )}

            <div className="mt-3 flex items-center justify-end gap-2">
              {dirty && <span className="mr-auto text-xs text-warning">有未保存的修改</span>}
              <button
                type="button"
                data-testid={`hooks-save-${source.key}`}
                disabled={!dirty || saving === source.key}
                onClick={() => void save(source)}
                className="rounded-md bg-primary px-3 py-1.5 text-xs font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
              >
                {saving === source.key ? "保存中…" : "保存"}
              </button>
            </div>
          </section>
        );
      })}
    </div>
  );
}
