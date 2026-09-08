import { useState } from "react";

export function DefinitionFormDialog({ domain, initial, existingNames, onSubmit, onClose }: {
  domain: "agents" | "tools";
  initial?: { name: string; definition: Record<string, unknown> };
  existingNames: string[];
  onSubmit: (name: string, definition: Record<string, unknown>) => void;
  onClose: () => void;
}) {
  const [name, setName] = useState(initial?.name ?? "");
  const [value, setValue] = useState(JSON.stringify(initial?.definition ?? (domain === "agents" ? {
    display_name: "YOUR_AGENT", bin: "YOUR_COMMAND", preferred_provider: "npm",
    providers: { npm: { managed: true, install: "npm install -g YOUR_PACKAGE", upgrade: "npm install -g YOUR_PACKAGE", uninstall: "npm uninstall -g YOUR_PACKAGE" } },
  } : { bin: "YOUR_COMMAND", install: "npm install -g YOUR_PACKAGE", upgrade: "npm install -g YOUR_PACKAGE" }), null, 2));
  const [error, setError] = useState<string | null>(null);
  const title = `${initial ? "编辑" : "添加"}${domain === "agents" ? "助手" : "工具"}`;
  const submit = () => {
    try {
      if (!/^[a-z0-9][a-z0-9-]*$/.test(name)) throw new Error("名称只能用小写字母、数字和连字符");
      if (!initial && existingNames.includes(name)) throw new Error("名称已存在，请编辑已有条目");
      const definition: unknown = JSON.parse(value);
      if (!definition || typeof definition !== "object" || Array.isArray(definition)) throw new Error("定义必须是 JSON 对象");
      const obj = definition as Record<string, unknown>;
      if (domain === "agents" && (typeof obj.bin !== "string" || !obj.bin.trim())) throw new Error("助手定义需要填写 bin 命令名");
      if (value.includes("YOUR_")) throw new Error("请先填写示例中 YOUR_* 对应的实际值");
      onSubmit(name, obj);
    } catch (err) { setError(err instanceof Error ? err.message : String(err)); }
  };
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-slate-900/40 p-4" onKeyDown={(event) => { if (event.key === "Escape") onClose(); }}>
      <section role="dialog" aria-modal="true" aria-label={title} className="max-h-[90vh] w-full max-w-2xl overflow-y-auto rounded-xl bg-white p-5 shadow-xl">
        <h3 className="text-base font-semibold">{title}</h3>
        <p className="mt-2 text-sm text-slate-500">配置安装来源与命令。保存后预览操作，确认后才会安装或更新。</p>
        <label className="mt-4 block text-sm">唯一名称<input autoFocus disabled={!!initial} value={name} onChange={(event) => setName(event.target.value)} className="mt-1 w-full rounded border p-2" /></label>
        <label className="mt-3 block text-sm">定义（JSON）<textarea value={value} onChange={(event) => setValue(event.target.value)} rows={14} className="mt-1 w-full rounded border p-2 font-mono text-xs" spellCheck={false} /></label>
        {error && <p role="alert" className="mt-2 text-sm text-red-600">{error}</p>}
        <div className="mt-4 flex justify-end gap-2">
          <button type="button" onClick={onClose} className="rounded border px-4 py-2">取消</button>
          <button type="button" onClick={submit} className="rounded bg-indigo-600 px-4 py-2 text-white">保存并预览同步</button>
        </div>
      </section>
    </div>
  );
}
