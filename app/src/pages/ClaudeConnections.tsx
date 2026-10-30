import { useState } from "react";
import { clearConnection, previewConnection, inspectClaudeConnections, type ConnectionInspection, type ConnectionPreview, type ConnectionResult } from "../lib/claude-connections";
import { ConfirmModal } from "../components/ConfirmModal";
import { toast } from "../lib/notifications";
import { engineErrorMessage } from "../lib/engine";
import { t } from "../lib/i18n";

export function ClaudeConnections({ disabled, onBusy }: { disabled: boolean; onBusy: (busy: boolean) => void }) {
  const [report, setReport] = useState<ConnectionInspection | null>(null);
  const [phase, setPhase] = useState<"inspect" | "preview" | "clear" | null>(null);
  const [preview, setPreview] = useState<ConnectionPreview | null>(null);
  const [result, setResult] = useState<ConnectionResult | null>(null);
  const [root, setRoot] = useState("");
  const [error, setError] = useState<string | null>(null);
  async function inspect() {
    setPhase("inspect"); onBusy(true); setError(null); setReport(null); setPreview(null); setResult(null);
    try { setReport(await inspectClaudeConnections(root.trim() || undefined)); }
    catch (error) { setError(engineErrorMessage(error)); }
    finally { setPhase(null); onBusy(false); }
  }
  async function prepare(item: string) {
    setPhase("preview"); onBusy(true); setError(null); setPreview(null); setResult(null);
    try { setPreview(await previewConnection(item, root.trim() || undefined)); }
    catch (error) { setError(engineErrorMessage(error)); }
    finally { setPhase(null); onBusy(false); }
  }
  async function clear() {
    if (!preview) return;
    setPhase("clear"); onBusy(true); setError(null);
    try {
      const outcome = await clearConnection(preview);
      setResult(outcome); setPreview(null); setReport(null);
      toast[outcome.status === "complete" ? "success" : "warning"](outcome.status === "complete" ? t("选定字段已移除") : t("定向清理未全部确认"), { description: outcome.message });
    } catch (error) { setPreview(null); setReport(null); setError(engineErrorMessage(error)); toast.error(t("定向清理未完成"), { description: engineErrorMessage(error) }); }
    finally { setPhase(null); onBusy(false); }
  }
  const sourceLabel = (id: string) => ({ caller: t("当前检查进程"), "user-environment": t("Windows 用户环境"), "machine-environment": t("Windows 系统环境") } as Record<string, string>)[id] ?? id;
  return <section className="space-y-3 rounded-lg border bg-card p-4 text-sm" aria-label={t("Claude 连接配置")}>
    <h2 className="font-semibold">{t("Claude 连接配置")}</h2>
    <p className="text-muted-foreground">{t("检查所选 Claude 用户目录、当前检查进程和列明的启动来源。仅显示字段名称，不显示密钥或地址值；发现字段不代表它是残留。")}</p>
    <label className="block space-y-1 text-xs"><span>{t("自定义 Claude 目录（留空使用默认目录）")}</span><input aria-label={t("自定义 Claude 目录")} value={root} disabled={disabled || phase !== null} onChange={event => { setRoot(event.target.value); setReport(null); setPreview(null); setResult(null); setError(null); }} placeholder="/YOUR_CLAUDE_CONFIG_DIR" className="w-full rounded-md border bg-background px-3 py-2 font-mono" /></label>
    <button type="button" disabled={disabled || phase !== null} onClick={() => void inspect()} className="rounded-md border px-3 py-1.5 font-medium hover:bg-accent disabled:opacity-50">{phase === "inspect" ? t("正在检查连接配置…") : t("检查连接配置")}</button>
    {phase === "preview" && <p role="status" className="text-muted-foreground">{t("正在准备字段移除预览…")}</p>}
    {error && <p role="alert" className="whitespace-pre-wrap break-words text-destructive">{error}</p>}
    {report && <div className="space-y-3">
      <p className="break-all font-mono text-xs">{report.root}</p>
      <p className="text-xs text-muted-foreground">{t("启动文件仅检查文字引用，不执行脚本。仅支持移除可识别的独立常量赋值；动态或复杂引用会保留并标为待处理。项目、组织管理的设置和未选择的位置尚未检查。")}</p>
      <p className="text-xs text-muted-foreground">{t("每次只移除一个选定字段或赋值。其他配置和会话保留。请先自行退出 Claude 和 CC Switch，再重新预览；Codex 文件不读取、不修改。")}</p>
      {!report.clear_supported && <p className="text-warning">{t("此平台提供只读检查；定向移除目前仅在 macOS 提供。")}</p>}
      {report.custom_config_present && <p className="text-warning">{t("当前进程设置了 CLAUDE_CONFIG_DIR；请核对所选目录是否为实际启动目录，其他目录尚未检查。")}</p>}
      <p className={report.cc_switch === "not_running" ? "text-muted-foreground" : "text-warning"}>{report.cc_switch === "running" ? t("发现 CC Switch 进程，配置可能被再次写入。请先退出该工具。") : report.cc_switch === "unknown" ? t("无法确认 CC Switch 是否运行。") : t("本次未发现 CC Switch 进程；其他写入来源仍需核对。")}</p>
      <div className="divide-y rounded-md border">{report.sources.map(source => <div key={source.id} className="min-w-0 space-y-1 p-3">
        <p className="break-all font-mono text-xs">{source.path ?? sourceLabel(source.id)}</p>
        <p className={source.status === "unavailable" ? "text-xs text-warning" : "text-xs text-muted-foreground"}>{source.status === "unavailable" ? t("无法安全检查：路径、权限、格式或内容尚未确认。") : source.status === "not_found" ? t("此位置不存在。") : source.items.length ? t("发现 {0} 项字段或引用。", source.items.length) : source.keys.length ? t("发现字段：{0}", source.keys.join(", ")) : t("本次未发现所列连接字段。")}</p>
        {source.id === "settings-local" && <p className="text-xs text-muted-foreground">{t("此文件仅作为所选目录中的配置载体检查，未确认它在当前启动场景生效。")}</p>}
        {source.items.map(item => <div key={item.id} className="min-w-0 space-y-1 pt-2"><div className="flex min-w-0 flex-wrap items-center justify-between gap-2">
          <code className="min-w-0 break-all text-xs">{item.key}{item.line !== null && ` · ${t("第 {0} 行", item.line)}`}</code><button type="button" disabled={disabled || phase !== null || !report.clear_supported || !item.editable} onClick={() => void prepare(item.id)} className="rounded-md border px-3 py-1.5 text-xs font-medium hover:bg-accent disabled:opacity-50" aria-label={t("预览移除 {0}", item.key)}>{t("预览移除")}</button>
        </div>{!item.editable && <p className="text-xs text-warning">{t("此项仅检查、尚未清理：引用需要人工核对，或属于影响其他应用的通用共享变量。")}</p>}
        </div>)}
      </div>)}</div>
      <p className="text-xs text-muted-foreground">{t("本次检查不修改文件、环境变量或进程，也不读取或修改 Codex 数据。")}</p>
    </div>}
    {result && <div role="status" className="space-y-2 rounded-md bg-muted p-3">
      <p className="font-medium">{result.status === "complete" ? t("选定字段已移除") : t("定向清理未全部确认")}</p>
      <p className="break-all font-mono text-xs">{result.item}</p><p className="break-words text-xs">{result.message}</p>
      <p className="text-xs text-muted-foreground">{t("结果仅覆盖该字段。请重新检查其他来源，并在重新启动的目标应用中验证实际连接。")}</p>
    </div>}
    {preview && <ConfirmModal title={t("移除选定连接字段")} description={t("将从所列来源移除这一个字段或赋值，保留其他配置。正常 API 密钥也可能在此列；请确认你确实不再需要此项。删除不会撤销远端令牌，也不保证恢复默认连接。")}
      risk="install-remove" riskLabel={t("移除连接配置")} requireText={preview.shared ? "REMOVE SHARED" : "REMOVE"} confirmLabel={t("移除此字段")} confirmDisabled={!preview.can_clear} executing={phase === "clear"} error={error}
      onConfirm={() => void clear()} onClose={() => { setPreview(null); setError(null); }}>
      <p className="mt-3 break-all font-mono text-xs">{preview.path}</p><p className="mt-2 break-all font-mono text-sm">{preview.kind === "settings" ? "env." : ""}{preview.key}{preview.line !== null && ` · ${t("第 {0} 行", preview.line)}`}</p>
      {preview.shared && <p className="mt-3 text-sm text-warning">{t("这是共享来源，移除后会影响今后使用该变量的所有程序；Codex 文件不修改，但尚未排除它的启动依赖。已有进程环境保持原样。请确认影响后输入 REMOVE SHARED。")}</p>}
      <p className="mt-3 text-xs text-muted-foreground">{t("正常完成不保留额外密钥副本。替换结果不确定时会保留临时恢复文件并报告位置；必须核对后再继续。")}</p>
      {preview.blockers.map(blocker => <p key={blocker} role="alert" className="mt-2 break-words text-xs text-warning">{blocker}</p>)}
    </ConfirmModal>}
  </section>;
}
