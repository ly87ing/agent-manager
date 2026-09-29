import { X } from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";
import { useQueryClient } from "@tanstack/react-query";
import type { PolicyData } from "../lib/contract";
import { useDialogFocus } from "../lib/dialogFocus";
import { runEngine } from "../lib/engine";
import { engineKeys, useEngineQuery } from "../lib/queries";
import { ConfirmModal } from "./ConfirmModal";

/**
 * 用户自己指定的规则来源（`rules.sources.<名>.path`）：一个 .md 文件或一个目录（目录里的每个 .md
 * 是一条规则）。原地读取、改了即时生效；这里只登记路径，不复制内容，也不改任何规则文件。
 */
export interface UserSource {
  name: string;
  path: string;
  /** false = 用户停用了：它的规则不出现，跟随记录保留。 */
  enabled: boolean;
}

const NAME_RE = /^[a-z0-9][a-z0-9-]*$/;

function sourcesOf(policy: PolicyData | undefined): { sources: UserSource[]; collections: string[] } {
  const effective = (policy?.effective ?? {}) as {
    rules?: { sources?: Record<string, { path?: unknown; enabled?: boolean; _delete?: boolean }> };
    skills?: { repositories?: Record<string, { _delete?: boolean }> };
  };
  const sources = Object.entries(effective.rules?.sources ?? {})
    .filter(([, def]) => def && def._delete !== true && typeof def.path === "string")
    .map(([name, def]) => ({ name, path: def.path as string, enabled: def.enabled !== false }));
  const collections = Object.entries(effective.skills?.repositories ?? {})
    .filter(([, def]) => def && def._delete !== true)
    .map(([name]) => name);
  return { sources, collections };
}

/** 「我的规则来源」：读取登记、添加与移除（policy:set rules.sources.<名>）。 */
export function useRuleSources() {
  const policy = useEngineQuery<PolicyData>("policy:get");
  const queryClient = useQueryClient();
  const [busy, setBusy] = useState(false);
  const { sources, collections } = sourcesOf(policy.data?.ok ? policy.data.data : undefined);

  const write = async (key: string, value: unknown, done: string): Promise<boolean> => {
    setBusy(true);
    try {
      const envelope = await runEngine("policy:set", ["--layer", "local", "--key", key, "--value", JSON.stringify(value)]);
      if (!envelope.ok) {
        toast.error(`没有保存：${envelope.errors[0]?.message ?? "未知错误"}`);
        return false;
      }
      toast.success(done);
      await queryClient.invalidateQueries({ queryKey: engineKeys.all });
      return true;
    } finally {
      setBusy(false);
    }
  };

  return {
    sources,
    /** 名字不能与已有来源或技能集合重名。 */
    taken: [...sources.map((source) => source.name), ...collections],
    busy,
    add: (name: string, path: string) => write(`rules.sources.${name}`, { path, _delete: false, _replace: true }, `已添加规则来源 ${name}`),
    remove: (source: UserSource) => write(`rules.sources.${source.name}`, { _delete: true }, `已移除规则来源 ${source.name}`),
    /** 停用 / 启用：只改开关，路径与跟随记录都不动。 */
    setEnabled: (source: UserSource, enabled: boolean) =>
      write(`rules.sources.${source.name}.enabled`, enabled, `已${enabled ? "启用" : "停用"}规则来源 ${source.name}`),
  };
}

/** 移除一条登记：只删登记，目录里的文件和已写进各账号的规则文件都不动。 */
export function RemoveSourceDialog({
  source,
  busy,
  onConfirm,
  onClose,
}: {
  source: UserSource;
  busy: boolean;
  onConfirm: () => void;
  onClose: () => void;
}) {
  return (
    <ConfirmModal
      title={`移除规则来源 ${source.name}`}
      description={`只移除这条登记，${source.path} 里的文件原样保留；已经写进各账号的规则文件也不动（不再提示来源更新）。`}
      risk="write-local"
      confirmLabel="移除"
      executing={busy}
      onConfirm={onConfirm}
      onClose={onClose}
    />
  );
}

export function AddSourceDialog({
  taken,
  busy,
  onClose,
  onSubmit,
}: {
  taken: string[];
  busy: boolean;
  onClose: () => void;
  onSubmit: (name: string, path: string) => void;
}) {
  const [name, setName] = useState("");
  const [path, setPath] = useState("");
  const { dialogRef, onDialogKeyDown } = useDialogFocus<HTMLDivElement>({ onEscape: onClose });
  const trimmed = path.trim();
  const error = !NAME_RE.test(name)
    ? "名称只能用小写字母、数字和连字符"
    : taken.includes(name)
      ? `${name} 已被占用（与已有来源或技能集合同名）`
      : !/^(~\/|\/)/.test(trimmed)
        ? "路径填绝对路径或以 ~/ 开头"
        : null;
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-slate-900/40 p-4" onMouseDown={(event) => event.target === event.currentTarget && onClose()}>
      <div ref={dialogRef} role="dialog" aria-modal="true" aria-label="添加规则来源" onKeyDown={onDialogKeyDown} className="w-full max-w-md rounded-xl border bg-card p-5 shadow-xl">
        <div className="flex items-center justify-between">
          <h3 className="text-base font-semibold">添加规则来源</h3>
          <button type="button" aria-label="关闭" onClick={onClose} className="rounded p-1 text-muted-foreground hover:bg-accent">
            <X className="h-4 w-4" aria-hidden />
          </button>
        </div>
        <div className="mt-4 space-y-3 text-sm">
          <label className="block">
            <span className="text-xs font-medium text-muted-foreground">名称</span>
            <input data-testid="rule-source-name" value={name} onChange={(event) => setName(event.target.value)} placeholder="my-rules" className="mt-1 w-full rounded border border-input bg-background px-2 py-1.5 font-mono text-xs" />
          </label>
          <label className="block">
            <span className="text-xs font-medium text-muted-foreground">位置：一个 .md 文件，或一个目录（目录里的每个 .md 是一条规则）</span>
            <input data-testid="rule-source-path" value={path} onChange={(event) => setPath(event.target.value)} placeholder="~/notes/CLAUDE.md 或 ~/notes/rules" className="mt-1 w-full rounded border border-input bg-background px-2 py-1.5 font-mono text-xs" />
          </label>
          <p className="text-xs text-muted-foreground">只登记位置、原地读取，文件改了这里即时看到；不会复制内容，也不会改任何账号的规则文件。</p>
        </div>
        <div className="mt-4 flex items-center justify-end gap-2">
          {error && (name || path) && <span className="mr-auto text-xs text-destructive">{error}</span>}
          <button type="button" onClick={onClose} className="rounded border px-3 py-1.5 text-sm hover:bg-accent">取消</button>
          <button
            type="button"
            data-testid="rule-source-save"
            disabled={Boolean(error) || busy}
            onClick={() => onSubmit(name, trimmed)}
            className="rounded bg-primary px-3 py-1.5 text-sm text-primary-foreground hover:bg-primary/90 disabled:opacity-40"
          >
            添加
          </button>
        </div>
      </div>
    </div>
  );
}
