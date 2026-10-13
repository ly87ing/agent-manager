import { t } from "../lib/i18n";
import { useState } from "react";
import { X } from "lucide-react";
import { useDialogFocus } from "../lib/dialogFocus";

/**
 * 填写一个凭据变量的取值。
 *
 * 只写不读：界面任何地方都不回显已保存的取值，要改只能覆写。
 * 删除走独立确认，不能靠“空值保存”这种不明确动作触发。
 */
export function SecretValueDialog({
  variable,
  stored,
  busy,
  onSubmit,
  onRequestClear,
  onClose,
}: {
  variable: string;
  /** 该变量是否存在于外部密钥文件——决定是否给独立的删除入口。 */
  stored: boolean;
  busy: boolean;
  onSubmit: (value: string) => void;
  onRequestClear?: () => void;
  onClose: () => void;
}) {
  const [value, setValue] = useState("");
  const { dialogRef, onDialogKeyDown } = useDialogFocus<HTMLDivElement>({
    onEscape: busy ? undefined : onClose,
    initialFocusSelector: "#secret-value-input",
  });

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-slate-900/40 p-4 animate-fade-in">
      <div ref={dialogRef} role="dialog" aria-modal="true" aria-label={t("填写 {0}", variable)} tabIndex={-1} onKeyDown={onDialogKeyDown} className="max-h-[calc(100vh-2rem)] w-full max-w-md overflow-y-auto rounded-lg border bg-card p-5 shadow-lg">
        <div className="flex items-start justify-between gap-3">
          <div className="min-w-0">
            <h3 className="text-base font-semibold text-foreground">{t("填写取值")}</h3>
            <p className="mt-0.5 break-all font-mono text-xs text-muted-foreground">{variable}</p>
          </div>
          <button type="button" onClick={onClose} aria-label={t("关闭")} className="rounded p-1 text-muted-foreground hover:bg-accent">
            <X className="h-4 w-4" aria-hidden />
          </button>
        </div>

        <label className="mt-4 block text-xs font-medium text-muted-foreground" htmlFor="secret-value-input">{t("取值")}</label>
        <input
          id="secret-value-input"
          type="password"
          value={value}
          autoComplete="off"
          data-testid="secret-value-input"
          onChange={(event) => setValue(event.target.value)}
          placeholder={stored ? t("粘贴新取值以覆盖") : t("粘贴取值")}
          className="mt-1 w-full rounded border border-input px-2 py-1.5 font-mono text-xs"
        />

        <p className="mt-3 rounded-md border bg-muted px-3 py-2 text-[11px] leading-relaxed text-muted-foreground">{t("取值只写入“设置 → 密钥”选择的外部文件，不进入本机数据库、操作记录或配置备份。\n          同步到部分助手时，目标配置本身可能需要明文；保存前请确认你信任该文件位置。")}</p>

        <div className="sticky -bottom-5 z-10 -mx-5 -mb-5 border-t bg-card px-5 py-3 mt-4 flex justify-end gap-2">
          {stored && onRequestClear && (
            <button
              type="button"
              data-testid="secret-value-clear"
              disabled={busy}
              onClick={onRequestClear}
              className="mr-auto rounded border border-destructive/30 px-3 py-1.5 text-xs text-destructive hover:bg-destructive/10 disabled:opacity-50"
            >{t("删除取值")}</button>
          )}
          <button type="button" onClick={onClose} className="rounded border px-3 py-1.5 text-xs text-muted-foreground hover:bg-accent">{t("取消")}</button>
          <button
            type="button"
            data-testid="secret-value-save"
            disabled={busy || value.length === 0}
            onClick={() => onSubmit(value)}
            className="rounded bg-primary px-3 py-1.5 text-xs font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-60"
          >{t("保存")}</button>
        </div>
      </div>
    </div>
  );
}
