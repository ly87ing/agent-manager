import { useEffect, useState, type ReactNode } from "react";
import type { AliasFilePreview } from "../lib/useAliasSource";
import { ConfirmModal } from "./ConfirmModal";

/** 别名写入确认：按当前文件基准只读计算候选原文，预览成功后才允许执行。 */
export function AliasApplyConfirm({
  title,
  description,
  confirmLabel,
  previewBase,
  op,
  onPreview,
  executing,
  onConfirm,
  onClose,
  children,
}: {
  title: string;
  description: string;
  confirmLabel: string;
  previewBase: string;
  op: string[];
  onPreview: (op: string[]) => Promise<AliasFilePreview | string>;
  executing: boolean;
  onConfirm: () => void;
  onClose: () => void;
  children?: ReactNode;
}) {
  const previewKey = JSON.stringify([previewBase, op]);
  const [preview, setPreview] = useState<{ key: string; result: AliasFilePreview | string } | null>(null);

  useEffect(() => {
    let current = true;
    void onPreview(op)
      .then((result) => { if (current) setPreview({ key: previewKey, result }); })
      .catch((error) => { if (current) setPreview({ key: previewKey, result: String(error) }); });
    return () => { current = false; };
  }, [previewKey, onPreview]);

  const result = preview?.key === previewKey ? preview.result : null;
  return (
    <ConfirmModal
      title={title}
      description={description}
      risk="write-local"
      confirmLabel={confirmLabel}
      confirmDisabled={!result || typeof result === "string"}
      executing={executing}
      onConfirm={onConfirm}
      onClose={onClose}
    >
      {children}
      <section className="mt-3 max-h-80 space-y-2 overflow-y-auto rounded border p-3 text-xs" data-testid="alias-confirm-file-preview">
        <p className="font-medium">终端配置文件写入后</p>
        {result === null && <p className="text-muted-foreground">正在读取实际文件并计算…</p>}
        {typeof result === "string" && <p role="alert" className="text-destructive">{result}</p>}
        {result && typeof result !== "string" && <>
          {result.files.length === 0 && <p className="text-muted-foreground">配置文件不变。</p>}
          {result.files.map((file) => <div key={file.target} className="space-y-1">
            <p className="break-all font-mono">{file.path}{file.exists ? "（已有文件）" : "（将新建）"}</p>
            <pre className="max-h-48 overflow-auto whitespace-pre-wrap break-all rounded bg-muted p-2 font-mono" data-testid={`alias-confirm-file-${file.target}`}>{file.content}</pre>
          </div>)}
          {result.skipped.length > 0 && <p className="text-muted-foreground">未变化：{result.skipped.join("、")}</p>}
        </>}
        <p className="text-muted-foreground">只读预览；原文可能含敏感值，仅在本机查看。</p>
      </section>
    </ConfirmModal>
  );
}
