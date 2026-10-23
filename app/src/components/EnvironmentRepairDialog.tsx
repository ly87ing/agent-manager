import { useEffect, useState } from "react";
import { ConfirmModal } from "./ConfirmModal";
import { previewEnvironmentRepair, profileTarget, repairEnvironment, type EnvironmentProfile, type EnvironmentRepairPlan, type EnvironmentRepairResult } from "../lib/environment";
import { engineErrorMessage } from "../lib/engine";
import { notifyError } from "../lib/notifications";
import { t } from "../lib/i18n";

export function repairStatus(status: string) {
  return ({ unchanged: t("无需修改"), change: t("将修复"), unavailable: t("暂无法修复"), verified: t("已修复并验证"), verified_after_error: t("已符合，但命令异常"),
    outcome_unknown: t("结果无法确认"), stale: t("实际值已变化"), not_applied: t("未执行"), unverified: t("尚未验证符合"), failed: t("修复失败") } as Record<string, string>)[status] ?? status;
}
function label(id: string, platform?: EnvironmentRepairPlan["platform"]) {
  if (id === "language.preferred" && platform === "linux") return t("GNOME 主语言偏好");
  if (id === "language.region" && platform === "windows") return t("用户所在地区");
  if (platform === "linux" && id === "ntp.server") return t("设置当前 timesyncd 实例的运行时 NTP 列表");
  if (id === "ntp.server.persistent") return t("保存标准主机路径的 NTP 配置文件");
  if (id === "ntp.server.directory") return t("创建 NTP 配置目录");
  return ({ "time.timezone": t("系统时区"), "time.automatic_timezone": t("自动设置时区"), "ntp.server": t("配置的时间服务器"), "ntp.automatic_time": t("自动设置时间"),
    "language.preferred": t("首选语言顺序"), "language.region": t("地区（系统区域格式）"), "language.independent_formats": t("保持区域格式独立于语言") } as Record<string, string>)[id] ?? id.replace("session.", "");
}
function value(value: EnvironmentRepairPlan["items"][number]["before"]) {
  return value === null ? t("无法读取") : value === "" ? t("未设置") : typeof value === "boolean" ? value ? t("开启") : t("已关闭") : Array.isArray(value) ? value.join(", ") : value;
}
export function repairSummary(result: EnvironmentRepairResult) {
  if (result.status === "cancelled") return t("已取消系统授权，未继续修复。操作请求已记录。");
  if (result.status === "audit_failed") return t("修复结果未能写入操作记录，请查看逐项结果。");
  if (result.report?.time_repair?.audit_recorded === false) return t("校时结果未能写入操作记录，本页保留复测结果。");
  if (result.status === "partial" || result.clock_error) return t("自动修复未全部完成，请查看逐项结果和重新检查。");
  if (result.verification_limits?.length) return result.status === "not_needed" ? t("所选设置无需修改，实际效果仍需在目标环境中确认。") : t("所选设置已回读，实际效果仍需在目标环境中确认。");
  if (result.report?.summary.status === "match" && (!result.report.time_repair || ["not_needed", "verified"].includes(result.report.time_repair.status))) return t("修复后复查：所选目标全部符合。");
  return t("已执行自动修复，仍有项目需要重启或继续确认。");
}

