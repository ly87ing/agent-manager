import { useState } from "react";
import { X } from "lucide-react";

/**
 * 填写一个凭据变量的取值。
 *
 * 只写不读：界面任何地方都不回显已保存的取值，要改只能覆写。留空保存即清除，
 * 清除后该变量回落到进程环境（若那里有的话）。
 */
export function SecretValueDialog({
  variable,
  stored,
  busy,
  onSubmit,
  onClose,
}: {
  variable: string;
  /** 该变量在本机是否已经填过——决定说明文案与是否给「清除」。 */
  stored: boolean;
  busy: boolean;
  onSubmit: (value: string) => void;
  onClose: () => void;
}) {
  const [value, setValue] = useState("");

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-slate-900/40 p-4 animate-fade-in">
      <div role="dialog" aria-modal="true" aria-label={`填写 ${variable}`} className="w-full max-w-md rounded-lg border bg-card p-5 shadow-lg">
        <div className="flex items-start justify-between gap-3">
          <div className="min-w-0">
            <h3 className="text-base font-semibold text-foreground">填写取值</h3>
            <p className="mt-0.5 break-all font-mono text-xs text-muted-foreground">{variable}</p>
          </div>
          <button type="button" onClick={onClose} aria-label="关闭" className="rounded p-1 text-muted-foreground hover:bg-accent">
            <X className="h-4 w-4" aria-hidden />
          </button>
        </div>

        <label className="mt-4 block text-xs font-medium text-muted-foreground" htmlFor="secret-value-input">
          取值
        </label>
        <input
          id="secret-value-input"
          type="password"
          value={value}
          autoComplete="off"
          data-testid="secret-value-input"
          onChange={(event) => setValue(event.target.value)}
          placeholder={stored ? "留空保存即清除已填的取值" : "粘贴取值"}
          className="mt-1 w-full rounded border border-input px-2 py-1.5 font-mono text-xs"
        />

        <p className="mt-3 rounded-md border bg-muted px-3 py-2 text-[11px] leading-relaxed text-muted-foreground">
          取值保存在本机数据库里，界面不回显，也不进操作记录、诊断日志与配置备份。
          它会在同步时写进对应助手的配置文件——那份文件里是明文，与你手工填进去的一样。
        </p>

        <div className="mt-4 flex justify-end gap-2">
          <button type="button" onClick={onClose} className="rounded border px-3 py-1.5 text-xs text-muted-foreground hover:bg-accent">
            取消
          </button>
          <button
            type="button"
            data-testid="secret-value-save"
            disabled={busy}
            onClick={() => onSubmit(value)}
            className="rounded bg-primary px-3 py-1.5 text-xs font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-60"
          >
            保存
          </button>
        </div>
      </div>
    </div>
  );
}
