import { ActionMenu } from "./AccountGrid";
import type { EnvironmentProfile } from "../lib/environment";
import { t } from "../lib/i18n";

const button = "rounded-md border px-3 py-1.5 text-xs hover:bg-accent disabled:opacity-50";

export function EnvironmentProfiles({ profiles, loading, error, busy, onReload, onNew, onEdit, onCopy, onDelete }: {
  profiles: EnvironmentProfile[]; loading: boolean; error: string | null; busy: boolean;
  onReload: () => void; onNew: () => void; onEdit: (profile: EnvironmentProfile) => void;
  onCopy: (profile: EnvironmentProfile) => void; onDelete: (profile: EnvironmentProfile) => void;
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
    {!loading && !error && profiles.length === 0 && <p className="text-xs text-muted-foreground">{t("尚无环境 Profile。先新增一份，再选择它对比或应用。")}</p>}
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
