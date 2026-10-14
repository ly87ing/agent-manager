import { useEffect, useRef, useState } from "react";
import { toast } from "sonner";
import { Select } from "../components/Select";
import { ConfirmModal } from "../components/ConfirmModal";
import { EnvironmentProfileEditor } from "../components/EnvironmentProfileEditor";
import { EnvironmentRemediation } from "../components/EnvironmentRemediation";
import { checkEnvironment, checkTimeSettings, deleteEnvironmentProfile, environmentLaunchRecipe, listEnvironmentProfiles, openEnvironmentSettings, profileTarget, repairEnvironmentTime, saveEnvironmentProfile, type EnvironmentCheck, type EnvironmentProfile, type EnvironmentProfileInput, type EnvironmentReport } from "../lib/environment";
import type { LaunchRecipe } from "../lib/launch";
import { engineErrorMessage } from "../lib/engine";
import { t, useLanguage } from "../lib/i18n";

const input = "w-full min-w-0 rounded-md border border-input bg-background px-2 py-1.5 text-sm";
const button = "rounded-md border px-3 py-1.5 text-xs hover:bg-accent disabled:opacity-50";
const groups = ["system", "time", "language", "process", "ntp"] as const;

function groupLabel(group: string) {
  return ({ system: t("操作系统"), time: t("时间与时区"), language: t("语言与地区"), process: t("当前进程环境"), ntp: t("网络校时") })[group];
}
function label(id: string) {
  return ({ "os.version": t("系统版本"), "os.arch": t("处理器架构"), "time.local": t("系统本地时间"), "time.timezone": t("系统时区"),
    "time.automatic_timezone": t("自动设置时区"), "language.preferred": t("首选语言顺序"), "language.region": t("地区"), "language.locale": t("系统区域格式"),
    "language.formats": t("自定义区域格式"), "ntp.server": t("配置的时间服务器"), "ntp.automatic_time": t("自动设置时间"), "ntp.service": t("校时服务运行"),
    "ntp.clock_offset": t("参考时间偏差"), "ntp.actual_source": t("后台实际授时来源"), "ntp.synchronized": t("内核时间同步状态"),
    "ntp.kernel_max_error": t("内核最大时间误差"), "ntp.kernel_estimated_error": t("内核估计时间误差"),
    "process.timezone": t("进程实际时区"), "process.variables": t("原始环境变量"), "process.raw_locale": t("原始 locale 一致性") } as Record<string, string>)[id] ?? id;
}
function note(code: string) {
  return ({
    inventory: t("仅展示本机信息。"), local_time: t("读取系统时区下的当前时间；时间准确性见参考时间偏差。"),
    system_timezone: t("系统时区供没有单独覆盖的进程使用。"), automatic_timezone: t("开启后可能随位置变化而改变时区。"),
    language_order: t("按完整语言列表和顺序比较；应用可以另设语言。"), user_region: t("当前登录用户的区域设置，与终端 locale 分别检查。"),
    format_overrides: t("显示已设置的时间制、单位和日期数字格式；这些观察值未纳入目标比较。"),
    configured_only: t("这是配置文件中的服务器；后台实际采用的服务器仍需单独确认。"),
    permission_required: t("macOS 的 systemsetup 读取需要管理员权限，本次无法确认。"),
    automatic_time: t("这是自动校时开关，成功同步需另外验证。"), service_only: t("服务运行不等于已成功同步。"),
    kernel_sync: t("内核报告的同步状态，不提供最近同步时间或后台采用的服务器。实际偏差请比较参考时间。"),
    kernel_error: t("内核校时器报告的误差，未与外部参考时间比较，也不纳入时间偏差目标。"),
    timezone_inherited: t("TZ 未设置，实际时区沿用系统。"), timezone_override: t("进程 TZ 覆盖系统时区；空值按 UTC 处理。"),
    runtime_scope: t("桌面检查只代表 Agent Manager 进程；终端和其他应用需要各自检查。"),
    locale_precedence: t("生效顺序：非空 LC_ALL → 此项 LC_* → 非空 LANG → C。"),
    not_queried: t("尚未查询外部时间服务器。"), server_required: t("请输入时间服务器后再比较参考时间。"),
    query_failed: t("未取得有效回复；可能是网络不通、服务器拒绝或查询间隔不足。"),
    time_sample: t("一次未认证的 NTP 查询，仅测量偏差，不证明后台已同步或服务器所在国家。"),
    clock_uncertain: t("测量误差覆盖了容许范围的边界，无法明确判断。"),
    actual_source_unavailable: t("本次无法可靠读取后台当前采用的授时节点。配置文件和单次查询不能代替此项证据。"),
    raw_locale_consistency: t("未设置的变量不强求；已设置的 locale 值都与目标比较，包括被 LC_ALL 覆盖的值。实际生效值在各 LC_* 项中展示。"),
  } as Record<string, string>)[code] ?? code;
}
function stateLabel(state: EnvironmentCheck["state"]) {
  return ({ match: t("符合"), mismatch: t("不符"), unknown: t("无法确认"), observed: t("未比较") })[state];
}
function showValue(value: string | null) {
  if (value === null) return t("无法读取");
  if (value === "synchronized") return t("已同步（内核报告）");
  if (value === "unsynchronized") return t("未同步或时钟错误（内核报告）");
  if (value === "true") return t("开启");
  if (value === "false") return t("已关闭");
  if (value === "{}") return t("未设置自定义值");
  return value;
}
function repairDescription(report: EnvironmentReport) {
  return report.time_repair ? ({
    not_needed: t("时间偏差符合目标，无需校时。"), unavailable: t("无法可靠判断时间偏差，未执行校时。"),
    verified: t("系统时间已校正，复测偏差符合目标。"), unverified: t("校时命令已完成，但复测尚未确认符合目标。"),
    cancelled: t("已取消管理员授权，未完成校时。"), failed: t("校时未成功，请检查管理员授权和网络连接。"),
  })[report.time_repair.status] : null;
}
function findingDetails(report: EnvironmentReport, limit = Infinity) {
  return ([
    { text: "不符项目：{0}", rows: report.checks.filter((row) => row.expected !== null && row.state === "mismatch") },
    { text: "目标中无法确认：{0}", rows: report.checks.filter((row) => row.expected !== null && row.state === "unknown") },
    { text: "未设目标且无法确认：{0}", rows: report.checks.filter((row) => row.expected === null && row.state === "unknown") },
  ] as const).filter(({ rows }) => rows.length).map(({ text, rows }) => {
    const names = rows.slice(0, limit).map((row) => label(row.id));
    if (rows.length > limit) names.push(t("另 {0} 项", String(rows.length - limit)));
    return t(text, names.join(t("、")));
  });
}
function notifyReport(report: EnvironmentReport) {
  const { matched, mismatched, unknown, compared } = report.summary;
  const repairUnconfirmed = report.time_repair && !["not_needed", "verified"].includes(report.time_repair.status);
  const title = mismatched || unknown ? t("检查完成：{0} 项不符，{1} 项无法确认。", String(mismatched), String(unknown))
    : repairUnconfirmed ? t("检查完成，校时未确认成功。")
    : compared ? t("检查完成：所选目标全部符合。") : t("检查完成：尚未设置比较目标。");
  const level = mismatched || unknown || repairUnconfirmed ? "warning" : compared ? "success" : "info";
  toast[level](title, { id: "environment-result", duration: 10000, closeButton: true,
    description: [t("符合 {0} 项，不符 {1} 项，无法确认 {2} 项。", String(matched), String(mismatched), String(unknown)),
      ...findingDetails(report, 3), repairDescription(report), t("完整明细见检查结果。")].filter(Boolean).join("\n"),
    classNames: { description: "whitespace-pre-line" },
  });
}
function notifyCheckError(message: string) {
  toast.error(t("环境检查失败"), { id: "environment-result", description: message, duration: 10000, closeButton: true,
    classNames: { description: "max-h-48 overflow-y-auto whitespace-pre-wrap break-words" } });
}

