import { useState } from "react";
import { X } from "lucide-react";

/**
 * 技能集合定义表单。未编辑的目标授权保持原配置语义。
 */
export function SkillCollectionFormDialog({
  agentIds,
  initial,
  onSubmit,
  onClose,
}: {
  agentIds: string[];
  initial?: { name: string; definition: Record<string, unknown> };
  onSubmit: (name: string, definition: Record<string, unknown>) => void;
  onClose: () => void;
}) {
  const [name, setName] = useState(initial?.name ?? "");
  const [url, setUrl] = useState(String(initial?.definition.url ?? ""));
  const [pathsText, setPathsText] = useState((initial?.definition.skill_paths as string[] | undefined)?.join("\n") ?? "");
  const [selectedAgents, setSelectedAgents] = useState<string[]>((initial?.definition.agents as string[] | undefined) ?? []);
  const [error, setError] = useState<string | null>(null);

  const submit = () => {
    if (!/^[a-z0-9][a-z0-9-]*$/.test(name)) {
      setError("名称只能用小写字母/数字/连字符（如 my-skills）");
      return;
    }
    if (!/^https?:\/\//.test(url.trim()) && !url.trim().startsWith("git@")) {
      setError("仓库地址要以 https:// 或 git@ 开头");
      return;
    }
    const skillPaths = pathsText.split("\n").map((line) => line.trim()).filter(Boolean);
    const definition: Record<string, unknown> = { url: url.trim(), skill_paths: skillPaths.length ? skillPaths : null, agents: selectedAgents.length ? selectedAgents : null };
    if (initial && JSON.stringify(selectedAgents) === JSON.stringify(initial.definition.agents ?? [])) {
      delete definition.agents;
    }
    onSubmit(name, definition);
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-slate-900/40 p-4" onClick={(e) => e.target === e.currentTarget && onClose()}>
      <div role="dialog" aria-modal="true" aria-label={initial ? "编辑集合" : "新增集合"} className="max-h-[90vh] w-full max-w-lg overflow-y-auto rounded-xl border border-slate-200 bg-white p-5 shadow-xl">
        <div className="flex items-start justify-between">
          <h3 className="text-base font-semibold text-slate-800">{initial ? "编辑技能集合" : "新增技能集合"}</h3>
          <button type="button" onClick={onClose} aria-label="关闭" className="rounded p-1 text-slate-400 hover:bg-slate-100">
            <X className="h-4 w-4" aria-hidden />
          </button>
        </div>
        <div className="mt-4 space-y-3 text-sm">
          <div>
            <label className="mb-1 block text-xs font-medium text-slate-500">集合名称（唯一，小写字母/数字/连字符）</label>
            <input value={name} disabled={!!initial} onChange={(e) => setName(e.target.value)} data-testid="collection-name-input" aria-label="集合名称" className="w-full rounded border border-slate-300 px-2 py-1.5 font-mono text-xs" placeholder="my-skills" />
          </div>
          <div>
            <label className="mb-1 block text-xs font-medium text-slate-500">仓库地址（git）</label>
            <input value={url} onChange={(e) => setUrl(e.target.value)} data-testid="collection-url-input" className="w-full rounded border border-slate-300 px-2 py-1.5 font-mono text-xs" placeholder="https://github.com/example/skills.git" />
          </div>
          <div>
            <label className="mb-1 block text-xs font-medium text-slate-500">发现范围（每行一个目录名；留空 = 整个仓库）</label>
            <textarea value={pathsText} onChange={(e) => setPathsText(e.target.value)} rows={3} data-testid="collection-paths-input" className="w-full rounded border border-slate-300 px-2 py-1.5 font-mono text-xs" placeholder={"safe-merge-review\nreader-facing-writing"} />
          </div>
          <div>
            <span className="mb-1 block text-xs font-medium text-slate-500">分发给哪些助手（都不勾 = 全部）</span>
            <div className="flex flex-wrap gap-3">
              {agentIds.map((agent) => (
                <label key={agent} className="flex items-center gap-1.5 text-xs text-slate-700">
                  <input
                    type="checkbox"
                    checked={selectedAgents.includes(agent)}
                    onChange={(e) => setSelectedAgents((prev) => (e.target.checked ? [...prev, agent] : prev.filter((item) => item !== agent)))}
                    data-testid={`collection-agent-${agent}`}
                  />
                  <span className="font-mono">{agent}</span>
                </label>
              ))}
            </div>
          </div>
          {error && <p className="text-xs text-red-600">{error}</p>}
        </div>
        <div className="mt-5 flex justify-end gap-2">
          <button type="button" onClick={onClose} className="rounded-md border border-slate-200 px-4 py-2 text-sm text-slate-600 hover:bg-slate-50">
            取消
          </button>
          <button type="button" onClick={submit} data-testid="collection-form-submit" className="rounded-md bg-indigo-600 px-4 py-2 text-sm font-medium text-white hover:bg-indigo-700">
            保存并预览同步
          </button>
        </div>
      </div>
    </div>
  );
}
