import { useState } from "react";
import { previewOrcaSearch, type OrcaSearchPreview } from "../lib/claude-data";
import { engineErrorMessage } from "../lib/engine";
import { t } from "../lib/i18n";

export function ClaudeOrcaSearch({ disabled, onBusy }: { disabled: boolean; onBusy: (busy: boolean) => void }) {
  const [checking, setChecking] = useState(false);
  const [preview, setPreview] = useState<OrcaSearchPreview | null>(null);
  const [error, setError] = useState<string | null>(null);
  const inspect = async () => {
    setChecking(true); onBusy(true); setPreview(null); setError(null);
    try { setPreview(await previewOrcaSearch()); }
    catch (error) { setError(engineErrorMessage(error)); }
    finally { setChecking(false); onBusy(false); }
  };
  return <section className="space-y-3 rounded-lg border bg-card p-4 text-sm" aria-label={t("Orca 中的 Claude 会话副本")}>
    <h2 className="font-semibold">{t("Orca 中的 Claude 会话副本")}</h2>
    <p className="text-muted-foreground">{t("当前仅检查搜索索引中的记录数量，尚不支持清理。不会展示会话正文。")}</p>
    <button type="button" onClick={() => void inspect()} disabled={disabled} className="rounded-md border px-3 py-1.5 font-medium hover:bg-accent disabled:opacity-50">
      {checking ? t("正在检查 Orca 索引…") : t("检查 Orca 搜索索引")}
    </button>
    {error && <p role="alert" className="whitespace-pre-wrap break-words text-destructive">{error}</p>}
    {preview && <div className="space-y-2" role="status">
      <p className="text-xs text-muted-foreground">{preview.location_origin === "override" ? t("检查位置来自当前进程的 Orca 路径设置") : t("检查位置为当前用户的默认 Orca 目录")}</p>
      <p className="break-all font-mono text-xs">{preview.path}</p>
      {preview.counts ? <dl className="space-y-2">
        {[
          [t("Claude 会话"), preview.counts.claude_sessions],
          [t("Claude 消息"), preview.counts.claude_messages],
          [t("Claude 文件索引"), preview.counts.claude_file_cursors],
          [t("非 Claude 标识的会话"), preview.counts.non_claude_sessions],
          [t("归属未确认的消息"), preview.counts.unattributed_messages],
          [t("归属未确认的文件索引"), preview.counts.unattributed_file_cursors],
          [t("归属未确认的搜索记录"), preview.counts.unattributed_fts_rows],
        ].map(([label, value]) => <div key={label} className="flex items-start justify-between gap-3"><dt>{label}</dt><dd className="shrink-0 tabular-nums">{value}</dd></div>)}
      </dl> : <p className="text-warning">{preview.status === "not_found" ? t("此位置未发现索引，其他位置仍未检查。") : t("未能安全检查索引。请关闭 Orca 后重试；不支持的格式或路径仍需核对。")}</p>}
      <p className="text-warning">{t("尚未清理 Orca 数据。其他会话存储、自定义目录和旧索引残留仍未检查。")}</p>
    </div>}
  </section>;
}
