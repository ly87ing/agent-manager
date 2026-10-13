import { useState } from "react";
import { isRemoteGitRepository } from "../lib/gitRepository";
import { ChevronRight, X } from "lucide-react";
import { useDialogFocus } from "../lib/dialogFocus";
import { type AvailableSkill, canAutoInclude, readScope, toSkillPaths } from "../lib/skillScope";
import { cn } from "../lib/utils";
import { TRACK_OPTIONS, trackFromForm, trackToForm, type TrackMode, type TrackSpec } from "../lib/skillTrack";
import { Select } from "./Select";

const AGENT_LABEL: Record<string, string> = { "claude-code": "Claude", codex: "Codex" };

/**
 * 技能集合定义表单。保存只把定义写入本地资源库，不自动安装或同步。
 * 编辑时若有完整原定义（fullDefinition，来自 policy:get 的 sqlite 有效定义），
 * 以它为基底只覆盖表单负责字段，整条替换保存，表单未展示的字段（如 branch）不丢；
 * 拿不到完整原定义（无 DB 旧存储）时只提交表单负责字段，由保存方迁移后取完整定义合并再整条替换。
 */
export function SkillCollectionFormDialog({
  agentIds,
  initial,
  draft,
  fullDefinition,
  available,
  repoCached,
  onSubmit,
  onClose,
}: {
  agentIds: string[];
  /** 来源里全部技能（引擎 observed.available，不受 skill_paths 限制）；用来把「用哪些技能」做成勾选。 */
  available?: AvailableSkill[];
  /** git 来源是否已把仓库取到本地——没取下来就没有候选可勾。 */
  repoCached?: boolean;
  initial?: { name: string; definition: Record<string, unknown> };
  /** 新增时的预填（例如来自分享链接）；和编辑不同，名称仍可改。 */
  draft?: { name: string; definition: Record<string, unknown> };
  /** 完整原定义（整条替换基底）；新增或未传入时为 null/缺省。 */
  fullDefinition?: Record<string, unknown> | null;
  /** complete=false 表示 definition 只是表单负责字段，需由保存方迁移后取完整定义合并再整条替换。 */
  onSubmit: (name: string, definition: Record<string, unknown>, complete: boolean) => void;
  onClose: () => void;
}) {
  const seed = initial ?? draft;
  const [name, setName] = useState(seed?.name ?? "");
  /** 来源二选一：git 仓库（下载到本地后按版本分发）或本地目录（直接链到该目录，内容随其更新）。 */
  const [sourceKind, setSourceKind] = useState<"git" | "local">(seed?.definition.source_path ? "local" : "git");
  const [url, setUrl] = useState(String(seed?.definition.url ?? ""));
  const [sourcePath, setSourcePath] = useState(String(seed?.definition.source_path ?? ""));
  /** 集合里规则的位置（相对集合根）：子目录或某个 .md 文件；留空用默认的 rules/。 */
  const [rulesPath, setRulesPath] = useState(String(seed?.definition.rules_path ?? ""));
  const initialPaths = (seed?.definition.skill_paths as string[] | null | undefined) ?? null;
  const skills = available ?? [];
  const listed = skills.length > 0;
  /** 来源本身就是一个技能文件夹：没有可挑的。 */
  const single = skills.length === 1 && skills[0].path === "";
  const initialScope = readScope(initialPaths, skills);
  const [selected, setSelected] = useState<string[]>(initialScope.selected);
  const [auto, setAuto] = useState(initialScope.auto);
  const [keptMissing, setKeptMissing] = useState<string[]>(initialScope.missing);
  /** 列不出候选（仓库没取回、新建）时直接填：每行一项，空着表示来源里全部技能。 */
  const [manualText, setManualText] = useState((initialPaths ?? []).join("\n"));
  const [selectedAgents, setSelectedAgents] = useState<string[]>((initial?.definition.agents as string[] | undefined) ?? []);
  /** git 来源的跟踪方式：自动 / 版本 / 主分支 / 指定分支 / 匹配分支 / 固定。 */
  const [track, setTrack] = useState(() => trackToForm(seed?.definition.track as TrackSpec | null | undefined));
  const [error, setError] = useState<string | null>(null);
  const { dialogRef, onDialogKeyDown } = useDialogFocus<HTMLDivElement>({ onEscape: onClose });

  const allSelected = listed && skills.every((skill) => selected.includes(skill.name));
  const autoPossible = allSelected && keptMissing.length === 0 && canAutoInclude(skills);
  const toggle = (value: string) =>
    setSelected((prev) => (prev.includes(value) ? prev.filter((item) => item !== value) : [...prev, value]));

  const submit = () => {
    if (!/^[a-z0-9][a-z0-9-]*$/.test(name)) {
      setError("名称只能用小写字母/数字/连字符（如 my-skills）");
      return;
    }
    if (sourceKind === "git" && !isRemoteGitRepository(url.trim()) && !/^file:\/\/\/[^\x00-\x1f\x7f]+$/.test(url.trim())) {
      setError("请输入 HTTPS、SSH、git:// 或 file:// 仓库地址");
      return;
    }
    if (sourceKind === "local" && !/^(~\/|\/)/.test(sourcePath.trim())) {
      setError("本地目录要填绝对路径或以 ~/ 开头");
      return;
    }
    const rulesRel = rulesPath.trim().replace(/^\.\//, "");
    if (rulesRel && (rulesRel.startsWith("/") || rulesRel.startsWith("~") || rulesRel.split("/").includes(".."))) {
      setError("规则位置填集合里的相对路径，例如 rules 或 docs/CLAUDE.md");
      return;
    }
    const trackResult = sourceKind === "git" ? trackFromForm(track) : { track: null };
    if ("error" in trackResult) {
      setError(trackResult.error);
      return;
    }
    // 直接选了某个技能的 SKILL.md：来源就是它所在的那个技能文件夹
    const localPath = sourcePath.trim().replace(/\/SKILL\.md$/i, "");
    let skillPaths: string[] | null;
    if (single) {
      skillPaths = initialPaths;
    } else if (listed) {
      skillPaths = toSkillPaths({ available: skills, selected, auto: autoPossible && auto, keptMissing, initialPaths });
      if (skillPaths !== null && skillPaths.length === 0) {
        setError("至少勾一个技能；一个都不用的话，可以在集合卡片上点「停用」");
        return;
      }
    } else {
      const lines = manualText.split("\n").map((line) => line.trim()).filter(Boolean);
      skillPaths = lines.length ? [...new Set(lines)] : null;
    }
    // 表单负责字段（可选字段显式给出，null 表示清空）；编辑且有完整原定义时以其为基底，
    // 未展示字段原样保留；否则只提交这些字段做片段合并。
    // 两种来源互斥：保存哪一种，另一种就显式清空，避免遗留字段让引擎拒绝。
    const owned: Record<string, unknown> = {
      url: sourceKind === "git" ? url.trim() : null,
      source_path: sourceKind === "local" ? localPath : null,
      rules_path: rulesRel || null,
      skill_paths: skillPaths,
      agents: selectedAgents.length ? selectedAgents : null,
      track: trackResult.track,
    };
    const complete = !initial || fullDefinition != null;
    const definition = initial && fullDefinition ? { ...fullDefinition, ...owned } : owned;
    onSubmit(name, definition, complete);
  };

  const inputClass = "w-full rounded-md border border-input bg-background px-2.5 py-1.5 font-mono text-xs";
  const sectionTitle = "mb-1.5 block text-xs font-medium text-muted-foreground";

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-slate-900/40 p-4 animate-fade-in" onClick={(e) => e.target === e.currentTarget && onClose()}>
      <div ref={dialogRef} role="dialog" aria-modal="true" aria-label={initial ? "编辑来源" : "添加来源"} tabIndex={-1} onKeyDown={onDialogKeyDown} className="flex max-h-[90vh] w-full max-w-xl flex-col rounded-xl border bg-card shadow-xl animate-zoom-in-95">
        <div className="flex items-center justify-between border-b px-5 py-3">
          <h3 className="text-base font-semibold text-foreground">{initial ? `编辑来源 ${initial.name}` : "添加来源"}</h3>
          <button type="button" onClick={onClose} aria-label="关闭" className="rounded p-1 text-muted-foreground hover:bg-accent">
            <X className="h-4 w-4" aria-hidden />
          </button>
        </div>

        <div className="min-h-0 flex-1 space-y-5 overflow-y-auto px-5 py-4 text-sm">
          {!initial && (
            <div>
              <label className={sectionTitle} htmlFor="collection-name">名称（小写字母、数字、连字符）</label>
              <input id="collection-name" value={name} onChange={(e) => setName(e.target.value)} data-testid="collection-name-input" aria-label="集合名称" className={inputClass} placeholder="my-skills" />
            </div>
          )}

          <div>
            <div className="mb-1.5 flex items-center gap-3">
              <span className="text-xs font-medium text-muted-foreground">来源</span>
              <div className="inline-flex rounded-md border p-0.5 text-xs">
                {([
                  { key: "git", label: "git 仓库" },
                  { key: "local", label: "本机目录" },
                ] as const).map((option) => (
                  <label
                    key={option.key}
                    className={cn(
                      "cursor-pointer rounded px-2.5 py-1",
                      sourceKind === option.key ? "bg-primary/15 font-medium text-primary" : "text-muted-foreground hover:text-foreground",
                    )}
                  >
                    <input
                      type="radio"
                      name="collection-source-kind"
                      className="sr-only"
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
              <>
                <input value={url} onChange={(e) => setUrl(e.target.value)} data-testid="collection-url-input" aria-label="仓库地址" className={inputClass} placeholder="https://github.com/example/skills.git" />
                <p className="mt-1 text-xs text-muted-foreground">推荐 HTTPS；也支持 SSH、git://、file://</p>
                <div className="mt-2 flex flex-wrap items-center gap-2">
                  <label htmlFor="collection-track-mode" className="text-xs text-muted-foreground">跟踪</label>
                  <Select
                    id="collection-track-mode"
                    label="跟踪方式"
                    testId="collection-track-mode"
                    value={track.mode}
                    onChange={(next) => setTrack({ mode: next as TrackMode, value: "", prerelease: false })}
                    className="rounded-md border bg-background px-2 py-1 text-xs"
                    options={TRACK_OPTIONS.map((option) => ({ value: option.mode, label: option.label }))}
                  />
                  {(track.mode === "tag" || track.mode === "branch" || track.mode === "branch-pattern" || track.mode === "pinned") && (
                    <input
                      data-testid="collection-track-value"
                      aria-label="跟踪的版本、分支或规则"
                      value={track.value}
                      onChange={(e) => setTrack({ ...track, value: e.target.value })}
                      className="w-40 rounded-md border border-input bg-background px-2 py-1 font-mono text-xs"
                      placeholder={{ tag: "v*（可空）", branch: "stable", "branch-pattern": "release/*", pinned: "v3.0.1" }[track.mode]}
                    />
                  )}
                  {track.mode === "tag" && (
                    <label className="flex items-center gap-1 text-xs text-muted-foreground">
                      <input type="checkbox" data-testid="collection-track-prerelease" checked={track.prerelease} onChange={(e) => setTrack({ ...track, prerelease: e.target.checked })} />
                      含预发布（-rc、-beta）
                    </label>
                  )}
                </div>
              </>
            ) : (
              <>
                <input value={sourcePath} onChange={(e) => setSourcePath(e.target.value)} data-testid="collection-source-path-input" aria-label="本机位置" className={inputClass} placeholder={`~/skills/${name || "my-skills"}`} />
                <p className="mt-1 text-[11px] text-muted-foreground">装着多个技能的目录，或直接是一个技能文件夹。助手里的链接直接指向它，内容在原处维护。</p>
              </>
            )}
          </div>

          <div data-testid="collection-skills">
            {single ? (
              <p className="text-xs text-muted-foreground">这个来源本身就是一个技能：{skills[0].name}。</p>
            ) : listed ? (
              <>
                <div className="mb-1.5 flex items-center gap-2">
                  <span className="text-xs font-medium text-muted-foreground">用哪些技能</span>
                  {sourceKind === "git" && (
                    <span className="text-[11px] text-muted-foreground" title="规则目录与 aliases/ 也会一起取回">只下载勾选的目录</span>
                  )}
                  <span data-testid="collection-paths-count" className="text-xs text-muted-foreground">已选 {selected.length} / {skills.length}</span>
                  <div className="ml-auto flex gap-1">
                    <button type="button" data-testid="collection-paths-select-all" onClick={() => setSelected(skills.map((skill) => skill.name))} className="rounded px-2 py-0.5 text-[11px] text-primary hover:bg-primary/10">全选</button>
                    <button type="button" data-testid="collection-paths-clear" onClick={() => setSelected([])} className="rounded px-2 py-0.5 text-[11px] text-muted-foreground hover:bg-accent">全不选</button>
                  </div>
                </div>
                <div className="grid max-h-56 grid-cols-2 gap-x-3 overflow-y-auto rounded-md border p-2">
                  {skills.map((skill) => (
                    <label key={skill.name} title={skill.path} className="flex min-w-0 items-center gap-2 rounded px-1 py-1 text-xs text-foreground hover:bg-accent/60">
                      <input type="checkbox" checked={selected.includes(skill.name)} onChange={() => toggle(skill.name)} data-testid={`collection-path-${skill.name}`} />
                      <span className="truncate">{skill.name}</span>
                    </label>
                  ))}
                </div>
                {autoPossible && (
                  <label className="mt-2 flex items-center gap-2 text-xs text-foreground">
                    <input type="checkbox" checked={auto} onChange={(e) => setAuto(e.target.checked)} data-testid="collection-auto-include" />
                    来源以后新增的技能也自动列进来（列进来后仍要在技能页点「添加」才会用上）
                  </label>
                )}
                {keptMissing.length > 0 && (
                  <div data-testid="collection-paths-missing" className="mt-2 rounded-md border border-warning/40 bg-warning/10 p-2 text-xs text-warning">
                    <p>配置里还有这些，但来源里已经找不到（改过名或删了）。取消勾选就从配置里去掉：</p>
                    {keptMissing.map((item) => (
                      <label key={item} className="mt-1 flex items-center gap-2">
                        <input type="checkbox" checked onChange={() => setKeptMissing((prev) => prev.filter((entry) => entry !== item))} data-testid={`collection-path-missing-${item}`} />
                        <span className="font-mono">{item}</span>
                      </label>
                    ))}
                  </div>
                )}
              </>
            ) : (
              <>
                <label className={sectionTitle} htmlFor="collection-paths-input">用哪些技能</label>
                <p data-testid="collection-paths-empty" className="mb-1.5 text-[11px] leading-relaxed text-muted-foreground">
                  {sourceKind === "git" && repoCached === false
                    ? "仓库还没取到本地，列不出里面的技能。空着就用全部；保存后在集合上点「更新」取回，再回来就能逐个勾选。"
                    : initial
                      ? "这个来源里没找到技能（技能文件夹里要有 SKILL.md）。"
                      : "保存后才能列出来源里的技能。空着就用全部；也可以先填技能名或装技能的子目录，每行一个。"}
                </p>
                <textarea
                  id="collection-paths-input"
                  value={manualText}
                  onChange={(e) => setManualText(e.target.value)}
                  rows={2}
                  data-testid="collection-paths-input"
                  placeholder={"空着 = 全部\n或每行一个，例如 skills"}
                  className={inputClass}
                />
              </>
            )}
          </div>

          <details className="group rounded-md border" open={Boolean(seed?.definition.rules_path) || selectedAgents.length > 0}>
            <summary className="flex cursor-pointer list-none items-center gap-1.5 px-3 py-2 text-xs text-muted-foreground hover:text-foreground">
              <ChevronRight className="h-3.5 w-3.5 transition-transform group-open:rotate-90" aria-hidden />
              更多设置
              <span className="text-muted-foreground/70">· 规则位置、只给某些助手用</span>
            </summary>
            <div className="space-y-3 border-t px-3 py-3">
              <div>
                <label className={sectionTitle} htmlFor="collection-rules-path">规则位置：集合里的子目录或某个 .md 文件，空着用 rules/</label>
                <input id="collection-rules-path" value={rulesPath} onChange={(e) => setRulesPath(e.target.value)} data-testid="collection-rules-path-input" className={inputClass} placeholder="rules" />
              </div>
              <div>
                <span className={sectionTitle}>只给这些助手用（都不勾 = 不限）</span>
                <div className="flex flex-wrap gap-4">
                  {agentIds.map((agent) => (
                    <label key={agent} className="flex items-center gap-1.5 text-xs text-foreground">
                      <input
                        type="checkbox"
                        checked={selectedAgents.includes(agent)}
                        onChange={(e) => setSelectedAgents((prev) => (e.target.checked ? [...prev, agent] : prev.filter((item) => item !== agent)))}
                        data-testid={`collection-agent-${agent}`}
                      />
                      {AGENT_LABEL[agent] ?? agent}
                    </label>
                  ))}
                </div>
              </div>
            </div>
          </details>
          {error && <p className="text-xs text-destructive">{error}</p>}
        </div>

        <div className="flex items-center justify-end gap-2 border-t px-5 py-3">
          <span className="mr-auto text-xs text-muted-foreground">保存不会把技能加到任何助手</span>
          <button type="button" onClick={onClose} className="rounded-md border px-4 py-1.5 text-sm text-muted-foreground hover:bg-accent">
            取消
          </button>
          <button type="button" onClick={submit} data-testid="collection-form-submit" className="rounded-md bg-primary px-4 py-1.5 text-sm font-medium text-primary-foreground hover:bg-primary/90">
            保存
          </button>
        </div>
      </div>
    </div>
  );
}
