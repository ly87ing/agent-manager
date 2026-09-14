import { useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { runEngine } from "../lib/engine";
import { engineKeys } from "../lib/queries";
import { useExecuteApply, usePlanPreview } from "../lib/usePlanPreview";
import { PageError, PageLoading } from "./PageState";

type Source = { key: string; label: string; value: string | Record<string, unknown>; revision: string; format: "text" | "json" };

export function SourceDefinitionsPanel({ domain }: { domain: "rules" | "hooks" | "aliases" }) {
  const query = useQuery({ queryKey: [...engineKeys.command("sources:get"), domain], queryFn: () => runEngine<{ sources: Source[] }>("sources:get", ["--key", domain]) });
  const [editing, setEditing] = useState<Source | null>(null);
  const [text, setText] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const queryClient = useQueryClient();
  const preview = usePlanPreview();
  const executeApply = useExecuteApply();
  const save = async () => {
    if (!editing) return;
    setSaving(true);
    try {
      const value = editing.format === "json" ? JSON.parse(text) : text;
      const result = await runEngine<Source>("sources:set", ["--key", editing.key, "--value", JSON.stringify({ revision: editing.revision, value })]);
      if (!result.ok) throw new Error(result.errors[0]?.message ?? "保存失败");
      setEditing(null);
      void queryClient.invalidateQueries({ queryKey: engineKeys.all });
      toast.success("定义已保存");
      preview.open({ title: "同步修改后的定义", subtitle: "来源已保存到本机 SQLite；确认后将根据下方计划同步到目标。", planCommands: [`${domain}:plan`], execute: executeApply({ command: `${domain}:apply`, label: "同步定义" }) });
    } catch (err) { setError(err instanceof Error ? err.message : String(err)); }
    finally { setSaving(false); }
  };
  if (query.isPending) return <PageLoading />;
  if (query.isError) return <PageError title="读取定义失败" error={query.error} />;
  if (!query.data.ok) return <PageError title="读取定义失败" error={new Error(query.data.errors[0]?.message ?? "未知错误")} />;
  return <>
    <section className="rounded-lg border bg-white p-4">
      <h3 className="text-sm font-semibold">管理定义</h3>
      <p className="mt-1 text-xs text-slate-500">在来源中添加、修改或移除条目，保存后预览同步。{domain === "rules" && "规则按共享内容和助手专用内容分别编辑。"}</p>
      <div className="mt-3 flex flex-wrap gap-2">{query.data.data.sources.map((source) => <button type="button" key={source.key} onClick={() => { setEditing(source); setText(source.format === "json" ? JSON.stringify(source.value, null, 2) : String(source.value)); setError(null); }} className="rounded border px-3 py-2 text-sm">编辑 {source.label}</button>)}</div>
    </section>
    {editing && <div className="fixed inset-0 z-50 flex items-center justify-center bg-slate-900/40 p-4" onKeyDown={(event) => { if (event.key === "Escape" && !saving) setEditing(null); }}>
      <section role="dialog" aria-modal="true" aria-label={`编辑 ${editing.label}`} className="max-h-[90vh] w-full max-w-3xl overflow-y-auto rounded-xl bg-white p-5 shadow-xl">
        <h3 className="font-semibold">编辑 {editing.label}</h3>
        <p className="my-2 text-sm text-slate-500">{editing.format === "json" ? "添加或修改对象中的条目，删除对应键即可移除。使用 {} 清空这组定义。" : "直接添加、修改或删除规则文本；清空内容可移除这组规则。"}</p>
        <textarea aria-label="定义内容" autoFocus value={text} onChange={(event) => setText(event.target.value)} rows={16} disabled={saving} spellCheck={false} autoCorrect="off" autoCapitalize="off" className="w-full rounded border p-2 font-mono text-sm" />
        {error && <p role="alert" className="mt-2 text-sm text-red-600">{error}</p>}
        <div className="mt-4 flex justify-end gap-2"><button type="button" disabled={saving} onClick={() => setEditing(null)} className="rounded border px-4 py-2">取消</button><button type="button" disabled={saving} onClick={() => void save()} className="rounded bg-indigo-600 px-4 py-2 text-white">保存并预览同步</button></div>
      </section>
    </div>}
    {preview.modal}
  </>;
}
