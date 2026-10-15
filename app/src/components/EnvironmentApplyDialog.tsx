import { useState } from "react";
import { ConfirmModal } from "./ConfirmModal";
import { Select } from "./Select";
import type { EnvironmentProfile, EnvironmentReport } from "../lib/environment";
import { t } from "../lib/i18n";

export type EnvironmentApplication = "clock" | "launch" | "date-time" | "language-region";

export function EnvironmentApplyDialog({ profile, report, busy, cooldown, error, canConfigureLaunch, onApply, onClose, onClearError }: {
  profile: Pick<EnvironmentProfile, "name" | "target">; report: EnvironmentReport | null; busy: boolean; error: string | null;
  cooldown: boolean;
  canConfigureLaunch: boolean; onApply: (action: EnvironmentApplication) => void; onClose: () => void; onClearError: () => void;
}) {
  const target = profile.target;
  const hasClock = Boolean(target.ntp_server && target.max_clock_offset_ms);
  const hasLaunch = Boolean(target.timezone || target.locale);
  const [action, setAction] = useState<EnvironmentApplication>(hasClock ? "clock" : hasLaunch && canConfigureLaunch ? "launch" : target.region || target.languages.length ? "language-region" : "date-time");
  const labels = { clock: t("校正系统时间"), launch: t("生成启动配置"), "date-time": t("打开日期与时间设置"), "language-region": t("打开语言与地区设置") };
  const rows = action === "clock" ? [["ntp.clock_offset", t("参考时间偏差"), target.max_clock_offset_ms == null ? t("未设置目标") : `± ${target.max_clock_offset_ms} ms`]]
    : action === "launch" ? [["process.timezone", "TZ", target.timezone], ["LC_CTYPE", "locale", target.locale]]
    : action === "date-time" ? [["time.timezone", t("系统时区"), target.timezone], ["ntp.server", t("目标时间服务器"), target.ntp_server],
      ["ntp.automatic_time", t("目标自动校时"), target.automatic_time == null ? null : target.automatic_time ? t("开启") : t("已关闭")],
      ["time.automatic_timezone", t("目标自动时区"), target.automatic_timezone == null ? null : target.automatic_timezone ? t("开启") : t("已关闭")]]
    : [["language.preferred", t("首选语言顺序"), target.languages.join(", ")], ["language.region", t("地区"), target.region]];
  const missing = action === "clock" && !hasClock ? t("校时需要此 Profile 填写时间服务器和偏差容限。可以先保存其他期望值，之后再补充。")
    : action === "clock" && cooldown ? t("两次时间查询至少间隔 4 秒。")
    : action === "launch" && !hasLaunch ? t("请先在 Profile 中填写时区或进程 locale。") : null;
  return <ConfirmModal title={t("应用环境 Profile")} description={t("所选 Profile：{0}。确认本次操作及影响范围后再执行。", profile.name)}
    risk={action === "clock" ? "write-local" : "read"} riskLabel={action === "clock" ? t("仅校正系统时钟") : action === "launch" ? t("预填启动配置") : t("手动调整系统设置")}
    executing={busy} error={error ?? missing} confirmLabel={labels[action]} confirmDisabled={Boolean(missing)} onConfirm={() => onApply(action)} onClose={onClose}>
    <div className="mt-3 space-y-3">
      <Select label={t("应用目标")} value={action} disabled={busy} options={[
        { value: "clock", label: t("本机系统时钟") }, ...(canConfigureLaunch ? [{ value: "launch", label: t("应用启动环境") }] : []),
        { value: "date-time", label: t("日期与时间设置（手动调整）") }, { value: "language-region", label: t("语言与地区设置（手动调整）") },
      ]} onChange={(value) => { setAction(value as EnvironmentApplication); onClearError(); }} />
      <div className="rounded-md border"><table className="w-full table-fixed text-left text-xs">
        <thead className="border-b text-muted-foreground"><tr><th className="p-2">{t("项目")}</th><th className="p-2">{t("实际值")}</th><th className="p-2">{t("期望值")}</th></tr></thead>
        <tbody>{rows.filter(([, , value]) => value).map(([id, name, value]) => <tr key={id} className="border-b last:border-b-0">
          <td className="break-words p-2">{name}</td><td className="break-all p-2 font-mono">{action === "launch" ? t("目标应用尚未选择") : report?.checks.find((row) => row.id === id)?.observed ?? t("尚未对比")}</td>
          <td className="break-all p-2 font-mono">{value}</td>
        </tr>)}</tbody>
      </table></div>
      {action === "clock" && <><p className="break-all text-xs">{t("时间服务器：{0}；偏差容限：{1}。", target.ntp_server ?? t("未设置目标"), target.max_clock_offset_ms == null ? t("未设置目标") : `${target.max_clock_offset_ms} ms`)}</p>
        <p className="text-xs text-muted-foreground">{t("执行前重新测量，仅在偏差明确超限时请求 macOS 授权并校正系统时钟，之后独立复测。其他期望值继续用于对比。")}</p></>}
      {action === "launch" && <p className="text-xs text-muted-foreground">{t("将时区和 locale 填入未保存的启动配置，再选择应用。已有进程需正常退出后，由你重新启动；系统设置保持不变。")}</p>}
      {(action === "date-time" || action === "language-region") && <p className="text-xs text-muted-foreground">{t("本次只打开系统设置。请按上方期望值手动调整，完成后重新对比；打开设置不代表已经应用成功。")}</p>}
    </div>
  </ConfirmModal>;
}
