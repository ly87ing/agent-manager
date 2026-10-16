import { useState } from "react";
import { ConfirmModal } from "./ConfirmModal";
import { t } from "../lib/i18n";
import { engineErrorMessage } from "../lib/engine";
import { toast, notifyError } from "../lib/notifications";
import { useRegisterShareLinks, useShareLinkRegistration, type ShareLinkPreview } from "../lib/shareLinkRegistration";

export function ShareLinkRegistration() {
  const state = useShareLinkRegistration();
  const change = useRegisterShareLinks();
  const [preview, setPreview] = useState<ShareLinkPreview | null>(null);
  const error = state.error ?? change.error;
  if (state.isPending) return <p role="status" className="text-xs text-muted-foreground">{t("正在读取分享链接关联…")}</p>;
  if (!state.data && !error) return null;
  return <div className="space-y-2 border-t px-4 py-3">
    <div className="flex flex-wrap items-center gap-2">
      <div className="min-w-0 flex-1 basis-48">
        <div className="text-sm">{t("AppImage 分享链接")}</div>
        <p className="text-xs text-muted-foreground">{t("让桌面使用此 AppImage 打开 agentmanager:// 分享链接。链接只打开导入预览。")}</p>
      </div>
      {state.data?.registered ? <span className="text-xs text-muted-foreground">{t("已关联此应用")}</span> : <button type="button" disabled={!state.data || change.isPending} onClick={() => { change.reset(); setPreview(state.data ?? null); }} className="rounded-md border px-3 py-1 text-xs disabled:opacity-50">{t("预览链接关联")}</button>}
      <button type="button" disabled={change.isPending || state.isFetching} onClick={() => { change.reset(); void state.refetch(); }} className="rounded-md border px-3 py-1 text-xs disabled:opacity-50">{t("重新读取")}</button>
    </div>
    {error && <p role="alert" className="break-words text-xs text-destructive">{engineErrorMessage(error)}</p>}
    {preview && <ConfirmModal title={t("关联 AppImage 分享链接")} description={t("将分享链接的默认打开方式设为此应用。现有应用文件会保留；其他类型的链接不受影响。移动 AppImage 后需要重新检查关联。")}
      risk="write-local" confirmLabel={t("确认关联链接")} executing={change.isPending} error={change.error ? engineErrorMessage(change.error) : null}
      onClose={() => setPreview(null)} onConfirm={() => change.mutate(preview.base, {
        onSuccess: () => { setPreview(null); toast.success(t("分享链接关联已验证")); },
        onError: (reason) => notifyError(t("分享链接关联失败"), reason),
      })}>
      <dl className="mt-3 space-y-2 break-all text-xs">
        <div><dt className="text-muted-foreground">{t("当前打开方式")}</dt><dd>{preview.current_handler || t("未设置")}</dd></div>
        <div><dt className="text-muted-foreground">{t("应用路径")}</dt><dd>{preview.executable}</dd></div>
        <div><dt className="text-muted-foreground">{t("将写入的桌面条目")}</dt><dd>{preview.entry}</dd></div>
      </dl>
    </ConfirmModal>}
  </div>;
}
