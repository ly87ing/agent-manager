import { useState } from "react";
import { ConfirmModal } from "./ConfirmModal";
import { Select } from "./Select";
import { type EnvironmentProfile, type EnvironmentProfileInput, type EnvironmentReport, type EnvironmentTarget } from "../lib/environment";
import { engineErrorMessage } from "../lib/engine";
import { notifyError } from "../lib/notifications";
import { t } from "../lib/i18n";
import { listTimeZones } from "../lib/launch";

const input = "w-full min-w-0 rounded-md border border-input bg-background px-2 py-1.5 text-sm";
const booleanText = (value: boolean | null | undefined) => value == null ? "" : String(value);

export function EnvironmentProfileEditor({ initial, observation, onSave, onClose }: {
  initial: EnvironmentProfile | null; observation: EnvironmentReport | null;
  onSave: (input: EnvironmentProfileInput) => Promise<void>; onClose: () => void;
}) {
  const target = initial?.target;
  const [name, setName] = useState(initial?.name ?? "");
  const [timezone, setTimezone] = useState(target?.timezone ?? "");
  const [languages, setLanguages] = useState(target?.languages.join(", ") ?? "");
  const [region, setRegion] = useState(target?.region ?? "");
  const [locale, setLocale] = useState(target?.locale ?? "");
  const [strictLocale, setStrictLocale] = useState(target?.strict_locale ?? true);
  const [server, setServer] = useState(target?.ntp_server ?? "");
  const [automaticTime, setAutomaticTime] = useState(booleanText(target?.automatic_time));
  const [automaticTimezone, setAutomaticTimezone] = useState(booleanText(target?.automatic_timezone));
  const [timeSynchronized, setTimeSynchronized] = useState(booleanText(target?.time_synchronized));
  const [tolerance, setTolerance] = useState(target?.max_clock_offset_ms == null ? "" : String(target.max_clock_offset_ms));
  const [autoSync, setAutoSync] = useState(initial?.auto_sync ?? false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const validTolerance = !tolerance || (Number.isInteger(Number(tolerance)) && Number(tolerance) >= 1 && Number(tolerance) <= 60000);
  const choices = [{ value: "", label: t("不比较") }, { value: "true", label: t("开启") }, { value: "false", label: t("已关闭") }];
  const zones = [{ value: "", label: t("不比较") }, ...listTimeZones().map((zone) => ({ value: zone, label: zone }))];

  function useCurrent() {
    if (!observation) return;
    const value = (id: string) => observation.checks.find((row) => row.id === id)?.observed ?? "";
    const locales = observation.checks.filter((row) => row.id.startsWith("LC_")).map((row) => row.observed);
    setTimezone(value("time.timezone")); setLanguages(value("language.preferred")); setRegion(value("language.region"));
    setLocale(locales.length && locales.every((item) => item === locales[0]) ? locales[0] ?? "" : "");
    setServer(value("ntp.server")); setAutomaticTime(value("ntp.automatic_time")); setAutomaticTimezone(value("time.automatic_timezone"));
    setTimeSynchronized(value("ntp.synchronized") === "synchronized" ? "true" : value("ntp.synchronized") === "unsynchronized" ? "false" : "");
  }
  async function save() {
    const expected: EnvironmentTarget = {
      ...(timezone && { timezone }), ...(languages.trim() && { languages: languages.split(",").map((value) => value.trim()) }),
      ...(region && { region }), ...(locale.trim() && { locale: locale.trim(), strict_locale: strictLocale }),
      ...(server.trim() && { ntp_server: server.trim() }), ...(automaticTime && { automatic_time: automaticTime === "true" }),
      ...(automaticTimezone && { automatic_timezone: automaticTimezone === "true" }), ...(timeSynchronized && { time_synchronized: timeSynchronized === "true" }),
      ...(tolerance && { max_clock_offset_ms: Number(tolerance) }),
    };
    setSaving(true); setError(null);
    try { await onSave({ name: name.trim(), target: expected, auto_sync: autoSync }); }
    catch (err) { setError(engineErrorMessage(err)); notifyError(t("保存期望集合失败"), err); setSaving(false); }
  }
  return <ConfirmModal title={initial ? t("编辑期望集合") : t("新增期望集合")} description={t("保存的是比较基准，不会应用到系统。留空的项目只展示实际值。")}
    risk="write-local" riskLabel={t("仅保存比较基准")} error={error} executing={saving} confirmLabel={t("保存期望集合")} confirmDisabled={!name.trim() || !validTolerance || (autoSync && (!server.trim() || !tolerance))}
    onConfirm={() => void save()} onClose={onClose}>
    <fieldset disabled={saving} className="mt-3 grid min-w-0 gap-3 sm:grid-cols-2">
      <label className="min-w-0 text-xs sm:col-span-2">{t("集合名称")}<input className={`${input} mt-1`} value={name} maxLength={120} onChange={(event) => setName(event.target.value)} /></label>
      <button type="button" className="justify-self-start rounded-md border px-3 py-1.5 text-xs disabled:opacity-50 sm:col-span-2" disabled={!observation} onClick={useCurrent}>{t("用当前值填写目标")}</button>
      <div className="min-w-0"><span className="mb-1 block text-xs">{t("目标时区")}</span><Select label={t("目标时区")} value={timezone} options={zones} disabled={saving} onChange={setTimezone} /></div>
      <label className="min-w-0 text-xs">{t("目标语言顺序")}<input className={`${input} mt-1`} value={languages} placeholder="en-GB, fr-FR" onChange={(event) => setLanguages(event.target.value)} /></label>
      <label className="min-w-0 text-xs">{t("目标地区")}<input className={`${input} mt-1`} value={region} maxLength={2} placeholder="GB" onChange={(event) => setRegion(event.target.value.toUpperCase())} /></label>
      <label className="min-w-0 text-xs">{t("目标进程 locale")}<input className={`${input} mt-1`} value={locale} placeholder="en_GB.UTF-8" onChange={(event) => setLocale(event.target.value)} /></label>
      <label className="min-w-0 text-xs">{t("目标时间服务器")}<input className={`${input} mt-1`} value={server} placeholder="pool.ntp.org" onChange={(event) => setServer(event.target.value)} /></label>
      <label className="min-w-0 text-xs">{t("容许时间偏差（毫秒）")}<input className={`${input} mt-1`} type="number" min={1} max={60000} value={tolerance} placeholder="2000" aria-invalid={!validTolerance} onChange={(event) => setTolerance(event.target.value)} /></label>
      <div className="min-w-0"><span className="mb-1 block text-xs">{t("目标自动校时")}</span><Select label={t("目标自动校时")} value={automaticTime} options={choices} disabled={saving} onChange={setAutomaticTime} /></div>
      <div className="min-w-0"><span className="mb-1 block text-xs">{t("目标自动时区")}</span><Select label={t("目标自动时区")} value={automaticTimezone} options={choices} disabled={saving} onChange={setAutomaticTimezone} /></div>
      <div className="min-w-0 sm:col-span-2"><span className="mb-1 block text-xs">{t("目标时间同步状态")}</span><Select label={t("目标时间同步状态")} value={timeSynchronized}
        options={[{ value: "", label: t("不比较") }, { value: "true", label: t("已同步（内核报告）") }, { value: "false", label: t("未同步或时钟错误（内核报告）") }]} disabled={saving} onChange={setTimeSynchronized} /></div>
      <label className="flex items-center gap-2 text-xs sm:col-span-2"><input type="checkbox" checked={strictLocale} onChange={(event) => setStrictLocale(event.target.checked)} />{t("同时检查已设置的原始 locale 变量")}</label>
      <label className="flex items-center gap-2 text-xs sm:col-span-2"><input type="checkbox" checked={autoSync} onChange={(event) => setAutoSync(event.target.checked)} />{t("检查发现时间偏差超限时自动校时")}</label>
      <p className="text-xs text-muted-foreground sm:col-span-2">{t("自动校时需要填写时间服务器和偏差容限。仅在主动检查此集合时执行，macOS 可能请求管理员授权，随后自动复测。")}</p>
    </fieldset>
    {!validTolerance && <p role="alert" className="mt-2 text-xs text-destructive">{t("时间偏差请输入 1 到 60000 之间的整数。")}</p>}
  </ConfirmModal>;
}
