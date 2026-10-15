import { ActionMenu } from "./AccountGrid";
import type { EnvironmentBuiltin, EnvironmentProfile } from "../lib/environment";
import { t } from "../lib/i18n";

const button = "rounded-md border px-3 py-1.5 text-xs hover:bg-accent disabled:opacity-50";

export function EnvironmentProfiles({ profiles, builtins, loading, error, busy, selectDisabled, onSelectBuiltin, onReload, onNew, onEdit, onCopy, onDelete }: {
  profiles: EnvironmentProfile[]; loading: boolean; error: string | null; busy: boolean;
  builtins: EnvironmentBuiltin[]; selectDisabled: boolean; onSelectBuiltin: (profile: EnvironmentBuiltin) => void;
  onReload: () => void; onNew: () => void; onEdit: (profile: EnvironmentProfile) => void;
  onCopy: (profile: Pick<EnvironmentProfile, "name" | "target">) => void; onDelete: (profile: EnvironmentProfile) => void;
}) {
  return <section className="space-y-3" aria-label={t("环境 Profiles")}>
    <div className="flex flex-wrap items-center justify-between gap-2">
      <p className="min-w-0 flex-1 text-xs text-muted-foreground">{t("独立维护期望值。保存后可在“对比与应用”中选择，保存本身不执行检查或应用。")}</p>
      <button type="button" className={button} disabled={loading || busy} onClick={onReload}>{t("重新读取集合")}</button>
      <button type="button" className={`${button} bg-primary text-primary-foreground hover:bg-primary/90`} disabled={loading || busy || Boolean(error)} onClick={onNew}>{t("新增 Profile")}</button>
    </div>
    {loading && <p role="status" className="text-xs text-muted-foreground">{t("正在读取环境 Profiles…")}</p>}
    {busy && <p role="status" className="text-xs text-muted-foreground">{t("请等待当前操作完成后再编辑 Profile。")}</p>}
    {error && <p role="alert" className="break-words text-xs text-destructive">{error}</p>}
    {builtins.length > 0 && <section aria-label={t("内置环境 Profiles")} className="space-y-2">
      <h2 className="text-sm font-medium">{t("内置环境 Profiles")}</h2>
      <p className="text-xs text-muted-foreground">{t("固定时区，严格检查完整语言顺序和原始 locale；要求自动校时开启、自动时区关闭、内核已同步，时间偏差不超过 2000 毫秒。")}</p>
      <p className="text-xs text-muted-foreground">{t("系统地区与进程 locale 分别比较；新加坡采用 macOS 兼容的英语进程 locale。")}</p>
      <ul className="divide-y rounded-md border">{builtins.map((profile) => <li key={profile.id} className="flex min-w-0 items-center gap-3 p-3">
        <div className="min-w-0 flex-1 space-y-1">
          <h3 className="text-xs font-medium">{profile.name} <span className="text-muted-foreground">· {t("内置")}</span></h3>
          <p className="break-words text-xs">{[profile.target.timezone, profile.target.region, ...profile.target.languages, profile.target.locale].join(" · ")}</p>
          <p className="break-words text-xs">NTP: {profile.target.ntp_server} · {profile.time_service}</p>
          <p className="break-all text-xs text-muted-foreground">{t("官方说明：{0}", profile.source_url)}</p>
        </div>
        <div className="flex w-32 shrink-0 items-center gap-2">
          <button type="button" className={`${button} min-w-0 flex-1`} aria-label={t("选择 Profile {0}", profile.name)} disabled={selectDisabled} onClick={() => onSelectBuiltin(profile)}>{t("选择 Profile")}</button>
          <ActionMenu label={t("内置 Profile 操作 {0}", profile.name)} items={[
            { label: t("复制为自定义 Profile"), testId: `copy-${profile.id}`, disabled: busy || loading, onSelect: () => onCopy(profile) },
          ]} />
        </div>
      </li>)}</ul>
      <p className="text-xs text-muted-foreground">{t("参考服务器的实际可达性以本次检查为准，失败时保留无法确认。需要更换服务器时，可复制为自定义 Profile。")}</p>
    </section>}
    {builtins.length > 0 && <h2 className="text-sm font-medium">{t("自定义 Profiles")}</h2>}
    {!loading && !error && profiles.length === 0 && <p className="text-xs text-muted-foreground">{builtins.length ? t("尚无自定义 Profile。可以新增，或复制内置 Profile 后调整。") : t("尚无环境 Profile。先新增一份，再选择它对比或应用。")}</p>}
    {profiles.length > 0 && <div className="rounded-md border">
      <table className="w-full table-fixed text-left text-xs" aria-label={t("环境 Profiles")}>
        <colgroup><col /><col className="w-40" /><col className="w-36" /><col className="w-32" /></colgroup>
        <thead className="border-b bg-muted/40 text-muted-foreground"><tr>
          <th className="px-3 py-2 font-medium">{t("名称")}</th><th className="px-3 py-2 font-medium">{t("时区")}</th>
          <th className="px-3 py-2 font-medium">locale</th><th className="px-3 py-2 font-medium">{t("操作")}</th>
        </tr></thead>
        <tbody>{profiles.slice().sort((a, b) => a.name.localeCompare(b.name)).map((profile) => <tr key={profile.id} className="border-b last:border-b-0">
          <td className="break-words px-3 py-2"><span className="font-medium">{profile.name}</span>
            {(profile.target.region || profile.target.languages.length > 0) && <p className="mt-1 break-words text-muted-foreground">{[profile.target.region, ...profile.target.languages].filter(Boolean).join(" · ")}</p>}</td>
          <td className="break-all px-3 py-2 font-mono">{profile.target.timezone || t("不比较")}</td>
          <td className="break-all px-3 py-2 font-mono">{profile.target.locale || t("不比较")}</td>
          <td className="px-3 py-2"><div className="flex items-center gap-2">
            <button type="button" className={`${button} min-w-0 flex-1`} aria-label={t("编辑 Profile {0}", profile.name)} disabled={busy || loading} onClick={() => onEdit(profile)}>{t("编辑")}</button>
            <ActionMenu label={t("Profile 操作 {0}", profile.name)} items={[
              { label: t("复制 Profile"), testId: `copy-environment-${profile.id}`, disabled: busy || loading, onSelect: () => onCopy(profile) },
              { label: t("删除 Profile"), testId: `delete-environment-${profile.id}`, danger: true, disabled: busy || loading, onSelect: () => onDelete(profile) },
            ]} />
          </div></td>
        </tr>)}</tbody>
      </table>
    </div>}
  </section>;
}
