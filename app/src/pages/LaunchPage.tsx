import { useEffect, useRef, useState } from "react";
import { isTauri } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { toast } from "sonner";
import { ConfirmModal } from "../components/ConfirmModal";
import { ActionMenu } from "../components/AccountGrid";
import { Select } from "../components/Select";
import { listTimeZones } from "../lib/aliasCommand";
import { engineErrorMessage } from "../lib/engine";
import { t, useLanguage } from "../lib/i18n";
import { launchRequest, type LaunchCatalog, type LaunchEntry, type LaunchRecipe, type SavedLaunch } from "../lib/launch";

const button = "rounded-md border px-3 py-1.5 text-xs hover:bg-accent disabled:opacity-50";
const input = "w-full min-w-0 rounded-md border border-input bg-background px-2 py-1.5 text-sm";
const zones = listTimeZones().map((value) => ({ value, label: value }));
const targetBase = (value: string) => value.split(/[\\/]/).pop()?.replace(/\.(app|exe)$/i, "").toLowerCase()
  .replace(/[^a-z0-9_-]+/g, "-").replace(/^-+|-+$/g, "") || "app";
const emptyRecipe = (platform: LaunchRecipe["platform"] = "macos", shell: LaunchRecipe["shell"] = "zsh"): LaunchRecipe => ({
  platform, shell, kind: "app", target: "", args: [], env: [],
  variants: [{ name: "app-sg", timezone: "Asia/Singapore" }, { name: "app-la", timezone: "America/Los_Angeles" }],
});

