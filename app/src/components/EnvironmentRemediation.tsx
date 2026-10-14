import type { EnvironmentCheck, EnvironmentReport } from "../lib/environment";
import { t } from "../lib/i18n";

const button = "rounded-md border px-3 py-1.5 text-xs hover:bg-accent disabled:opacity-50";

export function EnvironmentRemediation({ row, report, reason, busy, cooldown, onRepair, onAuthorize, onSettings, onConfigureLaunch, onRecheck }: {
  row: EnvironmentCheck; report: EnvironmentReport; reason: string; busy: boolean; cooldown: boolean;
  onRepair: () => void; onAuthorize: () => void; onSettings: (pane: "date-time" | "language-region") => void;
  onConfigureLaunch?: () => void; onRecheck: () => void;
}) {
  if (row.state !== "mismatch" && row.state !== "unknown") return null;
  const process = row.group === "process";
  const clock = row.id === "ntp.clock_offset";
  const permission = row.id === "ntp.automatic_time" && row.state === "unknown";
  const scope = process ? t("Agent Manager 当前进程") : row.group === "language" ? t("当前 macOS 用户") : clock ? t("本次参考时间测量") : t("本机系统");
  const effectiveLocaleMatches = row.id === "process.raw_locale" && report.checks.filter((item) => item.id.startsWith("LC_")).every((item) => item.state === "match");
  let guidance = reason;
  let action = null;
  if (process && row.state === "mismatch") {
    guidance = effectiveLocaleMatches ? t("实际生效的 locale 符合目标，但已设置的原始变量存在差异。") : reason;
    action = onConfigureLaunch && <button className={button} disabled={busy} onClick={onConfigureLaunch}>{t("创建启动配置")}</button>;
  } else if (permission) {
    action = <button className={button} disabled={busy || cooldown} onClick={onAuthorize}>{t("授权后重新检查")}</button>;
  } else if (clock && row.state === "mismatch" && report.target.ntp_server && report.target.max_clock_offset_ms) {
    action = <button className={button} disabled={busy || cooldown} onClick={onRepair}>{t("立即校时（系统授权）")}</button>;
  } else if (row.state === "mismatch" && (row.group === "time" || row.id === "ntp.server" || row.id === "ntp.automatic_time")) {
    guidance = t("在系统设置 → 通用 → 日期与时间中，将此项调整为期望值，然后重新检查。");
    action = <button className={button} disabled={busy} onClick={() => onSettings("date-time")}>{t("打开日期与时间设置")}</button>;
  } else if (row.state === "mismatch" && row.group === "language") {
    guidance = t("在系统设置 → 通用 → 语言与地区中调整此项；部分应用需要重启。完成后重新检查。");
    action = <button className={button} disabled={busy} onClick={() => onSettings("language-region")}>{t("打开语言与地区设置")}</button>;
  } else if ((clock || row.id === "ntp.synchronized") && report.target.ntp_server) {
    action = <button className={button} disabled={busy || cooldown} onClick={onRecheck}>{t("重新检查所选集合")}</button>;
  }
  return <div className="space-y-2 rounded bg-muted/40 p-2 sm:col-span-3" aria-label={t("处理建议")}>
    <p className="text-xs">{guidance}</p>
    <p className="text-xs text-muted-foreground">{t("影响范围：{0}", scope)}</p>
    {process && row.state === "mismatch" && <p className="text-xs text-muted-foreground">{t("把期望值带入启动配置，选择需要调整的应用，再预览或保存命令。先完全退出目标应用，再用新配置启动；其他应用需各自检查。")}</p>}
    {clock && row.state === "mismatch" && <p className="text-xs text-muted-foreground">{t("校时会调整本机系统时间，可能请求管理员授权；完成后自动复测。")}</p>}
    {permission && <p className="text-xs text-muted-foreground">{t("只读取自动设置时间开关，不校时、不修改设置。也可在系统设置 → 通用 → 日期与时间中手动确认。")}</p>}
    {row.id === "ntp.actual_source" && <p className="text-xs text-muted-foreground">{t("此项无法确认不代表配置错误。请结合配置的服务器、内核同步状态和参考时间偏差判断。")}</p>}
    {action && <div>{action}</div>}
  </div>;
}
