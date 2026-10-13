import { t } from "../lib/i18n";
import { useEffect, useState } from "react";
import { engineErrorMessage, runEngine } from "../lib/engine";
import { ConfirmModal } from "./ConfirmModal";
import { FileDiff } from "./FileDiff";

/** Preview and apply exactly one entry, using the same digest for both. */
export function McpMarkerCleanupDialog({ id, agent, base, nameOf, busy, onConfirm, onClose }: {
  id: string;
  agent: string;
  base: string;
  nameOf: (agent: string) => string;
  busy: boolean;
  onConfirm: (args: string[]) => void;
  onClose: () => void;
}) {
  const [preview, setPreview] = useState<{ path: string; original?: string | null; content: string }[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const args = ["--only", id, "--agent", agent, "--action", "clean-markers", "--base", base];
  useEffect(() => {
    let current = true;
    void runEngine<{ files: NonNullable<typeof preview> }>("mcp:preview", args).then((envelope) => {
      if (!current) return;
      if (envelope.ok) setPreview(envelope.data.files);
      else setError(envelope.errors[0]?.message ?? t("预览失败"));
    }).catch((reason) => { if (current) setError(engineErrorMessage(reason)); });
    return () => { current = false; };
  }, [id, agent, base]);
  return (
    <ConfirmModal title={t("清理旧标记 · {0}", id.replace(/^mcp:/, ""))} description={t("只移除 {0} 中这条服务及其环境变量里的 _MANAGED_BY 字段。保留启用状态、命令、取值和其他字段；不会修复服务自身的启动错误。预览可能包含配置原文，请勿分享。", nameOf(agent))} risk="write-local" confirmLabel={t("确认清理")} confirmDisabled={!preview?.length || Boolean(error)} executing={busy} onConfirm={() => onConfirm(args)} onClose={onClose}>
      {error ? <p role="alert" className="mt-3 text-sm text-destructive">{error}</p> : !preview ? <p className="mt-3 text-xs text-muted-foreground">{t("读取预览…")}</p> : <div className="mt-3 space-y-3">{preview.map((file) => <FileDiff key={file.path} path={file.path} original={file.original} content={file.content} testId="mcp-marker-preview" />)}</div>}
    </ConfirmModal>
  );
}
