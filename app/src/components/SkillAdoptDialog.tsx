import { t } from "../lib/i18n";
import { useEffect, useState } from "react";
import { X } from "lucide-react";
import { runEngine } from "../lib/engine";
import { useDialogFocus } from "../lib/dialogFocus";
import { shortRepository } from "./SkillOtherSources";
import { ProgressSteps } from "./ProgressSteps";
import { newRunId } from "../lib/runId";
import { useEngineProgress } from "../lib/useEngineProgress";
import { cn } from "../lib/utils";
import { ChangedFile, type FileChange } from "./FileDiff";


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
  /** 原来就有它的助手（它所在的助手目录，以及共用目录对应的全部 Codex 助手）。 */
  current_accounts?: string[];
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
  local_error?: string;
  local?: { root?: string; dest: string; dest_exists: boolean; collection: string; digest: string; accounts: string[]; from: string | null; variants: number };
}

const FILE_STATUS: Record<string, string> = {
  get modified() { return t("改过"); },
  get "only-local"() { return t("只在本地"); },
  get "only-source"() { return t("只在来源"); },
};

/**
 * 纳入管理：先预览（识别来源、与来源逐文件比对、列出会备份 / 删除 / 链接的内容），用户确认后执行。
 * 内容一致时直接纳入；不一致时由用户选「用来源的版本」或「保留我的改动」；链接到哪些助手可勾选。
 *
 * 预览只读，可以随时取消；执行会移动目录，中途不能安全停下：执行时不给「取消」、不响应 Esc 与点遮罩，
 * 只能「在后台继续」（关掉窗口，执行照常，结束后弹提示）。两个阶段都按引擎报告的步骤显示进度。
 */