export default function LaunchPage() {
  useLanguage();
  const [catalog, setCatalog] = useState<LaunchCatalog | null>(null);
  const [recipe, setRecipe] = useState<LaunchRecipe>(() => emptyRecipe());
  const [built, setBuilt] = useState<{ spec: string; entries: LaunchEntry[] } | null>(null);
  const [saved, setSaved] = useState<SavedLaunch[] | null>(null);
  const [selected, setSelected] = useState<SavedLaunch | null>(null);
  const [editing, setEditing] = useState(false);
  const [readError, setReadError] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [revision, setRevision] = useState(0);
  const [confirmation, setConfirmation] = useState<SavedLaunch | null>(null);
  const generation = useRef(0);
  const local = catalog?.platform === recipe.platform;
  const dirty = !selected || JSON.stringify(recipe) !== JSON.stringify(selected.recipe);

  useEffect(() => {
    let cancelled = false;
    void launchRequest<LaunchCatalog>("catalog").then((data) => {
      if (cancelled) return;
      setCatalog(data);
      if (generation.current === 0) setRecipe((old) => ({ ...old, platform: data.platform, shell: data.shell }));
    }).catch((err) => { if (!cancelled) setReadError(engineErrorMessage(err)); });
    return () => { cancelled = true; };
  }, []);

  useEffect(() => {
    let cancelled = false;
    setSaved(null);
    void launchRequest<{ items: SavedLaunch[] }>("list").then((data) => {
      if (!cancelled) { setSaved(data.items); setReadError(null); }
    }).catch((err) => { if (!cancelled) setReadError(engineErrorMessage(err)); });
    return () => { cancelled = true; };
  }, [revision]);

  function change(next: LaunchRecipe) {
    generation.current++;
    setRecipe(next); setBuilt(null); setConfirmation(null); setError(null); setMessage(null);
  }
  function target(value: string) {
    const previous = targetBase(recipe.target);
    const base = targetBase(value);
    change({ ...recipe, target: value, variants: recipe.variants.map((variant) => ({ ...variant,
      name: variant.name === `${previous}-sg` ? `${base}-sg` : variant.name === `${previous}-la` ? `${base}-la` : variant.name })) });
  }
  async function browse() {
    try {
      const value = await open({ multiple: false, directory: false,
        ...(recipe.platform === "windows" ? { filters: [{ name: "Application", extensions: ["exe"] }] } : {}),
        ...(recipe.platform === "macos" ? { defaultPath: "/Applications" } : {}) });
      if (typeof value === "string") target(value);
    } catch (err) { setError(engineErrorMessage(err)); }
  }
  async function generate() {
    const id = ++generation.current;
    const spec = JSON.stringify(recipe);
    setBusy(true); setError(null); setMessage(null);
    try {
      const data = await launchRequest<{ entries: LaunchEntry[] }>("build", ["--spec", spec]);
      if (id === generation.current) setBuilt({ spec, entries: data.entries });
    } catch (err) { if (id === generation.current) setError(engineErrorMessage(err)); }
    finally { setBusy(false); }
  }
  async function copy(value: string) {
    try { await navigator.clipboard.writeText(value); toast.success(t("已复制到剪贴板")); }
    catch (err) { setError(engineErrorMessage(err)); }
  }
  function openSaved(item: SavedLaunch) {
    const normalized = { ...item.recipe, env: item.recipe.env ?? [] };
    change(normalized); setSelected({ ...item, recipe: normalized });
    setBuilt({ spec: JSON.stringify(normalized), entries: item.entries });
    setEditing(true);
  }
  function startNew() {
    change(emptyRecipe(catalog?.platform, catalog?.shell));
    setSelected(null);
    setEditing(true);
  }
  function cancelEditing() {
    setEditing(false);
    setSelected(null);
    setBuilt(null);
    setError(null);
    setMessage(null);
  }
  async function save() {
    const spec = JSON.stringify(recipe);
    setBusy(true); setError(null); setMessage(null);
    try {
      const data = await launchRequest<{ item: SavedLaunch }>("save", ["--spec", spec,
        ...(selected ? ["--id", selected.id, "--base", selected.revision] : [])]);
      setSelected(data.item); setBuilt({ spec, entries: data.item.entries });
      setSaved((items) => [...(items ?? []).filter((item) => item.id !== data.item.id), data.item]);
      setMessage(t("配置已保存到 Agent Manager。"));
    } catch (err) { setError(engineErrorMessage(err)); }
    finally { setBusy(false); }
  }
  async function run(spec: LaunchRecipe, entry: LaunchEntry) {
    setBusy(true); setError(null); setMessage(null);
    try {
      await launchRequest("run", ["--spec", JSON.stringify(spec), "--name", entry.name, "--confirm"]);
      setMessage(t("已发送启动请求。请在应用中确认时间和环境。"));
    } catch (err) { setError(engineErrorMessage(err)); }
    finally { setBusy(false); }
  }
  async function confirm() {
    if (!confirmation) return;
    setBusy(true); setError(null);
    try {
      await launchRequest("delete", ["--id", confirmation.id, "--base", confirmation.revision]);
      setSaved((items) => (items ?? []).filter((old) => old.id !== confirmation.id));
      if (selected?.id === confirmation.id) {
        setSelected(null);
        setEditing(false);
        setBuilt(null);
      }
      setMessage(t("已删除保存的配置。"));
      setConfirmation(null);
    } catch (err) { setError(engineErrorMessage(err)); }
    finally { setBusy(false); }
  }

  return <div className="space-y-4" data-testid="launch-page">
    <div className="flex flex-wrap items-start justify-between gap-3">
      <div>
        <h1 className="text-lg font-semibold">{t("启动配置")}</h1>
        <p className="mt-1 text-xs text-muted-foreground">{t("保存图形应用或终端命令、环境变量、参数和别名。图形应用可直接启动，终端命令可复制后运行。")}</p>
      </div>
      {!editing && <button type="button" className={`${button} bg-primary text-primary-foreground hover:bg-primary/90`}
        disabled={busy} onClick={startNew}>{t("新增配置")}</button>}
    </div>
    <section className="space-y-2" aria-label={t("已保存配置")}>
      <div className="flex flex-wrap items-center justify-between gap-2"><h2 className="text-sm font-medium">{t("已保存配置")}</h2>
        <button type="button" className={button} disabled={busy} onClick={() => setRevision((value) => value + 1)}>{t("重新读取")}</button></div>
      {saved?.length === 0 && <p className="text-xs text-muted-foreground">{t("尚未保存配置。点击“新增配置”创建常用应用或命令。")}</p>}
      {saved?.map((item) => <div key={item.id} className="min-w-0 space-y-3 rounded-md border p-3">
        <div className="flex min-w-0 items-start gap-2">
          <div className="min-w-0 flex-1 space-y-1">
            <p className="break-all text-xs font-medium">{item.recipe.target}</p>
            <p className="text-xs text-muted-foreground">{item.recipe.kind === "app" ? t("图形应用") : t("命令行程序")} · {item.recipe.platform} · {item.recipe.shell}</p>
          </div>
          <button type="button" className={`${button} shrink-0`} disabled={busy} onClick={() => openSaved(item)}>{t("打开配置")}</button>
          <ActionMenu label={t("配置操作")} items={[{ label: t("删除配置"), testId: `delete-launch-${item.id}`, danger: true, disabled: busy,
            onSelect: () => { setError(null); setConfirmation(item); } }]} />
        </div>
        <div className="space-y-2">
          {item.entries.map((entry) => <div key={entry.name} className="flex min-w-0 flex-wrap items-center gap-2 rounded bg-muted/40 p-2">
            <div className="min-w-0 flex-1">
              <p className="break-all font-mono text-xs">{entry.name}</p>
              <p className="break-all text-xs text-muted-foreground">{entry.timezone || t("系统默认时区")}</p>
            </div>
            <button type="button" className={button} disabled={busy} onClick={() => void copy(entry.command)}>{t("复制运行命令")}</button>
            <button type="button" className={button} disabled={busy} onClick={() => void copy(entry.definition)}>{item.recipe.platform === "windows" ? t("复制完整函数定义") : t("复制完整别名定义")}</button>
            {catalog?.platform === item.recipe.platform && item.recipe.kind === "app" && <button type="button" className={button} disabled={busy}
              data-testid={`run-saved-${item.id}-${entry.name}`} onClick={() => void run(item.recipe, entry)}>{t("启动一次")}</button>}
          </div>)}
        </div>
      </div>)}
    </section>
    {editing && <section className="space-y-3 rounded-lg border bg-card p-4" aria-label={t("构建启动命令")}>
      <div data-testid="launch-editor-actions" className="sticky top-0 z-10 -mx-4 -mt-4 flex flex-wrap items-start justify-between gap-2 rounded-t-lg border-b bg-card px-4 py-3">
        <div>
          <h2 className="text-sm font-medium">{selected ? t("编辑配置") : t("新建配置")}</h2>
          <p className="mt-1 text-xs text-muted-foreground">{t("预览命令不会保存；保存到 Agent Manager 仅保存常用配置，不写入 Shell 配置或修改系统环境。")}</p>
        </div>
        <div className="flex flex-wrap items-center justify-end gap-2">
          {recipe.target.trim() && <span className="text-xs text-muted-foreground">{dirty ? t("未保存") : t("已保存")}</span>}
          {selected && <button type="button" className={button} disabled={busy} onClick={startNew}>{t("新增配置")}</button>}
          <button type="button" className={button} disabled={busy} onClick={cancelEditing}>{t("取消")}</button>
          <button type="button" className={button} disabled={!recipe.target.trim()} onClick={() => void generate()}>{t("预览命令")}</button>
          <button type="button" className={`${button} bg-primary text-primary-foreground hover:bg-primary/90`} disabled={!saved || !recipe.target.trim() || !dirty} onClick={() => void save()}>{t("保存到 Agent Manager")}</button>
        </div>
      </div>
      <fieldset disabled={busy} className="min-w-0 space-y-3 disabled:opacity-70">
        <div className="grid gap-3 sm:grid-cols-3">
          <div><p className="mb-1 text-xs">{t("目标系统")}</p><Select label={t("目标系统")} value={recipe.platform}
            options={[{ value: "macos", label: "macOS" }, { value: "linux", label: "Linux" }, { value: "windows", label: "Windows" }]}
            onChange={(value) => change({ ...recipe, platform: value as LaunchRecipe["platform"], shell: value === "windows" ? "powershell" : value === "linux" ? "bash" : "zsh" })} /></div>
          <div><p className="mb-1 text-xs">Shell</p><Select label="Shell" value={recipe.shell}
            options={(recipe.platform === "windows" ? ["powershell"] : ["zsh", "bash"]).map((value) => ({ value, label: value === "powershell" ? "PowerShell" : value }))}
            onChange={(value) => change({ ...recipe, shell: value as LaunchRecipe["shell"] })} /></div>
          <div><p className="mb-1 text-xs">{t("程序类型")}</p><Select label={t("程序类型")} value={recipe.kind}
            options={[{ value: "app", label: t("图形应用（可直接启动）") }, { value: "command", label: t("命令行程序（在终端运行）") }]}
            onChange={(value) => change({ ...recipe, kind: value as LaunchRecipe["kind"] })} /></div>
        </div>
        {local && recipe.kind === "app" && Boolean(catalog?.apps.length) && <Select label={t("本机应用")}
          value={catalog!.apps.some((app) => app.path === recipe.target) ? recipe.target : ""}
          options={[{ value: "", label: t("选择本机应用或输入路径") }, ...catalog!.apps.map((app) => ({ value: app.path, label: app.name }))]}
          onChange={(value) => target(value)} />}
        <div className="text-xs"><label htmlFor="launch-target">{t("程序路径或命令")}</label><div className="mt-1 flex gap-2">
          <input id="launch-target" aria-label={t("程序路径或命令")} className={`${input} font-mono`} value={recipe.target} onChange={(event) => target(event.target.value)}
            placeholder={recipe.platform === "macos" && recipe.kind === "app" ? "/Applications/Example.app" : recipe.platform === "windows" ? "C:\\Apps\\Example.exe" : "/path/to/program"} />
          {local && isTauri() && <button className={button} type="button" onClick={() => void browse()}>{t("选择文件")}</button>}
        </div></div>
        <details>
          <summary className="cursor-pointer text-xs">{t("环境变量（{0}）", String(recipe.env.length))}</summary>
          <div className="mt-2 space-y-2">
            {recipe.env.map((variable, index) => <div key={index} className="grid gap-2 sm:grid-cols-[minmax(0,1fr)_minmax(0,2fr)_auto]">
              <input className={`${input} font-mono`} aria-label={t("环境变量名称 {0}", String(index + 1))} value={variable.name}
                placeholder="LANG" onChange={(event) => change({ ...recipe, env: recipe.env.map((old, i) => i === index ? { ...old, name: event.target.value } : old) })} />
              <input className={`${input} font-mono`} aria-label={t("环境变量取值 {0}", String(index + 1))} value={variable.value}
                placeholder="en_US.UTF-8" onChange={(event) => change({ ...recipe, env: recipe.env.map((old, i) => i === index ? { ...old, value: event.target.value } : old) })} />
              <button type="button" className={button} onClick={() => change({ ...recipe, env: recipe.env.filter((_, i) => i !== index) })}>{t("移除")}</button>
            </div>)}
            <button type="button" className={button} disabled={recipe.env.length >= 64}
              onClick={() => change({ ...recipe, env: [...recipe.env, { name: "", value: "" }] })}>{t("添加环境变量")}</button>
            <p className="text-xs text-muted-foreground">{t("例如 LANG=en_US.UTF-8。Chrome 的 --lang=en-US 和 Codex 的 --dangerously-bypass-approvals-and-sandbox 属于启动参数。")}</p>
          </div>
        </details>
        <details>
          <summary className="cursor-pointer text-xs">{t("启动参数（{0}）", String(recipe.args.length))}</summary>
          <div className="mt-2 space-y-2">
            {recipe.args.map((arg, index) => <div key={index} className="flex gap-2">
              <input className={`${input} font-mono`} aria-label={t("启动参数 {0}", String(index + 1))} value={arg}
                onChange={(event) => change({ ...recipe, args: recipe.args.map((old, i) => i === index ? event.target.value : old) })} />
              <button type="button" className={button} onClick={() => change({ ...recipe, args: recipe.args.filter((_, i) => i !== index) })}>{t("移除")}</button>
            </div>)}
            <button type="button" className={button} onClick={() => change({ ...recipe, args: [...recipe.args, ""] })}>{t("添加参数")}</button>
            <p className="text-xs text-muted-foreground">{t("每项是一个原样传入的参数，无需添加引号。")}</p>
          </div>
        </details>
        <div className="space-y-2">
          {recipe.variants.map((variant, index) => <div key={index} className="grid min-w-0 items-end gap-2 rounded-md bg-muted/40 p-2 sm:grid-cols-[minmax(0,1fr)_minmax(0,1.5fr)_auto]">
            <label className="min-w-0 text-xs">{t("别名 {0}", String(index + 1))}<input className={`${input} mt-1 font-mono`} aria-label={t("别名 {0}", String(index + 1))}
              value={variant.name} onChange={(event) => change({ ...recipe, variants: recipe.variants.map((old, i) => i === index ? { ...old, name: event.target.value } : old) })} /></label>
            <div className="min-w-0 space-y-1">
              <Select label={t("选择时区 {0}", String(index + 1))} value={variant.timezone} options={[{ value: "", label: t("不设置时区") }, ...zones]}
                onChange={(value) => change({ ...recipe, variants: recipe.variants.map((old, i) => i === index ? { ...old, timezone: value } : old) })} />
              <input className={`${input} font-mono`} aria-label={t("自定义时区 {0}", String(index + 1))} value={variant.timezone}
                onChange={(event) => change({ ...recipe, variants: recipe.variants.map((old, i) => i === index ? { ...old, timezone: event.target.value } : old) })} />
            </div>
            <button className={button} type="button" disabled={recipe.variants.length === 1} onClick={() => change({ ...recipe, variants: recipe.variants.filter((_, i) => i !== index) })}>{t("移除")}</button>
          </div>)}
          <button type="button" className={button} disabled={recipe.variants.length >= 32} onClick={() => change({ ...recipe, variants: [...recipe.variants, { name: "", timezone: "" }] })}>{t("添加别名")}</button>
        </div>
      </fieldset>
    </section>}
    {editing && recipe.platform === "windows" && <p className="text-xs text-muted-foreground">{t("Windows 的 TZ 取值由应用决定，部分应用需要 POSIX 格式，部分会忽略它。不会修改系统时区、PATH 或脚本执行设置。")}</p>}
    {editing && built && <section className="space-y-3" aria-label={t("生成结果")}>
      {built.entries.map((entry) => <div key={entry.name} className="min-w-0 space-y-2 rounded-lg border bg-card p-3">
        <div className="flex flex-wrap items-center justify-between gap-2"><span className="font-mono text-sm">{entry.name}</span>
          <span className="text-xs text-muted-foreground">{entry.timezone || t("系统默认时区")}</span></div>
        <pre className="whitespace-pre-wrap break-all rounded bg-muted p-2 text-xs" data-testid={`launch-command-${entry.name}`}>{entry.command}</pre>
        <div className="flex flex-wrap gap-2">
          <button type="button" className={button} onClick={() => void copy(entry.command)}>{t("复制运行命令")}</button>
          <button type="button" className={button} onClick={() => void copy(entry.definition)}>{recipe.platform === "windows" ? t("复制完整函数定义") : t("复制完整别名定义")}</button>
          {local && recipe.kind === "app" && <button type="button" disabled={busy} className={button}
            onClick={() => void run(recipe, entry)}>{t("启动一次")}</button>}
        </div>
      </div>)}
      <div className="flex flex-wrap items-center gap-2">
        <button type="button" className={button} onClick={() => void copy(built.entries.map((entry) => entry.definition).join("\n"))}>{t("复制全部定义")}</button>
      </div>
      <p className="text-xs text-muted-foreground">{recipe.shell === "powershell" ? t("将完整函数定义粘贴到 PowerShell 的 $PROFILE，或当前会话中。")
        : recipe.shell === "zsh" ? t("将完整别名定义粘贴到 .zshrc，或当前终端会话中。") : t("将完整别名定义粘贴到 .bashrc，或当前终端会话中。")}</p>
      <p className="text-xs text-muted-foreground">{recipe.kind === "command" ? t("命令行程序请在终端运行复制的命令，保留正常交互。") : t("若应用已经运行，请先正常退出，再使用新命令启动。")}</p>
      {!local && <p className="text-xs text-muted-foreground">{t("其他系统的配置也可以保存和复制；仅本机系统可以启动。")}</p>}
    </section>}
    {message && <p role="status" className="text-xs text-success">{message}</p>}
    {error && !confirmation && <p role="alert" className="whitespace-pre-wrap break-words text-xs text-destructive">{error}</p>}
    {readError && <p role="alert" className="break-words text-xs text-destructive">{readError}</p>}
    {confirmation && <ConfirmModal title={t("删除配置")}
      description={t("仅删除 Agent Manager 中保存的配置。你自行添加的别名不受影响。")}
      risk="write-local" executing={busy}
      confirmLabel={t("删除配置")}
      onConfirm={() => void confirm()} onClose={() => { setConfirmation(null); setError(null); }}>
      <div className="mt-3 space-y-3">
        <pre className="whitespace-pre-wrap break-all rounded bg-muted p-2 text-xs">{confirmation.recipe.variants.map((variant) => variant.name).join(", ")}</pre>
        {error && <p role="alert" className="whitespace-pre-wrap break-words text-xs text-destructive">{error}</p>}
      </div>
    </ConfirmModal>}
  </div>;
}
