import { useEffect, useState } from "react";
import { X } from "lucide-react";
import { runEngine } from "../lib/engine";
import { useDialogFocus } from "../lib/dialogFocus";
import { shortRepository } from "./SkillOtherSources";
import { cn } from "../lib/utils";

interface FileChange {
  status: "modified" | "only-local" | "only-source" | string;
  file: string;
}

interface VersionRefLite {
  name: string;
  label: string;
  commit: string;
}

/** skills:adopt-preview 的结果。 */
export interface AdoptPreview {
  name: string;
  /** dedupe：已被管理，只清理 Codex 共用目录里的副本；adopt：纳入。 */
  kind: "dedupe" | "adopt";
  locations: { agent: string | null; shared: string | null; path: string; origin: string }[];
  removes: string[];
  source: { repository: string | null; path: string | null; version: string | null; installer: string | null } | null;
  collection?: { name: string | null; exists: boolean; url?: string; path?: string; linked_as?: string };
  digest?: string;
  source_check?:
    | {
        same: boolean;
        path: string;
        adopt_at: VersionRefLite;
        latest: VersionRefLite;
        ref: string | null;
        locations: { same: boolean; files: FileChange[] }[];
        accounts: string[];
        digest: string;
        error?: undefined;
      }
    | { error: string };
  local?: { dest: string; dest_exists: boolean; collection: string; digest: string; accounts: string[]; from: string | null; variants: number };
}

const FILE_STATUS: Record<string, string> = {
  modified: "改过",
  "only-local": "只在本地",
  "only-source": "只在来源",
};

/**
 * 纳入管理：先预览（识别来源、与来源逐文件比对、列出会备份 / 删除 / 链接的内容），用户确认后执行。
 * 内容一致时直接纳入；不一致时由用户选「用来源的版本」或「保留我的改动」。
 */
