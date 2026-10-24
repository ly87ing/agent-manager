import { useState } from "react";
import { Trash2 } from "lucide-react";
import { ConfirmModal } from "../components/ConfirmModal";
import { clearClaudeData, previewClaudeData, type ClaudeDataPreview, type ClaudeDataResult } from "../lib/claude-data";
import { engineErrorMessage } from "../lib/engine";
import { t } from "../lib/i18n";
import { toast } from "../lib/notifications";

function groupLabel(id: string): string {
  const labels: Record<string, string> = {
    account: t("账号与身份数据"), sessions: t("会话、历史与记忆"),
    configuration: t("配置、技能与插件"), cache: t("缓存与诊断日志"), other: t("其他本地数据"),
  };
  return labels[id] ?? t("其他本地数据");
}

function bytes(value: number): string {
  const units = ["B", "KiB", "MiB", "GiB", "TiB"];
  let index = 0;
  while (value >= 1024 && index < units.length - 1) { value /= 1024; index++; }
  return `${value.toFixed(index ? 1 : 0)} ${units[index]}`;
}

export default function ClaudeDataPage() {
  const [includeCredentials, setIncludeCredentials] = useState(true);
  const [preview, setPreview] = useState<ClaudeDataPreview | null>(null);
  const [result, setResult] = useState<ClaudeDataResult | null>(null);
  const [busy, setBusy] = useState<"preview" | "clear" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [confirm, setConfirm] = useState(false);

  const inspect = async () => {
    setBusy("preview"); setError(null); setPreview(null); setResult(null);
    try { setPreview(await previewClaudeData(!includeCredentials)); }
    catch (error) { setError(engineErrorMessage(error)); }
    finally { setBusy(null); }
  };

  const clear = async () => {
    if (!preview) return;
    setBusy("clear"); setError(null);
    try {
      const outcome = await clearClaudeData(preview);
      setResult(outcome); setPreview(null); setConfirm(false);
      const title = outcome.status === "complete" ? t("所选范围清理完成") : t("清理未全部完成");
      if (outcome.status === "complete") toast.success(title);
      else toast.warning(title, { description: outcome.message });
    } catch (error) { setPreview(null); setConfirm(false); setError(engineErrorMessage(error)); }
    finally { setBusy(null); }
  };

  return (
    <div className="space-y-4" data-testid="claude-data-page">
      <div>
        <h1 className="flex items-center gap-2 text-lg font-semibold"><Trash2 className="h-5 w-5" aria-hidden />{t("Claude 本地数据维护")}</h1>
        <p className="mt-1 text-sm text-muted-foreground">{t("清理 Claude Code 默认用户目录中的会话、账号、记忆、配置与缓存，保留遥测相关配置。")}</p>
      </div>
      <div className="rounded-lg border bg-card p-4 text-sm">
        <p>{t("不会删除项目源文件、外部技能仓库、共享 SDK 认证或云端数据；不会撤销远端令牌。")}</p>
        <p className="mt-2 text-muted-foreground">{t("Shell 文件、现有进程环境和外部认证来源不在本次范围内。用于遥测的认证配置也会保留。")}</p>
        <p className="mt-2 text-muted-foreground">{t("删除后无法恢复。账号可能需要重新登录，设备认证可能需要重新绑定。不会自动备份敏感数据。")}</p>
        <label className="mt-3 flex items-start gap-2">
          <input type="checkbox" checked={includeCredentials} disabled={busy !== null}
            onChange={(event) => { setIncludeCredentials(event.target.checked); setPreview(null); setResult(null); setError(null); }}
            className="mt-0.5 h-4 w-4 accent-primary" />
          <span>{t("同时清理可识别的系统凭据")}<span className="mt-1 block text-xs text-muted-foreground">{t("macOS 会包含当前默认凭据及共享设备认证。其他配置目录和历史凭据名称可能不在本次范围内。")}</span></span>
        </label>
        <button type="button" onClick={() => void inspect()} disabled={busy !== null} className="mt-4 rounded-md border px-3 py-1.5 text-sm font-medium hover:bg-accent disabled:opacity-50">
          {busy === "preview" ? t("正在检查清理范围…") : t("检查清理范围")}
        </button>
      </div>
      {error && !confirm && <p role="alert" className="whitespace-pre-wrap break-words text-sm text-destructive">{error}</p>}
      {preview && <section className="space-y-3 rounded-lg border bg-card p-4" aria-label={t("清理预览")}>
        <h2 className="text-sm font-semibold">{t("清理预览")}</h2>
        <div className="space-y-1 break-all font-mono text-xs text-muted-foreground"><p>{preview.root}</p><p>{preview.config_path}</p></div>
        <div className="divide-y">
          {preview.groups.map((group) => <div key={group.id} className="flex flex-wrap items-center justify-between gap-2 py-2 text-sm">
            <span>{groupLabel(group.id)}</span><span className="text-xs text-muted-foreground">{t("{0} 个文件、{1} 个目录", group.files, group.directories)} · {bytes(group.bytes)}</span>
          </div>)}
          {preview.platform === "macos" && <p className="py-2 text-sm">{preview.credential_items === null ? t("系统凭据未检查、不会清理") : t("可识别的系统凭据：{0} 项", preview.credential_items)}</p>}
        </div>
        {preview.preserved.length > 0 && <div className="rounded-md bg-muted p-3 text-sm">
          <p className="font-medium">{t("保留遥测相关配置")}</p>
          <ul className="mt-1 space-y-1 text-xs">{preview.preserved.map((file) => <li key={file.file} className="break-all">{file.file} · {t("保留 {0} 项设置", file.fields)} · {file.changed ? t("其他设置将清除") : t("文件保持原样")}</li>)}</ul>
        </div>}
        {preview.blockers.length > 0 && <div role="alert" className="space-y-1 rounded-md bg-warning/10 p-3 text-sm text-warning"><p>{t("暂不能安全清理，请处理以下问题后重新检查。")}</p>{preview.blockers.map((message) => <p key={message} className="break-words text-xs">{message}</p>)}</div>}
        {!preview.has_changes && <p role="status" className="text-sm text-muted-foreground">{t("所选范围没有需要清理的数据。")}</p>}
        <button type="button" onClick={() => setConfirm(true)} disabled={!preview.can_clear || busy !== null} className="rounded-md bg-destructive px-3 py-1.5 text-sm font-medium text-destructive-foreground disabled:opacity-50">{t("清理所选范围")}</button>
      </section>}
      {result && <section role="status" className="space-y-2 rounded-lg border bg-card p-4 text-sm">
        <h2 className="font-semibold">{result.status === "complete" ? t("所选范围清理完成") : t("清理未全部完成")}</h2>
        <p>{t("已移除 {0} 个文件、{1} 个目录和 {2} 项系统凭据。", result.deleted_files, result.deleted_directories, result.deleted_credentials)}</p>
        {result.status === "partial" && <p className="text-warning">{t("以上仅统计已确认的删除；未确认的项目可能已发生变化，请检查后再操作。")}</p>}
        <p>{result.telemetry_verified ? t("保留 {0} 项遥测相关设置。", result.preserved_fields) : t("预览要求保留 {0} 项遥测相关设置。", result.preserved_fields)}</p>
        {!result.telemetry_verified && <p className="text-warning">{t("遥测配置未能独立确认，请先检查结果。")}</p>}
        {result.files_only && <p className="text-muted-foreground">{t("本次仅处理文件，系统凭据未清理。")}</p>}
        <p className="whitespace-pre-wrap break-words text-xs text-muted-foreground">{result.message}</p>
        <p className="text-muted-foreground">{t("再次操作前，请重新检查清理范围。")}</p>
      </section>}
      {confirm && preview && <ConfirmModal title={t("清理 Claude 本地数据")} description={t("将永久删除预览中的数据，包括会话、账号、记忆及恢复记录。遥测相关配置会保留。请确认已关闭 Claude 及其集成。")}
        risk="install-remove" riskLabel={t("永久删除")} requireText="DELETE" confirmLabel={t("永久清理所选范围")} executing={busy === "clear"} error={error}
        onConfirm={() => void clear()} onClose={() => {setConfirm(false);setError(null);}} />}
    </div>
  );
}
