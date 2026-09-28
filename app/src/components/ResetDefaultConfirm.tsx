import { useEffect, useState } from "react";
import type { EngineElement } from "../lib/contract";
import { runEngine } from "../lib/engine";
import { connectionKindOf } from "../lib/agentManagement";
import { configureBaseOf } from "../lib/agentCurrent";
import { switchBackEffects } from "../lib/defaultEnvironment";
import type { AgentsSpec } from "../lib/observed";
import { ConfirmModal } from "./ConfirmModal";

type DefaultFilePreview = { path: string; content: string; exists: boolean; will_write: boolean; file_base: string };

export function ResetDefaultConfirm({
  element,
  executing,
  onConfirm,
  onClose,
  onRefresh,
}: {
  element: EngineElement;
  executing: boolean;
  onConfirm: (fileBase: string) => void;
  onClose: () => void;
  onRefresh: () => void;
}) {
  const [preview, setPreview] = useState<DefaultFilePreview | string | null>(null);
  const agentId = element.id.replace(/^agents:/, "");
  const base = configureBaseOf(element);

  useEffect(() => {
    let current = true;
    setPreview(null);
    if (!base) {
      setPreview("客户端配置文件暂不可读，请重新读取后再试。");
      return () => { current = false; };
    }
    void runEngine<DefaultFilePreview>("agents:preview-default", ["--only", element.id, "--base", base])
      .then((result) => {
        if (current) setPreview(result.ok && result.data?.file_base
          ? result.data : result.errors[0]?.message ?? "预览失败");
      })
      .catch((error) => {
        if (current) setPreview(`预览失败：${String(error)}`);
      });
    return () => { current = false; };
  }, [element.id, base]);

  const effects = switchBackEffects(connectionKindOf(element) ?? "claude");
  return (
    <ConfirmModal
      title={`把 ${(element.spec as AgentsSpec).display_name ?? agentId} 的主配置切回官方账号`}
      description="主配置改回只用官方账号。改动范围如下："
      risk="write-local"
      riskLabel="切回官方账号"
      confirmLabel="确认切回"
      confirmDisabled={preview === null || typeof preview === "string"}
      executing={executing}
      onConfirm={() => {
        if (preview && typeof preview !== "string") onConfirm(preview.file_base);
      }}
      onClose={onClose}
    >
      <dl data-testid="switch-back-effects" className="space-y-2 text-sm">
        <div><dt className="font-medium text-foreground">会撤掉</dt><dd className="text-muted-foreground">{effects.removes}</dd></div>
        <div><dt className="font-medium text-foreground">切之前</dt><dd className="text-muted-foreground">{effects.before}</dd></div>
        <div><dt className="font-medium text-foreground">不会动</dt><dd className="text-muted-foreground">{effects.keeps.join("；")}</dd></div>
        <div><dt className="font-medium text-foreground">之后</dt><dd className="text-muted-foreground">{effects.after}</dd></div>
      </dl>
      <section data-testid="switch-back-file-preview" className="mt-3 max-h-80 space-y-2 overflow-auto rounded border p-3 text-xs">
        <p className="font-medium">客户端配置文件写入后</p>
        {preview === null && <p className="text-muted-foreground">正在读取实际文件并计算…</p>}
        {typeof preview === "string" && <>
          <p role="alert" className="text-destructive">{preview}</p>
          <button type="button" onClick={onRefresh} className="rounded border px-2 py-1">关闭并重新读取</button>
        </>}
        {preview && typeof preview !== "string" && <>
          <p className="break-all font-mono text-muted-foreground">{preview.path} · {preview.will_write ? (preview.exists ? "合并现有文件" : "新文件") : "文件不变"}</p>
          <pre className="whitespace-pre-wrap break-all rounded bg-muted p-2 font-mono">{preview.content}</pre>
          <p className="text-muted-foreground">原文可能含凭据，仅在本机查看。确认时仍会重新校验；未保存的第三方接入须先安全回填。</p>
        </>}
      </section>
    </ConfirmModal>
  );
}