export function SkillAdoptDialog({
  name,
  nameOf,
  onConfirm,
  onClose,
}: {
  name: string;
  nameOf: (agent: string) => string;
  /** 返回执行是否成功；成功后由调用方关闭对话框。runId 用来轮询进度。 */
  onConfirm: (args: string[], label: string, runId: string) => Promise<boolean>;
  onClose: () => void;
}) {
  const [preview, setPreview] = useState<AdoptPreview | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [choice, setChoice] = useState<"source" | "local">("source");
  /** 执行中的运行编号：有值时不能取消。 */
  const [runningId, setRunningId] = useState<string | null>(null);
  const [previewId, setPreviewId] = useState<string | null>(null);
  /** 要链接的助手；预览回来后默认全选。 */
  const [selected, setSelected] = useState<string[]>([]);
  const [dest, setDest] = useState("");
  const [requestedDest, setRequestedDest] = useState<string | null>(null);
  const [sourceRequest, setSourceRequest] = useState(0);
  const running = runningId !== null;
  const progress = useEngineProgress(runningId ?? previewId);
  const close = () => {
    if (!running) onClose();
  };
  const { dialogRef, onDialogKeyDown } = useDialogFocus<HTMLDivElement>({ onEscape: close });

  useEffect(() => {
    let alive = true;
    const runId = newRunId();
    setPreview(null);
    setError(null);
    setPreviewId(runId);
    const args = ["--name", name];
    if (!sourceRequest) args.push("--local-only");
    if (requestedDest !== null) args.push("--dest", requestedDest);
    runEngine<AdoptPreview>("skills:adopt-preview", args, { runId })
      .then((envelope) => {
        if (!alive) return;
        if (envelope.ok) {
          setPreview(envelope.data);
          setDest(envelope.data.local?.root ?? envelope.data.local?.dest.replace(/[\\/][^\\/]+$/, "") ?? requestedDest ?? "~/skills/my-skills");
          const check = envelope.data.source_check;
          const sourceOk = Boolean(check && !("error" in check && check.error));
          // 认不出来源或取不到来源：只能保留本地内容
          setChoice(sourceOk ? "source" : "local");
          const accounts = sourceOk ? (check as { accounts: string[] }).accounts : envelope.data.local?.accounts ?? [];
          if (requestedDest === null) setSelected(accounts);
          else setSelected((previous) => previous.filter((agent) => accounts.includes(agent)));
        } else {
          setError(envelope.errors[0]?.message ?? t("预览失败"));
        }
      })
      .catch((reason) => alive && setError(reason instanceof Error ? reason.message : String(reason)))
      .finally(() => alive && setPreviewId(null));
    return () => {
      alive = false;
    };
  }, [name, requestedDest, sourceRequest]);

  const check = preview?.source_check && !("error" in preview.source_check && preview.source_check.error)
    ? (preview.source_check as Extract<AdoptPreview["source_check"], { same: boolean }>)
    : null;
  const checkError = preview?.source_check && "error" in preview.source_check ? preview.source_check.error : null;
  const dedupe = preview?.kind === "dedupe";
  const accounts = dedupe ? [] : choice === "source" ? check?.accounts ?? [] : preview?.local?.accounts ?? [];
  const chosen = accounts.filter((agent) => selected.includes(agent));
  const previewRoot = preview?.local?.root ?? preview?.local?.dest.replace(/[\\/][^\\/]+$/, "");
  const destDirty = dest.trim() !== previewRoot;
  const destOk = /^(~\/|\/|[a-zA-Z]:[\\/])/.test(dest.trim());
  const canConfirm =
    Boolean(preview) && !previewId &&
    (dedupe || choice !== "local" || !destDirty) &&
    (dedupe || ((choice === "source" ? Boolean(check) : Boolean(preview?.local && !preview.local.dest_exists)) && chosen.length > 0));

  const confirm = async () => {
    if (!preview || !canConfirm) return;
    let args: string[];
    let label: string;
    if (dedupe) {
      args = ["--name", name, "--digest", preview.digest ?? ""];
      label = t("清理 {0} 的多余副本", name);
    } else if (choice === "local") {
      args = ["--name", name, "--digest", preview.local?.digest ?? "", "--keep-local"];
      args.push("--dest", previewRoot!);
      label = t("纳入 {0}（保留本地内容）", name);
    } else {
      args = ["--name", name, "--digest", check!.digest, "--path", check!.path];
      if (check!.ref) args.push("--ref", check!.ref);
      if (!check!.same) args.push("--accept-source");
      label = t("纳入 {0}", name);
    }
    if (!dedupe) args.push("--agents", chosen.join(","));
    const runId = newRunId();
    setRunningId(runId);
    try {
      await onConfirm(args, label, runId);
    } finally {
      setRunningId(null);
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-slate-900/40 p-4 animate-fade-in" onClick={(e) => e.target === e.currentTarget && close()}>
      <div ref={dialogRef} role="dialog" aria-modal="true" aria-label={t("纳入管理 {0}", name)} tabIndex={-1} onKeyDown={onDialogKeyDown} className="flex max-h-[88vh] w-full max-w-xl flex-col rounded-xl border bg-card shadow-xl animate-zoom-in-95">
        <div className="flex items-center justify-between border-b px-5 py-3">
          <h3 className="text-base font-semibold text-foreground">{dedupe ? t("清理 {0} 的多余副本", name) : t("纳入管理 {0}", name)}</h3>
          {!running && (
            <button type="button" onClick={onClose} aria-label={t("关闭")} className="rounded p-1 text-muted-foreground hover:bg-accent">
              <X className="h-4 w-4" aria-hidden />
            </button>
          )}
        </div>

        <div data-testid="adopt-body" className="min-h-0 flex-1 space-y-4 overflow-y-auto px-5 py-4 text-sm">
          {running ? (
            <ProgressSteps testId="adopt-progress" progress={progress} fallback={dedupe ? t("正在清理…") : t("正在纳入…")} />
          ) : error ? (
            <p data-testid="adopt-error" className="text-xs text-destructive">{error}</p>
          ) : !preview ? (
            <ProgressSteps
              testId="adopt-loading"
              progress={progress}
              fallback={sourceRequest ? t("正在识别来源并与来源比对。预览不会改动任何东西，可以随时取消。") : t("正在读取本机内容，预览不会改动任何东西。")}
            />
          ) : dedupe ? (
            <p data-testid="adopt-dedupe" className="text-xs text-foreground">
              {name}{(" " + t("已由来源") + " ")}<span className="font-medium">{preview.collection?.name}</span>{(" " + t("管理；Codex 共用目录里还有一份，Codex 会加载两次。清理后只留来源这一份。"))}</p>
          ) : (
            <>
              <section className="space-y-1 text-xs">
                <p data-testid="adopt-source">{t("来源：")}{preview.source?.repository ? (
                    <span className="font-mono text-foreground">{shortRepository(preview.source.repository)}</span>
                  ) : (
                    <span className="text-muted-foreground">{t("认不出来")}</span>
                  )}
                  {check && <span className="text-muted-foreground"> · {check.path}</span>}
                </p>
                {check && (
                  <p data-testid="adopt-version" className="text-muted-foreground">{(t("纳入到来源") + " ")}<span className="text-foreground">{preview.collection?.name}</span>
                    {preview.collection?.exists ? t("（已有）") : t("（新建）")}{(t("，版本") + " ")}<span className="font-mono text-foreground">{check.adopt_at.label}</span>
                    {check.adopt_at.commit !== check.latest.commit && t("，之后可更新到 {0}", check.latest.label)}
                    {preview.collection?.linked_as && preview.collection.linked_as !== name && t("；在来源里叫 {0}，助手里按这个名字链接", preview.collection.linked_as)}
                  </p>
                )}
                {checkError && <p data-testid="adopt-source-error" className="text-warning">{checkError}</p>}
                {preview.source?.repository && <button type="button" onClick={() => setSourceRequest((value) => value + 1)} className="rounded-md border px-2 py-1 hover:bg-accent">{t("比对远程来源")}</button>}
              </section>

              {check && (
                <section data-testid="adopt-compare" className="space-y-1 text-xs">
                  {check.same ? (
                    <p className="text-foreground">{(t("现有内容与来源的") + " ")}{check.adopt_at.label}{(" " + t("完全一致。"))}</p>
                  ) : (
                    <>
                      <p className="text-warning">{(t("现有内容与来源的") + " ")}{check.adopt_at.label}{(" " + t("不一致："))}</p>
                      {preview.locations.map((location, index) => {
                        const files = check.locations[index]?.files ?? [];
                        return files.length === 0 ? null : (
                          <div key={location.path} className="rounded border p-2">
                            <p className="font-mono text-muted-foreground">{location.path}</p>
                            <ul className="mt-1 space-y-0.5">
                              {files.slice(0, 20).map((change) => (
                                <ChangedFile key={change.file} change={change} label={FILE_STATUS[change.status] ?? change.status} />
                              ))}
                              {files.length > 20 && <li className="text-muted-foreground">{(t("另有") + " ")}{files.length - 20}{(" " + t("处"))}</li>}
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
                  <legend className="mb-1 text-muted-foreground">{t("怎么纳入")}</legend>
                  <label className={cn("flex items-start gap-2", !check && "opacity-50")}>
                    <input type="radio" name="adopt-choice" data-testid="adopt-choice-source" disabled={!check} checked={choice === "source"} onChange={() => setChoice("source")} />
                    <span>{t("用来源的版本（你现在的内容留在备份里，可撤回）")}</span>
                  </label>
                  <label className="flex items-start gap-2">
                    <input type="radio" name="adopt-choice" data-testid="adopt-choice-local" checked={choice === "local"} onChange={() => setChoice("local")} />
                    <span>{(t("保留我的内容：放进") + " ")}<span className="font-mono">{preview.local?.dest}</span>{(" " + t("作为本机来源（没有版本跟踪）"))}{preview.local?.dest_exists && <span className="text-destructive">{t("——这个位置已经有内容，不能用")}</span>}
                    </span>
                  </label>
                </fieldset>
              )}
              {choice === "local" && (
                <section className="space-y-2 text-xs">
                  <label className="block">
                    <span className="text-muted-foreground">{t("个人集合目录（可自定义）")}</span>
                    <input data-testid="adopt-dest" value={dest} onChange={(event) => setDest(event.target.value)} className="mt-1 w-full rounded-md border border-input bg-background px-2 py-1.5 font-mono text-xs" />
                  </label>
                  {preview.local_error && <p className="text-destructive">{preview.local_error}</p>}
                  {destDirty && (
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="text-muted-foreground">{t("目录已修改，请更新预览。")}</span>
                      <button type="button" data-testid="adopt-refresh-preview" disabled={!destOk} onClick={() => { setSourceRequest(0); setRequestedDest(dest.trim()); }} className="rounded-md border px-2 py-1 disabled:opacity-40">{t("更新预览")}</button>
                    </div>
                  )}
                </section>
              )}
            </>
          )}

          {preview && !running && !dedupe && accounts.length > 0 && (
            <fieldset data-testid="adopt-accounts" className="space-y-1 text-xs">
              <legend className="mb-1 text-muted-foreground">{t("链接到哪些助手（默认全部）")}</legend>
              <div className="flex flex-wrap gap-x-4 gap-y-1">
                {accounts.map((agent) => (
                  <label key={agent} className="flex items-center gap-1.5 text-foreground">
                    <input
                      type="checkbox"
                      data-testid={`adopt-account-${agent}`}
                      checked={selected.includes(agent)}
                      onChange={(e) => setSelected((prev) => (e.target.checked ? [...prev, agent] : prev.filter((item) => item !== agent)))}
                    />
                    {nameOf(agent)}
                    {preview.current_accounts?.includes(agent) && (
                      <span className="rounded bg-muted px-1 py-0.5 text-[10px] text-muted-foreground">{t("原来就有")}</span>
                    )}
                  </label>
                ))}
              </div>
              {chosen.length === 0 && <p className="text-destructive">{t("至少选一个助手")}</p>}
            </fieldset>
          )}

          {preview && !running && (
            <section data-testid="adopt-plan" className="space-y-1 break-words rounded-md border bg-muted/30 p-3 text-xs [overflow-wrap:anywhere]">
              <p className="font-medium text-foreground">{t("点确认后会：")}</p>
              <ul className="list-disc space-y-0.5 pl-4 text-muted-foreground">
                {preview.removes.map((path) => (
                  <li key={path}>{(t("备份并移走") + " ")}<span className="font-mono">{path}</span>
                  </li>
                ))}
                {!dedupe && choice === "local" && preview.local && (
                  <li>{(t("复制到") + " ")}<span className="font-mono">{preview.local.dest}</span>{(t("，登记为本机来源") + " ")}{preview.local.collection}
                  </li>
                )}
                {!dedupe && choice === "source" && check && <li>{(t("登记来源并下载") + " ")}{check.path}{t("（只下载这一个目录）")}</li>}
                {chosen.length > 0 && <li>{(t("链接到") + " ")}{chosen.map(nameOf).join("、")}</li>}
                <li>{t("不改 npx skills 等别的工具的记录；可以在完成提示或操作记录里撤回")}</li>
              </ul>
            </section>
          )}
        </div>

        <div className="flex items-center justify-end gap-2 border-t px-5 py-3">
          {running ? (
            <>
              <span data-testid="adopt-running-note" className="mr-auto text-xs text-muted-foreground">{t("执行中不能取消；出错会自动恢复原样")}</span>
              <button
                type="button"
                data-testid="adopt-background"
                onClick={onClose}
                className="rounded-md border px-4 py-1.5 text-sm text-muted-foreground hover:bg-accent"
              >{t("在后台继续")}</button>
            </>
          ) : (
            <>
              <button type="button" data-testid="adopt-cancel" onClick={onClose} className="rounded-md border px-4 py-1.5 text-sm text-muted-foreground hover:bg-accent">{t("取消")}</button>
              <button
                type="button"
                data-testid="adopt-confirm"
                disabled={!canConfirm}
                onClick={() => void confirm()}
                className="rounded-md bg-primary px-4 py-1.5 text-sm font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-40"
              >
                {dedupe ? t("清理") : t("纳入")}
              </button>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
