import { useEffect, useState } from "react";
import type { BrowserEnvironmentReport as Report, EnvironmentCheck } from "../lib/environment";
import { t } from "../lib/i18n";

function label(id: string) {
  return ({ "browser.timezone": t("Chrome 时区"), "browser.languages": t("Chrome 语言顺序"), "browser.intl_locale": t("Chrome 区域格式"),
    "browser.screen": t("屏幕与窗口"), "browser.client_hints": "UA-CH (JavaScript)", "browser.ua_http": "User-Agent (HTTP / JavaScript)",
    "browser.language_http": "Accept-Language / navigator.languages", "browser.ch_platform": "UA-CH platform", "browser.ch_brands": "UA-CH brands",
    "browser.ch_mobile": "UA-CH mobile", "browser.ch_architecture": "UA-CH architecture", "browser.ch_bitness": "UA-CH bitness",
    "browser.ch_platform_version": "UA-CH platform version", "browser.ch_full_versions": "UA-CH full versions",
    "browser.platform_native": t("浏览器与系统平台"), "browser.ua_version": t("浏览器版本声明") } as Record<string, string>)[id] ?? id;
}
function state(row: EnvironmentCheck, expired: boolean) {
  return expired ? t("已过期") : ({ match: t("符合"), mismatch: t("不符"), unknown: t("无法确认"), observed: t("仅观察") })[row.state];
}

export function BrowserEnvironmentReport({ report }: { report?: Report | null }) {
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    setNow(Date.now());
    if (!report) return;
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [report]);
  const expired = report ? now >= Date.parse(report.expires_at) || now < Date.parse(report.checked_at) - 5000 : false;
  return <section className="space-y-3 rounded-lg border bg-card p-4" aria-label={t("Chrome 检查结果")}>
    <h2 className="text-sm font-medium">{t("Chrome 检查结果")}</h2>
    <p className="text-xs text-muted-foreground">{t("仅代表本次诊断页面及其浏览器环境。时区和完整语言顺序与所选 Profile 比较；请求头与同一页面的读数比较。屏幕数据只作观察。")}</p>
    <p className="text-xs text-warning">{t("未检查：GeoIP、WebRTC、Canvas/WebGL、字体、持续时钟偏差、TLS 指纹。已列出的符合项不表示全部环境参数一致。")}</p>
    {!report ? <p role="status" className="text-sm">{t("Chrome 尚未检查。")}</p> : <>
      <p role="status" className="text-sm font-medium">{expired ? t("Chrome 结果已过期，请重新检查。") : report.status === "received"
        ? t("Chrome 已读取：符合 {0} 项，不符 {1} 项，无法确认 {2} 项。", report.summary.matched, report.summary.mismatched, report.summary.unknown)
        : ({ timeout: t("Chrome 未及时返回结果，请重新检查。"), cancelled: t("Chrome 检查已取消，未修改设置。"), unavailable: t("无法打开 Chrome 诊断页面。请确认已安装 Chrome 后重试。") })[report.status]}</p>
      <p className="break-all text-xs text-muted-foreground">{t("检查时间：{0}", report.checked_at)} · {report.origin ?? t("无法确认")}</p>
      {report.sample && <p className="break-words text-xs">{report.sample.user_agent}</p>}
      <div className="divide-y rounded-md border">{report.checks.map(row => <div key={row.id} data-browser-check-id={row.id} className="grid min-w-0 gap-2 p-3 sm:grid-cols-[10rem_minmax(0,1fr)_minmax(0,1fr)]">
        <div className="min-w-0"><p className="break-words text-xs font-medium">{label(row.id)}</p><span className={`mt-1 inline-block whitespace-nowrap rounded px-1.5 py-0.5 text-xs ${expired || row.state === "unknown" ? "bg-warning/10 text-warning" : row.state === "mismatch" ? "bg-destructive/10 text-destructive" : row.state === "match" ? "bg-success/10 text-success" : "bg-muted text-muted-foreground"}`}>{state(row, expired)}</span></div>
        <div className="min-w-0"><p className="text-xs text-muted-foreground">{t("实际值")}</p><p className="break-all font-mono text-xs">{row.observed ?? t("无法确认")}</p></div>
        <div className="min-w-0"><p className="text-xs text-muted-foreground">{t("期望值")}</p><p className="break-all font-mono text-xs">{row.expected ?? t("未设置目标")}</p></div>
      </div>)}</div>
      <details className="text-xs"><summary className="cursor-pointer">{t("显示取值来源与说明")}</summary><pre className="mt-2 whitespace-pre-wrap break-all">{JSON.stringify({ source: report.source, scope: report.scope, session_id: report.session_id, headers: report.headers, sources: report.checks.map(row => ({ id: row.id, source: row.source, note: row.note })) }, null, 2)}</pre></details>
    </>}
  </section>;
}
