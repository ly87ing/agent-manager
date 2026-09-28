import { useEffect, useState } from "react";
import type { EngineElement } from "../lib/contract";
import { runEngine } from "../lib/engine";
import { ConfirmModal } from "./ConfirmModal";

type ProfileFilePreview = {
  path: string;
  content: string;
  exists: boolean;
  file_base: string;
  content_base: string;
};

export function ProfileApplyConfirm({ element, base, description, credentialOnly, executing, onConfirm, onClose, onRefresh }: {
  element: EngineElement;
  base: string;
  description: string;
  credentialOnly: boolean;
  executing: boolean;
  onConfirm: (fileBase: string, contentBase: string) => void;
  onClose: () => void;
  onRefresh: () => void;
}) {
  const [preview, setPreview] = useState<ProfileFilePreview | string | null>(null);
  const name = element.id.replace(/^profiles:/, "");
  const title = element.state === "drifted" && !credentialOnly
    ? `用定义覆盖 ${name} 的实际配置`
    : credentialOnly ? `写入 ${name} 的新密钥` : `写入 ${name} 的配置`;

  useEffect(() => {
    let current = true;
    setPreview(null);
    void runEngine<ProfileFilePreview>("profiles:preview", ["--only", element.id, "--base", base])
      .then((result) => {
        if (current) setPreview(result.ok && result.data?.file_base && result.data?.content_base
          ? result.data : result.errors[0]?.message ?? "预览失败");
      })
      .catch((error) => { if (current) setPreview(`预览失败：${String(error)}`); });
    return () => { current = false; };
  }, [element.id, base]);

  return <ConfirmModal
    title={title}
    description={description}
    risk="write-local"
    confirmLabel={credentialOnly ? "确认写入新密钥" : element.state === "drifted" ? "确认覆盖" : "确认写入"}
    confirmDisabled={preview === null || typeof preview === "string"}
    executing={executing}
    onConfirm={() => { if (preview && typeof preview !== "string") onConfirm(preview.file_base, preview.content_base); }}
    onClose={onClose}
  >
    <section data-testid="profile-apply-file-preview" className="mt-3 max-h-80 space-y-2 overflow-auto rounded border p-3 text-xs">
      <p className="font-medium">实例配置文件写入后</p>
      {preview === null && <p className="text-muted-foreground">正在读取实际文件并计算…</p>}
      {typeof preview === "string" && <>
        <p role="alert" className="text-destructive">{preview}</p>
        <button type="button" onClick={onRefresh} className="rounded border px-2 py-1">关闭并重新读取</button>
      </>}
      {preview && typeof preview !== "string" && <>
        <p className="break-all font-mono text-muted-foreground">{preview.path} · {preview.exists ? "合并现有文件" : "新文件"}</p>
        <pre className="whitespace-pre-wrap break-all rounded bg-muted p-2 font-mono">{preview.content}</pre>
        <p className="text-muted-foreground">内容可能含凭据，仅在本机查看。确认时会重新校验原文件和候选内容。</p>
      </>}
    </section>
  </ConfirmModal>;
}
