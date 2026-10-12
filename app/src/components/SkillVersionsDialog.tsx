import { useEffect, useState } from "react";
import { RefreshCw, X } from "lucide-react";
import { runEngine } from "../lib/engine";
import { useDialogFocus } from "../lib/dialogFocus";
import { describeTrack, versionLabel, type TrackSpec, type VersionRef } from "../lib/skillTrack";
import { cn } from "../lib/utils";
import { ChangedFile, type FileChange } from "./FileDiff";

interface PreviewData {
  from: VersionRef | null;
  to: VersionRef;
  downloaded: boolean;
  same: boolean;
  skills: { name: string; path: string; files: FileChange[]; accounts: string[] }[];
  rules: FileChange[];
  other: { status: string; file: string }[];
}

const CHANGE: Record<string, string> = { added: "新增", deleted: "删除", modified: "修改" };

/** 整个技能是新加的还是被删掉的：看它的 SKILL.md。 */
function skillChange(files: FileChange[]): "added" | "deleted" | null {
  const entry = files.find((change) => change.file === "SKILL.md");
  return entry?.status === "added" || entry?.status === "deleted" ? entry.status : null;
}

interface VersionsData {
  collection: string;
  track: TrackSpec;
  /** 自动跟踪时实际按什么走：tag = 版本，default = 主分支。 */
  auto_resolved?: "tag" | "default";
  latest: (VersionRef & { error?: undefined }) | { error: string };
  current: VersionRef | null;
  default_branch: string | null;
  tags: { name: string; commit: string; version: boolean; prerelease: boolean }[];
  tags_total: number;
  branches: { name: string; commit: string }[];
  branches_total: number;
}

/**
 * 一个 git 来源的版本：跟踪方式、当前与最新版本，可更新到最新或切换到任意标签 / 分支（即回滚）。
 * 打开时读一次远端清单（只读网络，不写）；切换由调用方执行：先把镜像换到那个版本，
 * 再把各助手在用的技能换过去。
 */