export default function EnvironmentPage({ onConfigureLaunch }: { onConfigureLaunch?: (recipe: LaunchRecipe) => void }) {
  useLanguage();
  const [profiles, setProfiles] = useState<EnvironmentProfile[]>([]);
  const [selectedId, setSelectedId] = useState("");
  const [profilesError, setProfilesError] = useState<string | null>(null);
  const [editor, setEditor] = useState<{ initial: EnvironmentProfile | null } | null>(null);
  const [deleting, setDeleting] = useState(false);
  const [removing, setRemoving] = useState(false);
  const [data, setData] = useState<EnvironmentReport | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(true);
  const [evidence, setEvidence] = useState(false);
  const [cooldown, setCooldown] = useState(false);
  const [authorizing, setAuthorizing] = useState(false);
  const [actionMessage, setActionMessage] = useState<string | null>(null);
  const generation = useRef(0);
  const cooldownTimer = useRef<ReturnType<typeof setTimeout>>();
  useEffect(() => {
    const id = ++generation.current;
    void Promise.allSettled([checkEnvironment(), listEnvironmentProfiles()]).then(([report, collections]) => {
      if (id !== generation.current) return;
      if (report.status === "fulfilled") setData(report.value); else setError(engineErrorMessage(report.reason));
      if (collections.status === "fulfilled") setProfiles(collections.value); else setProfilesError(engineErrorMessage(collections.reason));
      setBusy(false);
    });
    return () => { generation.current++; clearTimeout(cooldownTimer.current); toast.dismiss("environment-result"); };
  }, []);

  const selected = profiles.find((profile) => profile.id === selectedId);
  const server = selected?.target.ntp_server;
  function select(id: string) { generation.current++; setSelectedId(id); setData(null); setError(null); setActionMessage(null); toast.dismiss("environment-result"); }
  function startCooldown() {
    setCooldown(true); clearTimeout(cooldownTimer.current);
    cooldownTimer.current = setTimeout(() => setCooldown(false), 4000);
  }
  async function run(forceRepair = false) {
    const id = ++generation.current;
    setBusy(true); setError(null); setData(null); setActionMessage(null);
    toast.dismiss("environment-result");
    const target = selected ? profileTarget(selected) : {};
    const repair = forceRepair || Boolean(selected?.auto_sync);
    try {
      const result = await (repair ? repairEnvironmentTime(target) : checkEnvironment(target, Boolean(server)));
      if (id === generation.current) {
        setData(result);
        notifyReport(result);
        if (server) {
          startCooldown();
        }
      }
    } catch (err) { if (id === generation.current) {
      const message = engineErrorMessage(err); setError(message);
      notifyCheckError(message);
    } }
    finally { if (id === generation.current) setBusy(false); }
  }
  async function openSettings(pane: "date-time" | "language-region") {
    setBusy(true); setError(null); setActionMessage(null);
    try { await openEnvironmentSettings(pane); setActionMessage(t("已请求打开系统设置。完成调整后，请重新检查。")); }
    catch (err) { setError(engineErrorMessage(err)); }
    finally { setBusy(false); }
  }
  async function authorizeTimeSettings() {
    const id = ++generation.current;
    setBusy(true); setError(null); setActionMessage(null);
    toast.dismiss("environment-result");
    try {
      const result = await checkTimeSettings(selected ? profileTarget(selected) : {}, Boolean(server));
      if (id !== generation.current) return;
      if (result.status === "read") {
        setData(result.report); if (server) startCooldown();
        notifyReport(result.report);
        setActionMessage(t("已读取自动设置时间开关并更新检查结果，未修改系统设置。"));
      } else {
        const message = result.status === "cancelled" ? t("已取消授权，保留原检查结果。") : t("未能读取自动设置时间开关，保留原检查结果。可在系统设置中确认。");
        setActionMessage(message);
        toast[result.status === "cancelled" ? "info" : "error"](message, { id: "environment-result", duration: 10000, closeButton: true });
      }
    } catch (err) { if (id === generation.current) { const message = engineErrorMessage(err); setError(message); notifyCheckError(message); } }
    finally { if (id === generation.current) { setBusy(false); setAuthorizing(false); } }
  }
  async function save(input: EnvironmentProfileInput) {
    const saved = await saveEnvironmentProfile(input, editor?.initial ?? undefined);
    setProfiles((items) => [...items.filter((item) => item.id !== saved.id), saved]);
    select(saved.id); setEditor(null);
  }
  async function reloadProfiles() {
    setBusy(true); setProfilesError(null);
    try { const items = await listEnvironmentProfiles(); setProfiles(items); select(""); }
    catch (err) { setProfilesError(engineErrorMessage(err)); }
    finally { setBusy(false); }
  }
  async function remove() {
    if (!selected) return;
    setRemoving(true); setProfilesError(null);
    try { await deleteEnvironmentProfile(selected); setProfiles((items) => items.filter((item) => item.id !== selected.id)); select(""); setDeleting(false); }
    catch (err) { setProfilesError(engineErrorMessage(err)); }
    finally { setRemoving(false); }
  }
  const repairMessage = data ? repairDescription(data) : null;
  return <div className="space-y-4" data-testid="environment-page">
    <div>
      <h1 className="text-lg font-semibold">{t("环境检查")}</h1>
      <p className="mt-1 text-xs text-muted-foreground">{t("选择已保存的期望集合，比较本机实际环境。集合只定义检查基准，不会应用区域或语言设置。")}</p>
    </div>
    <section className="space-y-3 rounded-lg border bg-card p-4" aria-label={t("期望值集合")}>
      <h2 className="text-sm font-medium">{t("期望值集合")}</h2>
      <div className="flex min-w-0 flex-wrap items-center gap-2">
        <div className="min-w-0 flex-1 basis-56"><Select className={input} label={t("选择期望集合")} value={selectedId} disabled={busy || removing}
          options={[{ value: "", label: t("仅查看本机环境") }, ...profiles.slice().sort((a, b) => a.name.localeCompare(b.name)).map((profile) => ({ value: profile.id, label: profile.name }))]} onChange={select} /></div>
        <button className={button} disabled={busy || Boolean(profilesError)} onClick={() => setEditor({ initial: null })}>{t("新增期望集合")}</button>
        {selected && <><button className={button} disabled={busy} onClick={() => setEditor({ initial: selected })}>{t("编辑期望集合")}</button>
          <button className={button} disabled={busy} onClick={() => setDeleting(true)}>{t("删除期望集合")}</button></>}
      </div>
      {!profiles.length && !profilesError && <p className="text-xs text-muted-foreground">{t("尚无期望集合。新增并保存一次，之后选择集合即可重复检查。")}</p>}
      {selected && <p className="text-xs text-muted-foreground">{t("正在比较集合：{0}", selected.name)}</p>}
      <div className="flex flex-wrap gap-2">
        <button className={`${button} bg-primary text-primary-foreground hover:bg-primary/90`} disabled={busy || cooldown || removing}
          onClick={() => void run()}>{busy ? t("正在检查…") : selected ? t("检查所选集合") : t("检查本机环境")}</button>
      </div>
      {server && <p className="break-all text-xs text-muted-foreground">{t("检查会向 {0} 的 UDP 123 端口查询参考时间。", server)}</p>}
      {selected?.auto_sync && <p className="text-xs text-muted-foreground">{t("此集合已启用自动校时：偏差明确超限时会校正系统时间，macOS 可能请求管理员授权，完成后自动复测。")}</p>}
      {cooldown && <p role="status" className="text-xs text-muted-foreground">{t("两次时间查询至少间隔 4 秒。")}</p>}
      {profilesError && <div role="alert" className="space-y-2"><p className="break-words text-xs text-destructive">{profilesError}</p><button className={button} disabled={busy} onClick={() => void reloadProfiles()}>{t("重新读取集合")}</button></div>}
    </section>
    {editor && <EnvironmentProfileEditor initial={editor.initial} observation={data} onSave={save} onClose={() => setEditor(null)} />}
    {authorizing && <ConfirmModal title={t("授权后重新检查")} description={t("macOS 将请求管理员授权，仅运行以下只读查询，完成后更新检查结果。")}
      risk="read" riskLabel={t("只读时间设置")} executing={busy} confirmLabel={t("继续只读检查")} onConfirm={() => void authorizeTimeSettings()} onClose={() => setAuthorizing(false)}>
      <p className="mt-3 whitespace-pre-wrap break-all font-mono text-xs">/usr/sbin/systemsetup -getusingnetworktime</p>
      <p className="mt-3 text-xs text-muted-foreground">{t("不会调整系统时间、时间服务器或任何区域设置。")}</p>
      {server && <p className="mt-2 break-all text-xs text-muted-foreground">{t("检查会向 {0} 的 UDP 123 端口查询参考时间。", server)}</p>}
    </ConfirmModal>}
    {deleting && selected && <ConfirmModal title={t("删除期望集合")} description={t("仅删除集合 {0}，不修改系统环境。", selected.name)} risk="write-local" riskLabel={t("仅删除比较基准")} executing={removing}
      confirmLabel={t("删除期望集合")} onConfirm={() => void remove()} onClose={() => setDeleting(false)}>
      {profilesError && <p role="alert" className="mt-2 break-words text-xs text-destructive">{profilesError}</p>}
    </ConfirmModal>}
    {error && <div role="alert" className="space-y-2 rounded-md border border-destructive/30 p-3 text-sm"><p className="break-words text-destructive">{error}</p><button className={button} disabled={busy} onClick={() => void run()}>{t("重试检查")}</button></div>}
    {repairMessage && <p role="status" className="rounded-md border p-3 text-sm">{repairMessage}</p>}
    {actionMessage && <p role="status" className="rounded-md border p-3 text-sm">{actionMessage}</p>}
    {data && <>
      <section className="space-y-2 rounded-md border p-3" aria-label={t("检查结果")}>
        <p role="status" className="text-sm font-medium">{data.summary.compared === 0 ? t("已读取环境，尚未设置比较目标。") : data.summary.status === "match" ? t("已检查的目标全部符合。") : data.summary.status === "mismatch" ? t("发现与目标不符的项目。") : t("仍有目标项目无法确认。")}</p>
        <p className="text-xs text-muted-foreground">{t("符合 {0} 项，不符 {1} 项，无法确认 {2} 项。", String(data.summary.matched), String(data.summary.mismatched), String(data.summary.unknown))}</p>
        {findingDetails(data).map((detail) => <p key={detail} className="break-words text-xs">{detail}</p>)}
        <p className="text-xs text-muted-foreground">{t("另有 {0} 项未设目标，其中 {1} 项无法确认。", String(data.checks.filter((row) => row.expected === null).length), String(data.checks.filter((row) => row.expected === null && row.state === "unknown").length))}</p>
        <p className="break-all text-xs text-muted-foreground">{t("检查时间：{0}", data.checked_at)}</p>
        <label className="flex items-center gap-2 text-xs"><input type="checkbox" checked={evidence} onChange={(event) => setEvidence(event.target.checked)} />{t("显示取值来源与说明")}</label>
      </section>
      {groups.map((group) => <section key={group} className="space-y-2" aria-label={groupLabel(group)}>
        <h2 className="text-sm font-medium">{groupLabel(group)}</h2>
        <div className="divide-y rounded-md border">{data.checks.filter((row) => row.group === group).map((row) => <div key={row.id} className="grid min-w-0 gap-2 p-3 sm:grid-cols-[10rem_minmax(0,1fr)_minmax(0,1fr)]" data-check-id={row.id}>
          <div className="min-w-0"><p className="break-words text-xs font-medium">{label(row.id)}</p><span className={`mt-1 inline-block whitespace-nowrap rounded px-1.5 py-0.5 text-xs ${row.state === "match" ? "bg-success/10 text-success" : row.state === "mismatch" ? "bg-destructive/10 text-destructive" : row.state === "unknown" ? "bg-warning/10 text-warning" : "bg-muted text-muted-foreground"}`}>{stateLabel(row.state)}</span></div>
          <div className="min-w-0"><p className="text-xs text-muted-foreground">{t("实际值")}</p><p className="whitespace-pre-wrap break-all font-mono text-xs">{showValue(row.observed)}</p>{evidence && <p className="mt-1 break-all text-xs text-muted-foreground">{row.source}</p>}</div>
          <div className="min-w-0"><p className="text-xs text-muted-foreground">{t("期望值")}</p><p className="whitespace-pre-wrap break-all font-mono text-xs">{row.expected === null ? t("未设置目标") : showValue(row.expected)}</p>{evidence && row.state !== "mismatch" && row.state !== "unknown" && <p className="mt-1 text-xs text-muted-foreground">{note(row.note)}</p>}</div>
          <EnvironmentRemediation row={row} report={data} reason={note(row.note)} busy={busy} cooldown={cooldown}
            onRepair={() => void run(true)} onAuthorize={() => setAuthorizing(true)} onSettings={(pane) => void openSettings(pane)} onRecheck={() => void run()}
            onConfigureLaunch={onConfigureLaunch && selected ? () => onConfigureLaunch(environmentLaunchRecipe(profileTarget(selected))) : undefined} />
        </div>)}</div>
      </section>)}
      {data.time_sample && <p className="break-all text-xs text-muted-foreground">{t("查询地址：{0}；往返耗时：{1} ms；服务器层级：{2}。", data.time_sample.peer_address, data.time_sample.round_trip_ms.toFixed(1), String(data.time_sample.stratum))}{data.time_sample.special_use_address && ` ${t("该地址属于本地或特殊用途范围，无法据此确认实际服务器位置。")}`}</p>}
      {data.probe_error && <p role="alert" className="break-words text-xs text-destructive">{data.probe_error}</p>}
    </>}
    <p className="text-xs text-muted-foreground">{t("环境比较保持只读；仅主动校时或集合中启用的自动校时会调整系统时间。其他应用环境与后台实际授时来源仍需另行验证。")}</p>
  </div>;
}