export function SkillAdoptDialog({
  name,
  nameOf,
  onConfirm,
  onClose,
}: {
  name: string;
  nameOf: (agent: string) => string;
  /** 返回执行是否成功；成功后由调用方关闭对话框。 */
  onConfirm: (args: string[], label: string) => Promise<boolean>;
  onClose: () => void;
}) {
  const [preview, setPreview] = useState<AdoptPreview | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [choice, setChoice] = useState<"source" | "local">("source");
  const [busy, setBusy] = useState(false);
  const { dialogRef, onDialogKeyDown } = useDialogFocus<HTMLDivElement>({ onEscape: onClose });

  useEffect(() => {
    let alive = true;
    runEngine<AdoptPreview>("skills:adopt-preview", ["--name", name])
      .then((envelope) => {
        if (!alive) return;
        if (envelope.ok) {
          setPreview(envelope.data);
          const check = envelope.data.source_check;
          // 认不出来源或取不到来源：只能保留本地内容
          if (!check || "error" in check && check.error) setChoice("local");
        } else {
          setError(envelope.errors[0]?.message ?? "预览失败");
        }
      })
      .catch((reason) => alive && setError(reason instanceof Error ? reason.message : String(reason)));
    return () => {
      alive = false;
    };
  }, [name]);

  const check = preview?.source_check && !("error" in preview.source_check && preview.source_check.error)
    ? (preview.source_check as Extract<AdoptPreview["source_check"], { same: boolean }>)
    : null;
  const checkError = preview?.source_check && "error" in preview.source_check ? preview.source_check.error : null;
  const dedupe = preview?.kind === "dedupe";
  const accounts = dedupe ? [] : choice === "source" ? check?.accounts ?? [] : preview?.local?.accounts ?? [];
  const canConfirm = Boolean(preview) && (dedupe || (choice === "source" ? Boolean(check) : Boolean(preview?.local && !preview.local.dest_exists)));

  const confirm = async () => {
    if (!preview) return;
    let args: string[];
    let label: string;
    if (dedupe) {
      args = ["--name", name, "--digest", preview.digest ?? ""];
      label = `清理 ${name} 的多余副本`;
    } else if (choice === "local") {
      args = ["--name", name, "--digest", preview.local?.digest ?? "", "--keep-local"];
      label = `纳入 ${name}（保留本地内容）`;
    } else {
      args = ["--name", name, "--digest", check!.digest, "--path", check!.path];
      if (check!.ref) args.push("--ref", check!.ref);
      if (!check!.same) args.push("--accept-source");
      label = `纳入 ${name}`;
    }
    setBusy(true);
    try {
      await onConfirm(args, label);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-slate-900/40 p-4 animate-fade-in" onClick={(e) => e.target === e.currentTarget && onClose()}>
      <div ref={dialogRef} role="dialog" aria-modal="true" aria-label={`纳入管理 ${name}`} tabIndex={-1} onKeyDown={onDialogKeyDown} className="flex max-h-[88vh] w-full max-w-xl flex-col rounded-xl border bg-card shadow-xl animate-zoom-in-95">
        <div className="flex items-center justify-between border-b px-5 py-3">
          <h3 className="text-base font-semibold text-foreground">{dedupe ? `清理 ${name} 的多余副本` : `纳入管理 ${name}`}</h3>
          <button type="button" onClick={onClose} aria-label="关闭" className="rounded p-1 text-muted-foreground hover:bg-accent">
            <X className="h-4 w-4" aria-hidden />
          </button>
        </div>

        <div data-testid="adopt-body" className="min-h-0 flex-1 space-y-4 overflow-y-auto px-5 py-4 text-sm">
          {error ? (
            <p data-testid="adopt-error" className="text-xs text-destructive">{error}</p>
          ) : !preview ? (
            <p data-testid="adopt-loading" className="text-xs text-muted-foreground">正在识别来源并与来源比对（大仓库可能要一两分钟）…</p>
          ) : dedupe ? (
            <p data-testid="adopt-dedupe" className="text-xs text-foreground">
              {name} 已由来源 <span className="font-medium">{preview.collection?.name}</span> 管理；Codex 共用目录里还有一份，Codex 会加载两次。清理后只留来源这一份。
            </p>
          ) : (
            <>
              <section className="space-y-1 text-xs">
                <p data-testid="adopt-source">
                  来源：
                  {preview.source?.repository ? (
                    <span className="font-mono text-foreground">{shortRepository(preview.source.repository)}</span>
                  ) : (
                    <span className="text-muted-foreground">认不出来</span>
                  )}
                  {check && <span className="text-muted-foreground"> · {check.path}</span>}
                </p>
                {check && (
                  <p data-testid="adopt-version" className="text-muted-foreground">
                    纳入到来源 <span className="text-foreground">{preview.collection?.name}</span>
                    {preview.collection?.exists ? "（已有）" : "（新建）"}，版本 <span className="font-mono text-foreground">{check.adopt_at.label}</span>
                    {check.adopt_at.commit !== check.latest.commit && `，之后可更新到 ${check.latest.label}`}
                    {preview.collection?.linked_as && preview.collection.linked_as !== name && `；在来源里叫 ${preview.collection.linked_as}，账号里按这个名字链接`}
                  </p>
                )}
                {checkError && <p data-testid="adopt-source-error" className="text-warning">{checkError}</p>}
              </section>

              {check && (
                <section data-testid="adopt-compare" className="space-y-1 text-xs">
                  {check.same ? (
                    <p className="text-foreground">现有内容与来源的 {check.adopt_at.label} 完全一致。</p>
                  ) : (
                    <>
                      <p className="text-warning">现有内容与来源的 {check.adopt_at.label} 不一致：</p>
                      {preview.locations.map((location, index) => {
                        const files = check.locations[index]?.files ?? [];
                        return files.length === 0 ? null : (
                          <div key={location.path} className="rounded border p-2">
                            <p className="font-mono text-muted-foreground">{location.path}</p>
                            <ul className="mt-1 space-y-0.5">
                              {files.slice(0, 20).map((change) => (
                                <li key={change.file} className="font-mono">
                                  <span className="text-muted-foreground">{FILE_STATUS[change.status] ?? change.status}</span> {change.file}
                                </li>
                              ))}
                              {files.length > 20 && <li className="text-muted-foreground">另有 {files.length - 20} 处</li>}
                            </ul>
                          </div>
                        );
                      })}
                    </>
                  )}
                </section>
              )}

              {(!check || !check.same) && (
                <fieldset className="space-y-1.5 text-xs">
                  <legend className="mb-1 text-muted-foreground">怎么纳入</legend>
                  <label className={cn("flex items-start gap-2", !check && "opacity-50")}>
                    <input type="radio" name="adopt-choice" data-testid="adopt-choice-source" disabled={!check} checked={choice === "source"} onChange={() => setChoice("source")} />
                    <span>用来源的版本（你现在的内容留在备份里，可撤回）</span>
                  </label>
                  <label className="flex items-start gap-2">
                    <input type="radio" name="adopt-choice" data-testid="adopt-choice-local" checked={choice === "local"} onChange={() => setChoice("local")} />
                    <span>
                      保留我的内容：放进 <span className="font-mono">{preview.local?.dest}</span> 作为本机来源（没有版本跟踪）
                      {preview.local?.dest_exists && <span className="text-destructive">——这个位置已经有内容，不能用</span>}
                    </span>
                  </label>
                </fieldset>
              )}
            </>
          )}

          {preview && (
            <section data-testid="adopt-plan" className="space-y-1 rounded-md border bg-muted/30 p-3 text-xs">
              <p className="font-medium text-foreground">点确认后会：</p>
              <ul className="list-disc space-y-0.5 pl-4 text-muted-foreground">
                {preview.removes.map((path) => (
                  <li key={path}>
                    备份并移走 <span className="font-mono">{path}</span>
                  </li>
                ))}
                {!dedupe && choice === "local" && preview.local && (
                  <li>
                    复制到 <span className="font-mono">{preview.local.dest}</span>，登记为本机来源 {preview.local.collection}
                  </li>
                )}
                {!dedupe && choice === "source" && check && <li>登记来源并下载 {check.path}（只下载这一个目录）</li>}
                {accounts.length > 0 && <li>链接到 {accounts.map(nameOf).join("、")}</li>}
                <li>不改 npx skills 等别的工具的记录；可以在完成提示或操作记录里撤回</li>
              </ul>
            </section>
          )}
        </div>

        <div className="flex items-center justify-end gap-2 border-t px-5 py-3">
          <button type="button" onClick={onClose} className="rounded-md border px-4 py-1.5 text-sm text-muted-foreground hover:bg-accent">
            取消
          </button>
          <button
            type="button"
            data-testid="adopt-confirm"
            disabled={!canConfirm || busy}
            onClick={() => void confirm()}
            className="rounded-md bg-primary px-4 py-1.5 text-sm font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-40"
          >
            {busy ? "正在处理…" : dedupe ? "清理" : "纳入"}
          </button>
        </div>
      </div>
    </div>
  );
}