export function SkillVersionsDialog({
  collectionId,
  name,
  busy,
  openLatest = false,
  nameOf = (agent: string) => agent,
  onSwitch,
  onClose,
}: {
  collectionId: string;
  name: string;
  busy: boolean;
  /** 打开时直接预览「更新到最新」（来源行的更新按钮）。 */
  openLatest?: boolean;
  nameOf?: (agent: string) => string;
  /** ref 为 null 表示按跟踪方式更新到最新。 */
  onSwitch: (ref: string | null, label: string) => Promise<boolean>;
  onClose: () => void;
}) {
  const [data, setData] = useState<VersionsData | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [tab, setTab] = useState<"tags" | "branches">("tags");
  const [reload, setReload] = useState(0);
  /** 确认前的预览：要换到哪个版本、各技能改了什么、哪些助手跟着换。 */
  const [pending, setPending] = useState<{ ref: string | null; label: string; data: PreviewData | null; error: string | null } | null>(null);
  // 切换中（镜像已在换、助手链接在跟着换）不能安全停下：不响应 Esc、遮罩与关闭
  const close = () => {
    if (!busy) onClose();
  };
  const { dialogRef, onDialogKeyDown } = useDialogFocus<HTMLDivElement>({ onEscape: close });

  useEffect(() => {
    let alive = true;
    setError(null);
    runEngine<VersionsData>("skills:versions", ["--only", collectionId])
      .then((envelope) => {
        if (!alive) return;
        if (envelope.ok) {
          setData(envelope.data);
          if (envelope.data.tags.length === 0) setTab("branches");
        } else {
          setError(envelope.errors[0]?.message ?? "读不到版本清单");
        }
      })
      .catch((reason) => alive && setError(reason instanceof Error ? reason.message : String(reason)));
    return () => {
      alive = false;
    };
  }, [collectionId, reload]);

  const current = data?.current ?? null;
  const latest = data && !("error" in data.latest && data.latest.error) ? (data.latest as VersionRef) : null;
  const behind = Boolean(latest && current && latest.commit !== current.commit);
  const previewSwitch = (ref: string | null, label: string) => {
    setPending({ ref, label, data: null, error: null });
    runEngine<PreviewData>("skills:preview", ["--only", collectionId, ...(ref ? ["--ref", ref] : [])])
      .then((envelope) =>
        setPending((prev) =>
          prev && prev.ref === ref
            ? envelope.ok
              ? { ...prev, data: envelope.data }
              : { ...prev, error: envelope.errors[0]?.message ?? "预览失败" }
            : prev,
        ),
      )
      .catch((reason) => setPending((prev) => (prev ? { ...prev, error: reason instanceof Error ? reason.message : String(reason) } : prev)));
  };
  const confirmSwitch = async () => {
    if (!pending) return;
    if (await onSwitch(pending.ref, pending.label)) {
      setPending(null);
      setReload((n) => n + 1);
    }
  };
  useEffect(() => {
    // 来源行的「更新」：打开就预览换到最新
    if (openLatest) previewSwitch(null, "最新");
  }, []);
  const rows = data ? (tab === "tags" ? data.tags : data.branches) : [];
  const total = data ? (tab === "tags" ? data.tags_total : data.branches_total) : 0;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-slate-900/40 p-4 animate-fade-in" onClick={(e) => e.target === e.currentTarget && close()}>
      <div ref={dialogRef} role="dialog" aria-modal="true" aria-label={`${name} 的版本`} tabIndex={-1} onKeyDown={onDialogKeyDown} className="flex max-h-[85vh] w-full max-w-lg flex-col rounded-xl border bg-card shadow-xl animate-zoom-in-95">
        <div className="flex items-center justify-between border-b px-5 py-3">
          <h3 className="text-base font-semibold text-foreground">{name} 的版本</h3>
          {!busy && (
            <button type="button" onClick={onClose} aria-label="关闭" className="rounded p-1 text-muted-foreground hover:bg-accent">
              <X className="h-4 w-4" aria-hidden />
            </button>
          )}
        </div>

        <div className="space-y-2 border-b px-5 py-3 text-sm">
          {error ? (
            <p data-testid="versions-error" className="text-xs text-destructive">{error}</p>
          ) : !data ? (
            <p className="text-xs text-muted-foreground">正在读取远端的版本…</p>
          ) : (
            <>
              <p data-testid="versions-summary" className="text-xs text-muted-foreground">
                跟踪：{describeTrack(data.track)}
                {data.auto_resolved && `（${data.auto_resolved === "tag" ? "按版本" : "按主分支"}）`}
                {" · "}当前 <span className="font-mono text-foreground">{versionLabel(current) ?? "未下载"}</span>
                {" · "}最新 <span className="font-mono text-foreground">{latest ? versionLabel(latest) : "—"}</span>
              </p>
              {"error" in data.latest && data.latest.error && (
                <p data-testid="versions-latest-error" className="text-xs text-warning">{data.latest.error}</p>
              )}
              {(behind || !current) && latest && (
                <button
                  type="button"
                  data-testid="versions-update-latest"
                  disabled={busy}
                  onClick={() => previewSwitch(null, versionLabel(latest) ?? "最新")}
                  className="inline-flex items-center gap-1 rounded-md bg-primary px-3 py-1.5 text-xs text-primary-foreground hover:bg-primary/90 disabled:opacity-40"
                >
                  <RefreshCw className={cn("h-3.5 w-3.5", busy && "animate-spin")} aria-hidden /> 更新到 {versionLabel(latest)}
                </button>
              )}
              <p className="text-[11px] text-muted-foreground">切换后，各助手在用的技能一起换到那个版本，停用的不动。要一直停在某个版本，把跟踪改成「固定」。</p>
            </>
          )}
        </div>

        {pending && (
          <div data-testid="versions-preview" className="min-h-0 flex-1 space-y-2 overflow-y-auto px-5 py-3 text-xs">
            {pending.error ? (
              <p data-testid="versions-preview-error" className="text-destructive">{pending.error}</p>
            ) : !pending.data ? (
              <p className="text-muted-foreground">正在取 {pending.label} 并比较…</p>
            ) : (
              <>
                <p className="text-foreground">
                  {versionLabel(pending.data.from) ?? "未下载"} → <span className="font-mono">{versionLabel(pending.data.to)}</span>
                </p>
                {!pending.data.downloaded ? (
                  <p className="text-muted-foreground">还没下载过，这次是首次下载。</p>
                ) : pending.data.same ? (
                  <p data-testid="versions-preview-same" className="text-muted-foreground">已经是这个版本，内容没有变化。</p>
                ) : (
                  <>
                    {pending.data.skills.length === 0 && pending.data.rules.length === 0 && (
                      <p className="text-muted-foreground">用到的技能与规则都没有变化。</p>
                    )}
                    {pending.data.skills.map((skill) => (
                      <div key={skill.name} data-testid={`preview-skill-${skill.name}`} className="rounded border p-2">
                        <p className="font-medium text-foreground">
                          {skill.name}
                          {skillChange(skill.files) === "added" && (
                            <span data-testid={`preview-skill-new-${skill.name}`} className="ml-2 rounded bg-info/10 px-1.5 py-0.5 text-[11px] font-normal text-info">
                              新技能
                            </span>
                          )}
                          {skillChange(skill.files) === "deleted" && (
                            <span data-testid={`preview-skill-gone-${skill.name}`} className="ml-2 rounded bg-warning/15 px-1.5 py-0.5 text-[11px] font-normal text-warning">
                              这一版删掉了它
                            </span>
                          )}
                          <span className="ml-2 font-normal text-muted-foreground">
                            {skill.accounts.length
                              ? `跟着换：${skill.accounts.map(nameOf).join("、")}`
                              : skillChange(skill.files) === "added"
                                ? "换过去后出现在技能页，由你决定给哪些助手"
                                : "没有助手在用"}
                          </span>
                        </p>
                        <ul className="mt-1 space-y-0.5 font-mono">
                          {skill.files.slice(0, 15).map((change) => (
                            <ChangedFile key={change.file} change={change} label={CHANGE[change.status] ?? change.status} />
                          ))}
                          {skill.files.length > 15 && <li className="text-muted-foreground">另有 {skill.files.length - 15} 处</li>}
                        </ul>
                      </div>
                    ))}
                    {pending.data.rules.length > 0 && (
                      <p data-testid="preview-rules" className="text-muted-foreground">
                        规则变化 {pending.data.rules.length} 处（规则页会提示「来源有新版」，确认后才覆盖）
                      </p>
                    )}
                  </>
                )}
              </>
            )}
            <div className="sticky -bottom-3 z-10 -mx-5 -mb-3 flex items-center justify-end gap-2 border-t bg-card px-5 py-2">
              {busy ? (
                <span data-testid="versions-switching-note" className="mr-auto text-muted-foreground">
                  正在下载并把在用的技能换过去，完成前不能取消
                </span>
              ) : (
                <button type="button" onClick={() => setPending(null)} className="rounded-md border px-3 py-1 text-xs text-muted-foreground hover:bg-accent">
                  返回
                </button>
              )}
              <button
                type="button"
                data-testid="versions-preview-confirm"
                disabled={busy || !pending.data || pending.data.same}
                onClick={() => void confirmSwitch()}
                className="rounded-md bg-primary px-3 py-1 text-xs text-primary-foreground hover:bg-primary/90 disabled:opacity-40"
              >
                {busy ? "正在切换…" : `确认换到 ${pending.label}`}
              </button>
            </div>
          </div>
        )}

        {data && !pending && (
          <>
            <div className="flex gap-1 px-5 pt-2 text-xs">
              {(["tags", "branches"] as const).map((key) => (
                <button
                  key={key}
                  type="button"
                  data-testid={`versions-tab-${key}`}
                  onClick={() => setTab(key)}
                  className={cn("rounded px-2.5 py-1", tab === key ? "bg-primary/15 font-medium text-primary" : "text-muted-foreground hover:text-foreground")}
                >
                  {key === "tags" ? `版本 ${data.tags_total}` : `分支 ${data.branches_total}`}
                </button>
              ))}
            </div>
            <ul data-testid="versions-list" className="min-h-0 flex-1 divide-y overflow-y-auto px-5 py-2">
              {rows.length === 0 && <li className="py-2 text-xs text-muted-foreground">{tab === "tags" ? "这个仓库没有标签" : "没有分支"}</li>}
              {rows.map((item) => {
                const isCurrent = current?.commit === item.commit && (tab === "branches" || current?.kind !== "branch");
                return (
                  <li key={item.name} data-testid={`version-${item.name}`} className="flex items-center gap-2 py-1.5">
                    <span className="min-w-0 flex-1 truncate font-mono text-xs text-foreground" title={item.commit}>{item.name}</span>
                    {(item as { prerelease?: boolean }).prerelease === true && <span className="rounded bg-muted px-1.5 py-0.5 text-[11px] text-muted-foreground">预发布</span>}
                    {tab === "branches" && item.name === data.default_branch && <span className="rounded bg-muted px-1.5 py-0.5 text-[11px] text-muted-foreground">主分支</span>}
                    {latest?.commit === item.commit && <span className="rounded bg-info/10 px-1.5 py-0.5 text-[11px] text-info">最新</span>}
                    {isCurrent ? (
                      <span className="rounded bg-primary/10 px-1.5 py-0.5 text-[11px] text-primary">当前</span>
                    ) : (
                      <button
                        type="button"
                        data-testid={`version-switch-${item.name}`}
                        disabled={busy}
                        onClick={() => previewSwitch(item.name, item.name)}
                        className="rounded border px-2 py-0.5 text-xs hover:bg-accent disabled:opacity-40"
                      >
                        切换
                      </button>
                    )}
                  </li>
                );
              })}
              {total > rows.length && <li className="py-2 text-[11px] text-muted-foreground">只列出前 {rows.length} 个（共 {total} 个）</li>}
            </ul>
          </>
        )}
      </div>
    </div>
  );
}