export function EnvironmentRepairDialog({ profile, onResult, onClose }: {
  profile: Pick<EnvironmentProfile, "name" | "target">; onResult: (result: EnvironmentRepairResult) => void; onClose: () => void;
}) {
  const [plan, setPlan] = useState<EnvironmentRepairPlan | null>(null);
  const [result, setResult] = useState<EnvironmentRepairResult | null>(null);
  const [loading, setLoading] = useState(true);
  const [executing, setExecuting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [reload, setReload] = useState(0);
  const [persistent, setPersistent] = useState(false);
  const [platform, setPlatform] = useState<EnvironmentRepairPlan["platform"]>();
  useEffect(() => {
    let active = true;
    setLoading(true); setPlan(null); setError(null); setResult(null);
    void previewEnvironmentRepair(profileTarget(profile), persistent).then((plan) => { if (active) { setPlan(plan); setPlatform(plan.platform); } })
      .catch((error) => { if (active) setError(engineErrorMessage(error)); }).finally(() => { if (active) setLoading(false); });
    return () => { active = false; };
  }, [profile, reload, persistent]);
  async function apply() {
    if (result) { onClose(); return; }
    if (!plan) return;
    setExecuting(true); setError(null);
    try { const result = await repairEnvironment(profileTarget(profile), plan.base, persistent); setResult(result); onResult(result); }
    catch (error) { const message = engineErrorMessage(error); setError(message); setPlan(null); notifyError(t("自动修复失败"), message); }
    finally { setExecuting(false); }
  }
  const items = result?.items ?? plan?.items ?? [];
  const needsProcessVerification = items.some(item => item.id.startsWith("session.") || item.id.startsWith("language."));
  const gnomeFormats = platform === "linux" && items.some(item => item.id === "language.region");
  const gnomeLanguage = platform === "linux" && items.some(item => item.id === "language.preferred");
  const linuxNtp = platform === "linux" && Boolean(profile.target.ntp_server);
  const ntpStatus = (id: string, status: string) => status === "verified" && linuxNtp ? ({
    "ntp.server": t("运行时列表已设置并读回"), "ntp.server.persistent": t("文件已保存并读回"), "ntp.server.directory": t("目录已创建并读回"),
  } as Record<string, string>)[id] : undefined;
  return <ConfirmModal title={t("自动修复环境")} description={t("所选 Profile：{0}。确认以下变更及影响范围后执行。", profile.name)} risk="write-local" riskLabel={t("修改系统与用户设置")}
    executing={executing} confirmLabel={result ? t("关闭") : t("确认自动修复")} confirmDisabled={!result && (loading || !plan)} onConfirm={() => void apply()} onClose={onClose}
    error={error} secondaryLabel={error || result ? t("重新预览修复") : undefined} onSecondary={() => setReload((value) => value + 1)}>
    {loading ? <p role="status" className="mt-3 text-xs">{t("正在读取修复预览…")}</p> : <div className="mt-3 space-y-3">
      {result && <p role="status" className="text-sm">{repairSummary(result)}</p>}
      <p className="text-xs text-muted-foreground">{platform === "windows" ? t("Windows 语言与地区仅修改当前用户。持久环境变量影响以后登录及资源管理器新启动的进程，不会在退出登录时清除。") : platform === "linux" ? t("Linux 系统时区影响本机；GNOME 自动时区和地区格式偏好仅修改当前用户。") : t("系统设置影响本机；语言与地区影响当前用户。登录会话环境仅供该会话中后续由系统启动的进程使用，退出登录后失效。")}</p>
      {platform === "linux" && items.some(item => item.scope === "session") && <p className="text-xs text-muted-foreground">{t("Linux 会话变量仅供用户服务管理器以后启动的进程使用，管理器退出后失效；已有桌面应用不受影响。")}</p>}
      {platform === "windows" && (profile.target.timezone || profile.target.locale) && !result && <label className="flex items-start gap-2 text-xs"><input type="checkbox" checked={persistent} disabled={executing} onChange={(event) => setPersistent(event.target.checked)} />{t("允许写入 Windows 当前用户的持久环境变量（新进程生效）")}</label>}
      {platform === "windows" && profile.target.languages.length > 0 && <p className="text-xs text-muted-foreground">{t("保留已有语言的输入法选项；新增语言使用系统默认选项。区域格式独立设置会作为单独变更列出。")}</p>}
      {gnomeFormats && !gnomeLanguage && <p className="text-xs text-muted-foreground">{t("GNOME 地区格式将保存为预览中的 locale，语言偏好保持原样。请保存工作后自行退出并重新登录，在新会话中验证；现有应用和当前检查进程保持原样。")}</p>}
      {gnomeLanguage && <><p className="text-xs text-muted-foreground">{t("GNOME 支持单一主语言偏好，需要已安装的 UTF-8 locale。已有语言元数据会保留；多语言回退暂不可修复。继承的地区格式需要同时选择目标地区。")}</p>
        {items.some(item => item.id === "language.independent_formats") && <p className="text-xs text-muted-foreground">{t("将先把所选地区的格式保存为独立偏好，再修改主语言；即使当前地区已符合，也会保存此偏好。若后续失败，已保存的格式会保留。")}</p>}
        <p className="text-xs text-muted-foreground">{t("这里只验证服务中的偏好回读，持久保存、当前显示语言和新会话效果尚未验证；已有语言覆盖变量仍可能影响显示。")}</p></>}
      {needsProcessVerification && !gnomeFormats && !gnomeLanguage && <p className="text-xs text-muted-foreground">{platform === "windows" ? t("Windows 会通知应用环境变量已改变；当前检查进程保持原样。请在重新启动的目标应用中验证。shell 文件和登录项保持不变。") : t("已有应用和终端不会被退出或改写；请保存工作后重新启动，并在目标应用中验证。shell 文件和登录项保持不变。")}</p>}
      {plan && items.some(item => item.id.startsWith("session.")) && <p className="break-all text-xs text-muted-foreground">{t("环境变量范围：{0}", plan.session_scope)}</p>}
      {profile.target.ntp_server && <p className="break-all text-xs">{t("复查会查询 {0} 的 UDP 123 端口。", profile.target.ntp_server)}</p>}
      {linuxNtp && <><p className="break-all text-xs">{t("标准主机路径：/etc/systemd/timesyncd.conf.d/90-agent-manager.conf。需要时创建目录；先保存文件，再设置运行时列表。")}</p>
        <p className="text-xs text-amber-500">{t("设置运行时列表可能使原生时间服务立即查询服务器并调整系统时钟，早于本工具另行测量和授权的校时步骤。")}</p>
        <p className="text-xs text-muted-foreground">{t("不会重启时间服务。以后启动的实例是否加载此主机路径文件仍未验证；文件和运行时列表回读不代表实际 NTP 对端、同步状态或时钟准确度已验证。")}</p></>}
      {profile.target.ntp_server && profile.target.max_clock_offset_ms && <p className="text-xs">{linuxNtp ? t("额外校时步骤的偏差容限：± {0} ms；需要新的测量和单独系统授权。", String(profile.target.max_clock_offset_ms)) : t("时间偏差容限：± {0} ms。仅在新测量明确超限时校正时钟，可能再次请求系统授权。", String(profile.target.max_clock_offset_ms))}</p>}
      <div className="space-y-2">{items.map((item) => <div key={item.id} className="rounded border p-2 text-xs" data-repair-id={item.id}>
        <div className="flex flex-wrap justify-between gap-2"><span className="font-medium">{label(item.id, platform)}</span><span>{ntpStatus(item.id, item.status) ?? (gnomeLanguage && item.id === "language.preferred" && item.status === "verified" ? t("主语言偏好已读回") : undefined) ?? (platform === "linux" && ["time.automatic_timezone", "language.region", "language.independent_formats"].includes(item.id) && item.status === "verified" ? t("偏好已保存并读回") : repairStatus(item.status))}</span></div>
        <p className="mt-1 break-all font-mono">{item.id === "ntp.server.directory" ? t("不存在 → 创建目录") : <>{value(item.before)} → {value(item.expected)}</>}</p>
        <p className="mt-1 text-muted-foreground">{t("影响范围：{0}", item.scope === "system" ? t("本机系统") : item.scope === "user" ? t("当前用户") : t("后续启动的会话进程"))}</p>
      </div>)}</div>
      {platform === "linux" && profile.target.automatic_timezone != null && <p className="text-xs text-muted-foreground">{t("GNOME 自动时区仅验证偏好回读；定位与自动切换效果尚未验证。开启需要定位已启用且桌面服务有时区权限。")}</p>}
      {platform === "linux" && profile.target.automatic_time != null && <p className="text-xs text-muted-foreground">{t("Linux 自动校时开关目前仅支持读取。已符合目标时无需修改；需要切换时，请使用系统日期与时间设置，再重新预览。服务的开机启用状态及实际同步效果尚未验证。")}</p>}
      {profile.target.automatic_timezone && profile.target.timezone && <p className="text-xs text-amber-500">{t("自动时区开启后，系统可能根据位置再次改变固定时区。")}</p>}
      <p className="text-xs text-muted-foreground">{t("系统项目可能请求管理员授权。部分失败会停止后续修改，保留已完成结果；不会自动撤销或重试。")}</p>
      {result?.relogin_required ? <p className="text-xs font-medium">{gnomeLanguage ? t("请保存工作后自行退出并重新登录，再验证显示语言和地区格式。应用不会替你退出登录。") : t("请保存工作后自行退出并重新登录，再验证地区格式。应用不会替你退出登录。")}</p> : result?.restart_required && <p className="text-xs font-medium">{t("请保存工作并重新启动相关应用；当前进程的检查结果保留原样。")}</p>}
      {result?.clock_error && <p role="alert" className="break-words text-xs text-destructive">{result.clock_error}</p>}
      {!!result?.verification_limits?.length && <details className="text-xs text-muted-foreground">
        <summary className="cursor-pointer">{t("验证范围与限制")}</summary>
        <div className="mt-2 space-y-2">{result.verification_limits.map(note => <p key={note} className="break-words">{note}</p>)}</div>
      </details>}
      {result && !result.report && <p className="text-xs text-amber-500">{t("修复后未能读取检查结果，请重新对比 Profile。")} </p>}
    </div>}
  </ConfirmModal>;
}
