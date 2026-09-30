import { useEffect, useState, type ReactNode } from "react";
import type { AliasFilePreview } from "../lib/useAliasSource";
import { FileDiff } from "./FileDiff";
import { ConfirmModal } from "./ConfirmModal";

/** 别名写入确认：按当前文件基准只读计算候选原文，预览成功后才允许执行。 */
export function AliasApplyConfirm({
  title,
  description,
  confirmLabel,
  previewBase,
  op,
  onPreview,
  onRefresh,
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
  onRefresh: () => void;
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
        <p className="font-medium">终端配置文件的改动</p>
        {result === null && <p className="text-muted-foreground">正在读取实际文件并计算…</p>}
        {typeof result === "string" && <>
          <p role="alert" className="text-destructive">{result}</p>
          <button type="button" onClick={onRefresh} className="rounded border px-2 py-1">关闭并重新读取</button>
        </>}
        {result && typeof result !== "string" && <>
          {result.files.length === 0 && <p className="text-muted-foreground">配置文件不变。</p>}
          {result.files.map((file) => (
            <FileDiff
              key={file.target}
              path={file.path}
              original={file.exists ? file.original : null}
              content={file.content}
              testId={`alias-confirm-file-${file.target}`}
              maxHeight="max-h-48"
            />
          ))}
          {result.skipped.length > 0 && <p className="text-muted-foreground">未变化：{result.skipped.join("、")}</p>}
        </>}
        <p className="text-muted-foreground">只读预览，只显示变化的部分。</p>
      </section>
    </ConfirmModal>
  );
}
