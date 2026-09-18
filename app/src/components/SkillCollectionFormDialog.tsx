import { useState } from "react";
import { X } from "lucide-react";

/**
 * 技能集合定义表单。保存只把定义写入本地资源库，不自动安装或同步。
 * 编辑时若有完整原定义（fullDefinition，来自 policy:get 的 sqlite 有效定义），
 * 以它为基底只覆盖表单负责字段，整条替换保存，表单未展示的字段（如 branch）不丢；
 * 拿不到完整原定义（无 DB 旧存储）时只提交表单负责字段，由保存方迁移后取完整定义合并再整条替换。
 */
export function SkillCollectionFormDialog({
  agentIds,
  initial,
  fullDefinition,
  onSubmit,
  onClose,
}: {
  agentIds: string[];
  initial?: { name: string; definition: Record<string, unknown> };
  /** 完整原定义（整条替换基底）；新增或未传入时为 null/缺省。 */
  fullDefinition?: Record<string, unknown> | null;
  /** complete=false 表示 definition 只是表单负责字段，需由保存方迁移后取完整定义合并再整条替换。 */
  onSubmit: (name: string, definition: Record<string, unknown>, complete: boolean) => void;
  onClose: () => void;
}) {
  const [name, setName] = useState(initial?.name ?? "");
  /** 来源二选一：git 仓库（下载到本地后按版本分发）或本地目录（直接链到该目录，内容随其更新）。 */
  const [sourceKind, setSourceKind] = useState<"git" | "local">(initial?.definition.source_path ? "local" : "git");
  const [url, setUrl] = useState(String(initial?.definition.url ?? ""));
  const [sourcePath, setSourcePath] = useState(String(initial?.definition.source_path ?? ""));
  const [pathsText, setPathsText] = useState((initial?.definition.skill_paths as string[] | undefined)?.join("\n") ?? "");
  const [selectedAgents, setSelectedAgents] = useState<string[]>((initial?.definition.agents as string[] | undefined) ?? []);
  const [error, setError] = useState<string | null>(null);

  const submit = () => {
    if (!/^[a-z0-9][a-z0-9-]*$/.test(name)) {
      setError("名称只能用小写字母/数字/连字符（如 my-skills）");
      return;
    }
    if (sourceKind === "git" && !/^https?:\/\//.test(url.trim()) && !url.trim().startsWith("git@")) {
      setError("仓库地址要以 https:// 或 git@ 开头");
      return;
    }
    if (sourceKind === "local" && !/^(~\/|\/)/.test(sourcePath.trim())) {
      setError("本地目录要填绝对路径或以 ~/ 开头");
      return;
    }
    const skillPaths = pathsText.split("\n").map((line) => line.trim()).filter(Boolean);
    // 表单负责字段（可选字段显式给出，null 表示清空）；编辑且有完整原定义时以其为基底，
    // 未展示字段原样保留；否则只提交这些字段做片段合并。
    // 两种来源互斥：保存哪一种，另一种就显式清空，避免遗留字段让引擎拒绝。
    const owned: Record<string, unknown> = {
      url: sourceKind === "git" ? url.trim() : null,
      source_path: sourceKind === "local" ? sourcePath.trim() : null,
      skill_paths: skillPaths.length ? skillPaths : null,
      agents: selectedAgents.length ? selectedAgents : null,
    };
    const complete = !initial || fullDefinition != null;
    const definition = initial && fullDefinition ? { ...fullDefinition, ...owned } : owned;
    onSubmit(name, definition, complete);
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-slate-900/40 p-4 animate-fade-in" onClick={(e) => e.target === e.currentTarget && onClose()}>
      <div role="dialog" aria-modal="true" aria-label={initial ? "编辑集合" : "新增集合"} className="max-h-[90vh] w-full max-w-lg overflow-y-auto rounded-xl border bg-card p-5 shadow-xl animate-zoom-in-95">
        <div className="flex items-start justify-between">
          <h3 className="text-base font-semibold text-foreground">{initial ? "编辑技能集合" : "新增技能集合"}</h3>
          <button type="button" onClick={onClose} aria-label="关闭" className="rounded p-1 text-muted-foreground hover:bg-accent">
            <X className="h-4 w-4" aria-hidden />
          </button>
        </div>
        <div className="mt-4 space-y-3 text-sm">
          <div>
            <label className="mb-1 block text-xs font-medium text-muted-foreground">集合名称（唯一，小写字母/数字/连字符）</label>
            <input value={name} disabled={!!initial} onChange={(e) => setName(e.target.value)} data-testid="collection-name-input" aria-label="集合名称" className="w-full rounded border border-input px-2 py-1.5 font-mono text-xs" placeholder="my-skills" />
          </div>
          <div>
            <span className="mb-1 block text-xs font-medium text-muted-foreground">技能来自哪里</span>
            <div className="flex gap-3 text-xs">
              {([
                { key: "git", label: "git 仓库" },
                { key: "local", label: "本机目录" },
              ] as const).map((option) => (
                <label key={option.key} className="flex items-center gap-1.5 text-foreground">
                  <input
                    type="radio"
                    name="collection-source-kind"
                    checked={sourceKind === option.key}
                    onChange={() => setSourceKind(option.key)}
                    data-testid={`collection-source-${option.key}`}
                  />
                  {option.label}
                </label>
              ))}
            </div>
          </div>
          {sourceKind === "git" ? (
            <div>
              <label className="mb-1 block text-xs font-medium text-muted-foreground">仓库地址（git）</label>
              <input value={url} onChange={(e) => setUrl(e.target.value)} data-testid="collection-url-input" className="w-full rounded border border-input px-2 py-1.5 font-mono text-xs" placeholder="https://github.com/example/skills.git" />
            </div>
          ) : (
            <div>
              <label className="mb-1 block text-xs font-medium text-muted-foreground">本机目录（助手目录里的链接直接指向它，内容仍由原处维护）</label>
              <input value={sourcePath} onChange={(e) => setSourcePath(e.target.value)} data-testid="collection-source-path-input" className="w-full rounded border border-input px-2 py-1.5 font-mono text-xs" placeholder="~/.some-tool/skills" />
            </div>
          )}
          <div>
            <label className="mb-1 block text-xs font-medium text-muted-foreground">
              {sourceKind === "git" ? "发现范围（每行一个目录名；留空 = 整个仓库）" : "发现范围（每行一个技能目录名；留空 = 整个目录）"}
            </label>
            <textarea value={pathsText} onChange={(e) => setPathsText(e.target.value)} rows={3} data-testid="collection-paths-input" className="w-full rounded border border-input px-2 py-1.5 font-mono text-xs" placeholder={"safe-merge-review\nreader-facing-writing"} />
          </div>
          <div>
            <span className="mb-1 block text-xs font-medium text-muted-foreground">允许分配的助手（不选则不限）</span>
            <p className="mb-1.5 text-xs text-muted-foreground">这里只保存允许范围，保存后再为各助手分别添加。</p>
            <div className="flex flex-wrap gap-3">
              {agentIds.map((agent) => (
                <label key={agent} className="flex items-center gap-1.5 text-xs text-foreground">
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
          {error && <p className="text-xs text-destructive">{error}</p>}
        </div>
        <div className="mt-5 flex items-center justify-end gap-2">
          <span className="mr-auto text-xs text-muted-foreground">只保存到本地资源库，不会自动安装或同步</span>
          <button type="button" onClick={onClose} className="rounded-md border px-4 py-2 text-sm text-muted-foreground hover:bg-accent">
            取消
          </button>
          <button type="button" onClick={submit} data-testid="collection-form-submit" className="rounded-md bg-primary px-4 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90">
            保存到资源库
          </button>
        </div>
      </div>
    </div>
  );
}
