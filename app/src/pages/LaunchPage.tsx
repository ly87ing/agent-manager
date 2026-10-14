import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { isTauri } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { toast } from "sonner";
import { ConfirmModal } from "../components/ConfirmModal";
import { ActionMenu } from "../components/AccountGrid";
import { Select } from "../components/Select";
import { listTimeZones } from "../lib/launch";
import { engineErrorMessage } from "../lib/engine";
import { useDialogFocus } from "../lib/dialogFocus";
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

export default function LaunchPage({ initialRecipe, onInitialConsumed }: { initialRecipe?: LaunchRecipe | null; onInitialConsumed?: () => void }) {
  useLanguage();
  const [catalog, setCatalog] = useState<LaunchCatalog | null>(null);
  const [recipe, setRecipe] = useState<LaunchRecipe>(() => initialRecipe ?? emptyRecipe());
  const [built, setBuilt] = useState<{ spec: string; entries: LaunchEntry[] } | null>(null);
  const [saved, setSaved] = useState<SavedLaunch[] | null>(null);
  const [selected, setSelected] = useState<SavedLaunch | null>(null);
  const [editing, setEditing] = useState(Boolean(initialRecipe));
  const [readError, setReadError] = useState<string | null>(null);
  const [error, setError] = useState<{ message: string; location: "editor" | "delete" } | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [revision, setRevision] = useState(0);
  const [confirmation, setConfirmation] = useState<SavedLaunch | null>(null);
  const generation = useRef(initialRecipe ? 1 : 0);
  const local = catalog?.platform === recipe.platform;
  const dirty = !selected || JSON.stringify(recipe) !== JSON.stringify(selected.recipe);
  const { dialogRef, onDialogKeyDown } = useDialogFocus<HTMLDivElement>({
    active: editing,
    onEscape: () => { if (!busy) cancelEditing(); },
  });

  useEffect(() => { if (initialRecipe) onInitialConsumed?.(); }, [initialRecipe, onInitialConsumed]);
  useEffect(() => () => { toast.dismiss("launch-error"); }, []);

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

  function clearFeedback() {
    setError(null); setMessage(null); toast.dismiss("launch-error");
  }
  function fail(reason: unknown, title: string, location: "editor" | "delete" | null = "editor") {
    const message = engineErrorMessage(reason);
    setError(location ? { message, location } : null);
    toast.error(title, { id: "launch-error", description: message, duration: 10000, closeButton: true,
      position: editing ? "top-right" : "bottom-right" });
  }
  function change(next: LaunchRecipe) {
    generation.current++;
    setRecipe(next); setBuilt(null); setConfirmation(null); clearFeedback();
  }
  function target(value: string) {
    const previous = targetBase(recipe.target);
    const base = targetBase(value);
    change({ ...recipe, target: value, variants: recipe.variants.map((variant) => ({ ...variant,
      name: variant.name === `${previous}-sg` ? `${base}-sg` : variant.name === `${previous}-la` ? `${base}-la` : variant.name })) });
  }
  async function browse() {
    clearFeedback();
    try {
      const value = await open({ multiple: false, directory: false,
        ...(recipe.platform === "windows" ? { filters: [{ name: "Application", extensions: ["exe"] }] } : {}),
        ...(recipe.platform === "macos" ? { defaultPath: "/Applications" } : {}) });
      if (typeof value === "string") target(value);
    } catch (err) { fail(err, t("选择文件失败")); }
  }
  async function generate() {
    const id = ++generation.current;
    const spec = JSON.stringify(recipe);
    setBusy(true); clearFeedback();
    try {
      const data = await launchRequest<{ entries: LaunchEntry[] }>("build", ["--spec", spec]);
      if (id === generation.current) setBuilt({ spec, entries: data.entries });
    } catch (err) { if (id === generation.current) fail(err, t("预览失败")); }
    finally { setBusy(false); }
  }
  async function copy(value: string) {
    clearFeedback();
    try { await navigator.clipboard.writeText(value); toast.success(t("已复制到剪贴板"), { position: editing ? "top-right" : "bottom-right" }); }
    catch (err) { fail(err, t("复制命令失败"), null); }
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
    clearFeedback();
  }
  async function save() {
    const spec = JSON.stringify(recipe);
    setBusy(true); clearFeedback();
    try {
      const data = await launchRequest<{ item: SavedLaunch }>("save", ["--spec", spec,
        ...(selected ? ["--id", selected.id, "--base", selected.revision] : [])]);
      setSelected(data.item); setBuilt({ spec, entries: data.item.entries });
      setSaved((items) => [...(items ?? []).filter((item) => item.id !== data.item.id), data.item]);
      setMessage(t("配置已保存到 Agent Manager。"));
    } catch (err) { fail(err, t("保存配置失败")); }
    finally { setBusy(false); }
  }
  async function run(spec: LaunchRecipe, entry: LaunchEntry) {
    setBusy(true); clearFeedback();
    try {
      await launchRequest("run", ["--spec", JSON.stringify(spec), "--name", entry.name, "--confirm"]);
      setMessage(t("已发送启动请求。请在应用中确认时间和环境。"));
    } catch (err) { fail(err, t("启动失败"), null); }
    finally { setBusy(false); }
  }
  async function confirm() {
    if (!confirmation) return;
    setBusy(true); clearFeedback();
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
    } catch (err) { fail(err, t("删除配置失败"), "delete"); }
    finally { setBusy(false); }
  }

  return <div className="space-y-4" data-testid="launch-page">
    <div className="flex flex-wrap items-start justify-between gap-3">
      <div className="min-w-0 flex-1">
        <h1 className="text-lg font-semibold">{t("启动配置")}</h1>
        <p className="mt-1 text-xs text-muted-foreground">{t("保存图形应用或终端命令、环境变量、参数和别名。图形应用可直接启动，终端命令可复制后运行。")}</p>
      </div>
      <button type="button" className={`${button} shrink-0 bg-primary text-primary-foreground hover:bg-primary/90`}
        disabled={busy} onClick={startNew}>{t("新增配置")}</button>
    </div>
    <section className="space-y-2" aria-label={t("已保存配置")}>
      <div className="flex flex-wrap items-center justify-between gap-2"><h2 className="text-sm font-medium">{t("已保存配置")}</h2>
        <button type="button" className={button} disabled={busy} onClick={() => setRevision((value) => value + 1)}>{t("重新读取")}</button></div>
      {readError && <p role="alert" className="break-words text-xs text-destructive">{readError}</p>}
      {saved?.length === 0 && <p className="text-xs text-muted-foreground">{t("尚未保存配置。点击“新增配置”创建常用应用或命令。")}</p>}
      {Boolean(saved?.length) && <div className="rounded-md border">
        <table className="w-full table-fixed text-left text-xs">
          <colgroup><col /><col className="w-28" /><col className="w-40" /><col className="w-44" /></colgroup>
          <thead className="border-b bg-muted/40 text-muted-foreground"><tr>
            <th className="px-3 py-2 font-medium">{t("程序")}</th>
            <th className="px-3 py-2 font-medium">{t("别名")}</th>
            <th className="px-3 py-2 font-medium">{t("时区")}</th>
            <th className="px-3 py-2 font-medium">{t("操作")}</th>
          </tr></thead>
          {saved?.flatMap((item) => item.entries.map((entry) => {
            const location = `saved-${item.id}-${entry.name}`;
            const runnable = catalog?.platform === item.recipe.platform && item.recipe.kind === "app";
            return <tbody key={location} data-launch-entry={location} className="border-b last:border-b-0">
              <tr>
                <td className="px-3 py-2">
                  <button type="button" className="w-full break-all text-left font-medium hover:underline disabled:opacity-50"
                    aria-label={t("编辑配置 {0}", entry.name)} disabled={busy} onClick={() => openSaved(item)}>{item.recipe.target}</button>
                  {catalog && catalog.platform !== item.recipe.platform && <span className="ml-2 text-muted-foreground">{item.recipe.platform}</span>}
                </td>
                <td className="break-all px-3 py-2 font-mono">{entry.name}</td>
                <td className="break-words px-3 py-2 text-muted-foreground">{entry.timezone || t("系统默认时区")}</td>
                <td className="px-3 py-2"><div className="flex items-center gap-2">
                  <button type="button" className={`${button} min-w-0 flex-1`} disabled={busy}
                    data-testid={runnable ? `run-saved-${item.id}-${entry.name}` : undefined}
                    onClick={() => void (runnable ? run(item.recipe, entry) : copy(entry.command))}>{runnable ? t("启动") : t("复制命令")}</button>
                  <ActionMenu label={t("配置操作 {0}", entry.name)} items={[
                    { label: t("编辑"), testId: `edit-launch-${item.id}-${entry.name}`, disabled: busy, onSelect: () => openSaved(item) },
                    { label: t("复制命令"), testId: `copy-launch-${item.id}-${entry.name}`, disabled: busy, onSelect: () => void copy(entry.command) },
                    { label: item.recipe.platform === "windows" ? t("复制函数") : t("复制别名"), testId: `copy-definition-${item.id}-${entry.name}`, disabled: busy,
                      onSelect: () => void copy(entry.definition) },
                    { label: t("删除"), testId: `delete-launch-${item.id}-${entry.name}`, danger: true, disabled: busy,
                      onSelect: () => { clearFeedback(); setConfirmation(item); } },
                  ]} />
                </div></td>
              </tr>
            </tbody>;
          }))}
        </table>
      </div>}
    </section>
    {editing && createPortal(<div className="fixed inset-0 z-50 flex items-center justify-center bg-slate-900/40 p-4">
      <div ref={dialogRef} role="dialog" aria-modal="true" aria-label={selected ? t("编辑配置") : t("新建配置")}
        tabIndex={-1} onKeyDown={onDialogKeyDown} className="flex max-h-[calc(100vh-2rem)] w-full max-w-3xl flex-col rounded-xl border bg-card shadow-xl">
      <div className="shrink-0 border-b px-4 py-3">
        <h2 className="text-sm font-medium">{selected ? t("编辑配置") : t("新建配置")}</h2>
        <p className="mt-1 text-xs text-muted-foreground">{t("预览命令不会保存；保存到 Agent Manager 仅保存常用配置，不写入 Shell 配置或修改系统环境。")}</p>
      </div>
      <div className="min-h-0 flex-1 space-y-3 overflow-y-auto overscroll-contain p-4" data-testid="launch-editor-body">
      <fieldset disabled={busy} className="min-w-0 space-y-3 disabled:opacity-70">
        <div className="grid gap-3 sm:grid-cols-2 md:grid-cols-[1fr_1fr_2fr]">
          <div><p className="mb-1 text-xs">{t("目标系统")}</p><Select label={t("目标系统")} value={recipe.platform}
            options={[{ value: "macos", label: "macOS" }, { value: "linux", label: "Linux" }, { value: "windows", label: "Windows" }]}
            onChange={(value) => change({ ...recipe, platform: value as LaunchRecipe["platform"], shell: value === "windows" ? "powershell" : value === "linux" ? "bash" : "zsh" })} /></div>
          <div><p className="mb-1 text-xs">Shell</p><Select label="Shell" value={recipe.shell}
            options={(recipe.platform === "windows" ? ["powershell"] : ["zsh", "bash"]).map((value) => ({ value, label: value === "powershell" ? "PowerShell" : value }))}
            onChange={(value) => change({ ...recipe, shell: value as LaunchRecipe["shell"] })} /></div>
          <div className="min-w-0 sm:col-span-2 md:col-span-1"><p className="mb-1 text-xs">{t("程序类型")}</p><Select label={t("程序类型")} value={recipe.kind}
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
    {recipe.platform === "windows" && <p className="text-xs text-muted-foreground">{t("Windows 的 TZ 取值由应用决定，部分应用需要 POSIX 格式，部分会忽略它。不会修改系统时区、PATH 或脚本执行设置。")}</p>}
    {built && <section className="space-y-3" aria-label={t("生成结果")}>
      {built.entries.map((entry) => <div key={entry.name} data-launch-entry={`preview-${entry.name}`} className="min-w-0 space-y-2 rounded-lg border bg-card p-3">
        <div className="flex flex-wrap items-center justify-between gap-2"><span className="font-mono text-sm">{entry.name}</span>
          <span className="text-xs text-muted-foreground">{entry.timezone || t("系统默认时区")}</span></div>
        <pre className="whitespace-pre-wrap break-all rounded bg-muted p-2 text-xs" data-testid={`launch-command-${entry.name}`}>{entry.command}</pre>
        <div className="flex flex-wrap gap-2">
          <button type="button" className={button} onClick={() => void copy(entry.command)}>{t("复制命令")}</button>
          <button type="button" className={button} onClick={() => void copy(entry.definition)}>{recipe.platform === "windows" ? t("复制函数") : t("复制别名")}</button>
          {local && recipe.kind === "app" && <button type="button" disabled={busy} className={button}
            onClick={() => void run(recipe, entry)}>{t("启动")}</button>}
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
      </div>
      <div data-testid="launch-editor-actions" className="shrink-0 space-y-2 border-t px-4 py-3">
        {error?.location === "editor" && <p role="alert" className="max-h-24 overflow-y-auto whitespace-pre-wrap break-words text-xs text-destructive">{error.message}</p>}
        {message && <p role="status" className="text-xs text-success">{message}</p>}
        <div className="flex flex-wrap items-center justify-end gap-2">
          {recipe.target.trim() && <span className="mr-auto text-xs text-muted-foreground">{dirty ? t("未保存") : t("已保存")}</span>}
          <button type="button" className={button} disabled={busy} onClick={cancelEditing}>{t("取消")}</button>
          <button type="button" className={button} disabled={busy || !recipe.target.trim()} onClick={() => void generate()}>{t("预览命令")}</button>
          <button type="button" className={`${button} bg-primary text-primary-foreground hover:bg-primary/90`} disabled={busy || !saved || !recipe.target.trim() || !dirty} onClick={() => void save()}>{t("保存到 Agent Manager")}</button>
        </div>
      </div>
      </div>
    </div>, document.body)}
    {!editing && message && <p role="status" className="text-xs text-success">{message}</p>}
    {confirmation && <ConfirmModal title={t("删除配置")}
      description={t("仅删除 Agent Manager 中保存的配置。你自行添加的别名不受影响。")}
      risk="write-local" executing={busy}
      confirmLabel={t("删除配置")}
      onConfirm={() => void confirm()} onClose={() => { setConfirmation(null); clearFeedback(); }}>
      <div className="mt-3 space-y-3">
        <pre className="whitespace-pre-wrap break-all rounded bg-muted p-2 text-xs">{confirmation.recipe.variants.map((variant) => variant.name).join(", ")}</pre>
        {error?.location === "delete" && <p role="alert" className="whitespace-pre-wrap break-words text-xs text-destructive">{error.message}</p>}
      </div>
    </ConfirmModal>}
  </div>;
}
