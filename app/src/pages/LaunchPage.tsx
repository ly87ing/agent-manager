import { useEffect, useRef, useState } from "react";
import { isTauri } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { toast } from "sonner";
import { ConfirmModal } from "../components/ConfirmModal";
import { FileDiff } from "../components/FileDiff";
import { Select } from "../components/Select";
import { listTimeZones } from "../lib/aliasCommand";
import { engineErrorMessage } from "../lib/engine";
import { t, useLanguage } from "../lib/i18n";
import { launchRequest, type LaunchCatalog, type LaunchEntry, type LaunchPreview, type LaunchRecipe, type LaunchStatus } from "../lib/launch";

const button = "rounded-md border px-3 py-1.5 text-xs hover:bg-accent disabled:opacity-50";
const input = "w-full min-w-0 rounded-md border border-input bg-background px-2 py-1.5 text-sm";
const zones = listTimeZones().map((value) => ({ value, label: value }));
type Confirmation = { kind: "apply"; args: string[]; preview: LaunchPreview } | { kind: "run"; args: string[]; entry: LaunchEntry };

export default function LaunchPage() {
  useLanguage();
  const [catalog, setCatalog] = useState<LaunchCatalog | null>(null);
  const [recipe, setRecipe] = useState<LaunchRecipe>({ platform: "macos", shell: "zsh", kind: "app", target: "", args: [],
    variants: [{ name: "app-sg", timezone: "Asia/Singapore" }, { name: "app-la", timezone: "America/Los_Angeles" }] });
  const [built, setBuilt] = useState<{ spec: string; entries: LaunchEntry[] } | null>(null);
  const [status, setStatus] = useState<LaunchStatus | null>(null);
  const [readError, setReadError] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [revision, setRevision] = useState(0);
  const [confirmation, setConfirmation] = useState<Confirmation | null>(null);
  const generation = useRef(0);
  const local = catalog?.platform === recipe.platform;

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
    setStatus(null);
    if (!local) return;
    setReadError(null);
    void launchRequest<LaunchStatus>("status", ["--shell", recipe.shell]).then((data) => {
      if (!cancelled) setStatus(data);
    }).catch((err) => { if (!cancelled) setReadError(engineErrorMessage(err)); });
    return () => { cancelled = true; };
  }, [local, recipe.shell, revision]);

  function change(next: LaunchRecipe) {
    generation.current++;
    setRecipe(next); setBuilt(null); setConfirmation(null); setError(null); setMessage(null);
  }
  function target(value: string) {
    const base = value.split(/[\\/]/).pop()?.replace(/\.(app|exe)$/i, "").toLowerCase().replace(/[^a-z0-9_-]+/g, "-").replace(/^-+|-+$/g, "") || "app";
    change({ ...recipe, target: value, variants: recipe.variants.map((variant) => ({ ...variant,
      name: variant.name === "app-sg" ? `${base}-sg` : variant.name === "app-la" ? `${base}-la` : variant.name })) });
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
  async function preview(remove?: string) {
    if (!status || (!remove && !built)) return;
    const args = ["--shell", recipe.shell, "--base", status.base,
      ...(remove ? ["--remove", remove] : ["--spec", built!.spec])];
    setBusy(true); setError(null); setMessage(null);
    try {
      const data = await launchRequest<LaunchPreview>("preview", args);
      setConfirmation({ kind: "apply", args, preview: data });
    } catch (err) { setError(engineErrorMessage(err)); }
    finally { setBusy(false); }
  }
  async function confirm() {
    if (!confirmation) return;
    setBusy(true); setError(null);
    try {
      if (confirmation.kind === "apply") {
        const data = await launchRequest<{ status: LaunchStatus }>("apply", [...confirmation.args, "--confirm"]);
        setStatus(data.status); setMessage(t("快捷命令已应用。新开终端后生效；应用显示的时间仍需验证。"));
      } else {
        await launchRequest("run", [...confirmation.args, "--confirm"]);
        setMessage(t("已发送启动请求。请在应用中确认时间。"));
      }
      setConfirmation(null);
    } catch (err) { setError(engineErrorMessage(err)); }
    finally { setBusy(false); }
  }

  return <div className="space-y-4" data-testid="launch-page">
    <div>
      <h1 className="text-lg font-semibold">{t("启动配置")}</h1>
      <p className="mt-1 text-xs text-muted-foreground">{t("选择程序、时区和别名。复制不会修改本机；应用前会展示文件改动并请你确认。")}</p>
    </div>
    <section className="space-y-3 rounded-lg border bg-card p-4" aria-label={t("构建启动命令")}>
      <fieldset disabled={busy} className="min-w-0 space-y-3 disabled:opacity-70">
        <div className="grid gap-3 sm:grid-cols-3">
          <div><p className="mb-1 text-xs">{t("目标系统")}</p><Select label={t("目标系统")} value={recipe.platform}
            options={[{ value: "macos", label: "macOS" }, { value: "linux", label: "Linux" }, { value: "windows", label: "Windows" }]}
            onChange={(value) => change({ ...recipe, platform: value as LaunchRecipe["platform"], shell: value === "windows" ? "powershell" : value === "linux" ? "bash" : "zsh" })} /></div>
          <div><p className="mb-1 text-xs">Shell</p><Select label="Shell" value={recipe.shell}
            options={(recipe.platform === "windows" ? ["powershell"] : ["zsh", "bash"]).map((value) => ({ value, label: value === "powershell" ? "PowerShell" : value }))}
            onChange={(value) => change({ ...recipe, shell: value as LaunchRecipe["shell"] })} /></div>
          <div><p className="mb-1 text-xs">{t("程序类型")}</p><Select label={t("程序类型")} value={recipe.kind}
            options={[{ value: "app", label: t("图形应用") }, { value: "command", label: t("命令行程序") }]}
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
              <Select label={t("选择时区 {0}", String(index + 1))} value={variant.timezone} options={zones}
                onChange={(value) => change({ ...recipe, variants: recipe.variants.map((old, i) => i === index ? { ...old, timezone: value } : old) })} />
              <input className={`${input} font-mono`} aria-label={t("自定义时区 {0}", String(index + 1))} value={variant.timezone}
                onChange={(event) => change({ ...recipe, variants: recipe.variants.map((old, i) => i === index ? { ...old, timezone: event.target.value } : old) })} />
            </div>
            <button className={button} type="button" disabled={recipe.variants.length === 1} onClick={() => change({ ...recipe, variants: recipe.variants.filter((_, i) => i !== index) })}>{t("移除")}</button>
          </div>)}
          <div className="flex flex-wrap justify-between gap-2">
            <button type="button" className={button} disabled={recipe.variants.length >= 32} onClick={() => change({ ...recipe, variants: [...recipe.variants, { name: "", timezone: "UTC" }] })}>{t("添加时区别名")}</button>
            <button type="button" className={`${button} bg-primary text-primary-foreground hover:bg-primary/90`} disabled={!recipe.target.trim()} onClick={() => void generate()}>{t("生成命令")}</button>
          </div>
        </div>
      </fieldset>
    </section>
    {recipe.platform === "windows" && <p className="text-xs text-muted-foreground">{t("Windows 的 TZ 取值由应用决定，部分应用需要 POSIX 格式，部分会忽略它。不会修改系统时区、PATH 或脚本执行设置。")}</p>}
    {built && <section className="space-y-3" aria-label={t("生成结果")}>
      {built.entries.map((entry) => <div key={entry.name} className="min-w-0 space-y-2 rounded-lg border bg-card p-3">
        <div className="flex flex-wrap items-center justify-between gap-2"><span className="font-mono text-sm">{entry.name}</span>
          <span className="text-xs text-muted-foreground">{entry.timezone}</span></div>
        <pre className="whitespace-pre-wrap break-all rounded bg-muted p-2 text-xs" data-testid={`launch-command-${entry.name}`}>{entry.command}</pre>
        <div className="flex flex-wrap gap-2">
          <button type="button" className={button} onClick={() => void copy(entry.command)}>{t("复制完整命令")}</button>
          <button type="button" className={button} onClick={() => void copy(entry.definition)}>{t("复制快捷命令定义")}</button>
          {local && recipe.kind === "app" && <button type="button" disabled={busy} className={button} onClick={() => {
            setError(null); setConfirmation({ kind: "run", args: ["--spec", built.spec, "--name", entry.name], entry });
          }}>{t("启动一次")}</button>}
        </div>
      </div>)}
      <div className="flex flex-wrap items-center gap-2">
        <button type="button" className={button} onClick={() => void copy(built.entries.map((entry) => entry.definition).join("\n"))}>{t("复制全部定义")}</button>
        <button type="button" className={`${button} bg-primary text-primary-foreground hover:bg-primary/90`} disabled={!local || !status || busy} onClick={() => void preview()}>{t("应用到本机")}</button>
      </div>
      <p className="text-xs text-muted-foreground">{recipe.kind === "command" ? t("命令行程序请在终端运行复制的命令，保留正常交互。") : t("若应用已经运行，请先正常退出，再使用新命令启动。")}</p>
      {!local && <p className="text-xs text-muted-foreground">{t("其他系统的命令可以复制；仅本机系统可以应用或启动。")}</p>}
    </section>}
    {message && <p role="status" className="text-xs text-success">{message}</p>}
    {error && !confirmation && <p role="alert" className="whitespace-pre-wrap break-words text-xs text-destructive">{error}</p>}
    {local && <section className="space-y-2 border-t pt-3" aria-label={t("本机快捷命令")}>
      <div className="flex flex-wrap items-center justify-between gap-2"><h2 className="text-sm font-medium">{t("本机快捷命令")}</h2>
        <button type="button" className={button} disabled={busy} onClick={() => { setConfirmation(null); setRevision((value) => value + 1); }}>{t("重新读取")}</button></div>
      {status && <><p className="break-all font-mono text-xs text-muted-foreground">{status.path}</p>
        {status.entries.length === 0 && <p className="text-xs text-muted-foreground">{t("尚未应用快捷命令。")}</p>}
        {status.entries.map((entry) => <div key={entry.name} className="min-w-0 space-y-1 border-b py-2">
          <div className="flex flex-wrap items-center justify-between gap-2"><span className="font-mono text-xs">{entry.name}</span><div className="flex gap-2">
            <button type="button" className={button} onClick={() => void copy(entry.command ?? entry.definition)}>{t("复制")}</button>
            <button type="button" className={button} disabled={busy} onClick={() => void preview(entry.name)}>{t("移除")}</button>
          </div></div>
          <pre className="whitespace-pre-wrap break-all text-xs text-muted-foreground">{entry.command ?? entry.definition}</pre>
          {recipe.shell === "powershell" && <div className="flex flex-wrap items-center gap-2">
            <code className="min-w-0 flex-1 break-all text-xs">{entry.path}</code>
            <button type="button" className={button} onClick={() => void copy(`& '${entry.path.replaceAll("'", "''")}'`)}>{t("复制脚本调用")}</button>
          </div>}
        </div>)}
        <p className="text-xs text-muted-foreground">{recipe.shell === "powershell" ? t("在 PowerShell 中使用显示的脚本完整路径。") : t("新开交互终端即可使用。zsh 仅维护 .zshrc，不写 .zprofile。")}</p>
      </>}
    </section>}
    {readError && <p role="alert" className="break-words text-xs text-destructive">{readError}</p>}
    {confirmation && <ConfirmModal title={confirmation.kind === "apply" ? t("确认应用快捷命令") : t("确认启动应用")}
      description={confirmation.kind === "apply" ? t("以下文件将按预览改动。若文件在预览后发生变化，会停止写入。") : t("将使用下面的命令启动应用一次，不保存快捷命令。应用显示的时间需单独确认。")}
      risk={confirmation.kind === "apply" ? "write-local" : "exec-local"} executing={busy}
      confirmLabel={confirmation.kind === "apply" ? t("确认应用") : t("确认启动")}
      onConfirm={() => void confirm()} onClose={() => { setConfirmation(null); setError(null); }}>
      <div className="mt-3 space-y-3">
        {confirmation.kind === "apply" ? confirmation.preview.files.map((file) => <div key={file.path}>
          <p className="mb-1 break-all font-mono text-xs">{file.path}</p>
          <FileDiff original={file.original} content={file.content} />
        </div>) : <pre className="whitespace-pre-wrap break-all rounded bg-muted p-2 text-xs">{confirmation.entry.command}</pre>}
        {error && <p role="alert" className="whitespace-pre-wrap break-words text-xs text-destructive">{error}</p>}
      </div>
    </ConfirmModal>}
  </div>;
}
