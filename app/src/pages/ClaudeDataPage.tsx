import { useState } from "react";
import { Trash2 } from "lucide-react";
import { ConfirmModal } from "../components/ConfirmModal";
import { clearClaudeData, previewClaudeData, type ClaudeDataPreview, type ClaudeDataResult } from "../lib/claude-data";
import { engineErrorMessage } from "../lib/engine";
import { t } from "../lib/i18n";
import { toast } from "../lib/notifications";
import { ClaudeOrcaSearch } from "./ClaudeOrcaSearch";

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

function RetainedContent({ items, verified = true }: { items: ClaudeDataPreview["retained"]; verified?: boolean }) {
  if (!items.length) return null;
  return <div className="space-y-2 rounded-md bg-muted p-3 text-sm">
    <p className="font-medium">{verified ? t("已保留的内容") : t("预览中的保留内容，当前状态未确认")}</p>
    <ul className="space-y-2 text-xs">{items.map((item) => <li key={item.path}>
      <p className="break-all font-mono">{item.path}</p>
      <p className="text-muted-foreground">{item.reason === "project" ? t("项目或工作树，保留代码和文档") : t("归属未确认，已保留待核对")}</p>
    </li>)}</ul>
  </div>;
}

export default function ClaudeDataPage() {
  const [includeCredentials, setIncludeCredentials] = useState(true);
  const [preview, setPreview] = useState<ClaudeDataPreview | null>(null);
  const [result, setResult] = useState<ClaudeDataResult | null>(null);
  const [busy, setBusy] = useState<"preview" | "clear" | "orca-preview" | null>(null);
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
        <p className="mt-1 text-sm text-muted-foreground">{t("清理 Claude Code 默认用户目录中的会话、账号、记忆、配置与缓存，包括遥测配置。")}</p>
        <p className="mt-2 text-sm text-muted-foreground">{t("确认后先关闭当前用户下已识别的 Claude 会话进程，再删除数据。未退出的进程会被强制关闭；请先保存工作。")}</p>
        <p className="mt-2 text-sm text-warning">{t("全来源清理尚未完成。以下结果仅覆盖列明的范围，未检查的位置不能视为已清理。")}</p>
      </div>
      <div className="rounded-lg border bg-card p-4 text-sm">
        <p>{t("不会删除项目源文件、外部技能仓库、共享 SDK 认证或云端数据；不会撤销远端令牌。")}</p>
        <p className="mt-2 text-muted-foreground">{t("本次不修改 Shell 文件和现有进程环境，也不删除配置引用的外部证书、脚本和共享认证。")}</p>
        <p className="mt-2 text-muted-foreground">{t("归属未确认的内容会保留，并列为未完成项。")}</p>
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
        <p className="break-all font-mono text-xs text-muted-foreground">{preview.root}</p>
        <p className="text-sm">{preview.process_ids === null ? t("待关闭的 Claude 进程尚未确认。") : t("将关闭 {0} 个 Claude 进程。", preview.process_ids.length)}</p>
        {preview.configuration_paths.length > 0 && <div className="space-y-2 rounded-md bg-muted p-3 text-sm">
          <h3 className="font-medium">{t("将清理的配置与备份")}</h3>
          <ul className="max-h-40 space-y-1 overflow-y-auto font-mono text-xs">{preview.configuration_paths.map((path) => <li key={path} className="break-all">{path}</li>)}</ul>
        </div>}
        <div className="divide-y">
          {preview.groups.map((group) => <div key={group.id} className="flex flex-wrap items-center justify-between gap-2 py-2 text-sm">
            <span>{groupLabel(group.id)}</span><span className="text-xs text-muted-foreground">{t("{0} 个文件、{1} 个目录", group.files, group.directories)} · {bytes(group.bytes)}</span>
          </div>)}
          {preview.platform === "macos" && <p className="py-2 text-sm">{preview.credential_items === null ? t("系统凭据未检查、不会清理") : t("可识别的系统凭据：{0} 项", preview.credential_items)}</p>}
        </div>
        <p className="text-sm text-muted-foreground">{t("所选范围内的遥测配置也将删除。")}</p>
        {preview.blockers.length > 0 && <div role="alert" className="space-y-1 rounded-md bg-warning/10 p-3 text-sm text-warning"><p>{t("暂不能安全清理，请处理以下问题后重新检查。")}</p>{preview.blockers.map((message) => <p key={message} className="break-words text-xs">{message}</p>)}</div>}
        <RetainedContent items={preview.retained} />
        {!preview.has_changes && <p role="status" className="text-sm text-muted-foreground">{preview.retained.some((item) => item.reason === "unclassified") ? t("仍有归属未确认的内容，请先核对保留路径。") : t("所选范围没有需要清理的数据。")}</p>}
        <button type="button" onClick={() => setConfirm(true)} disabled={!preview.can_clear || busy !== null} className="rounded-md bg-destructive px-3 py-1.5 text-sm font-medium text-destructive-foreground disabled:opacity-50">{t("清理所选范围")}</button>
      </section>}
      {result && <section role="status" className="space-y-2 rounded-lg border bg-card p-4 text-sm">
        <h2 className="font-semibold">{result.status === "complete" ? t("所选范围清理完成") : t("清理未全部完成")}</h2>
        <p>{t("已确认关闭 {0} 个 Claude 进程，其中 {1} 个强制关闭。", result.stopped_processes, result.forced_processes)}</p>
        <p>{t("已移除 {0} 个文件、{1} 个目录和 {2} 项系统凭据。", result.deleted_files, result.deleted_directories, result.deleted_credentials)}</p>
        {result.status === "partial" && <p className="text-warning">{t("以上仅统计已确认的删除；未确认的项目可能已发生变化，请检查后再操作。")}</p>}
        {!result.scope_verified && <p className="text-warning">{t("未能独立确认清理后的范围，请先检查结果。")}</p>}
        {result.files_only && <p className="text-muted-foreground">{t("本次仅处理文件，系统凭据未清理。")}</p>}
        <p className="whitespace-pre-wrap break-words text-xs text-muted-foreground">{result.message}</p>
        <RetainedContent items={result.retained} verified={result.retained_verified} />
        <p className="text-muted-foreground">{t("再次操作前，请重新检查清理范围。")}</p>
      </section>}
      <ClaudeOrcaSearch disabled={busy !== null} onBusy={(checking) => setBusy(checking ? "orca-preview" : null)} />
      {confirm && preview && <ConfirmModal title={t("清理 Claude 本地数据")} description={t("将先关闭预览中的 Claude 进程，必要时强制关闭，再永久删除会话、账号、记忆、恢复记录及遥测配置。关闭时写回的数据需要重新检查。")}
        risk="install-remove" riskLabel={t("永久删除")} requireText="DELETE" confirmLabel={t("永久清理所选范围")} executing={busy === "clear"} error={error}
        onConfirm={() => void clear()} onClose={() => {setConfirm(false);setError(null);}} />}
    </div>
  );
}
