import { useEffect, useState } from "react";
import type { McpAgentCell } from "../lib/observed";
import { ConfirmModal } from "./ConfirmModal";

export type McpRemovePreview = { files: { path: string; exists: boolean; content: string }[]; warnings: string[] } | string;

/**
 * 删除一个服务：列出有它的账号，默认全选，可以取消某个账号；每个账号给出删除后的配置文件预览。
 * 预览全部取回且没有出错才能确认。确认后由调用方逐个账号执行，各自带写入基准。
 */
export function McpRemoveDialog({
  name,
  entries,
  busy,
  nameOf,
  onPreview,
  onConfirm,
  onClose,
}: {
  name: string;
  entries: { agent: string; cell: McpAgentCell }[];
  busy: boolean;
  nameOf: (agent: string) => string;
  onPreview: (agent: string, cell: McpAgentCell) => Promise<McpRemovePreview>;
  onConfirm: (agents: string[]) => void;
  onClose: () => void;
}) {
  const [selected, setSelected] = useState<string[]>(entries.map((entry) => entry.agent));
  const [previews, setPreviews] = useState<Record<string, McpRemovePreview>>({});

  useEffect(() => {
    let current = true;
    for (const { agent, cell } of entries) {
      void onPreview(agent, cell).then((result) => {
        if (current) setPreviews((prev) => ({ ...prev, [agent]: result }));
      });
    }
    return () => {
      current = false;
    };
    // 打开时按当时的条目取一次预览
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const ready = selected.length > 0 && selected.every((agent) => previews[agent] !== undefined && typeof previews[agent] !== "string");
  const savedOnly = (cell: McpAgentCell) => cell.source === "saved";

  return (
    <ConfirmModal
      title={`删除 ${name}`}
      description="从勾选的账号里删掉这一条，其他条目与配置保持不变；停用时保存在本工具里的内容一并删除。确认后逐个账号执行。"
      risk="write-local"
      confirmLabel={selected.length ? `从 ${selected.length} 个账号删除` : "确认删除"}
      confirmDisabled={!ready}
      executing={busy}
      onConfirm={() => onConfirm(selected)}
      onClose={onClose}
    >
      <section className="mt-3 space-y-2 text-xs" data-testid="mcp-remove-preview">
        {entries.map(({ agent, cell }) => {
          const preview = previews[agent];
          const checked = selected.includes(agent);
          return (
            <div key={agent} className="rounded border p-2">
              <label className="flex items-center gap-2 font-medium">
                <input
                  type="checkbox"
                  checked={checked}
                  data-testid={`mcp-remove-agent-${agent}`}
                  onChange={() => setSelected((prev) => (checked ? prev.filter((item) => item !== agent) : [...prev, agent]))}
                />
                {nameOf(agent)}
                <span className="font-normal text-muted-foreground">{savedOnly(cell) ? "（已停用，只删本工具保存的内容）" : ""}</span>
              </label>
              {checked && (
                <div className="mt-1.5 space-y-1 pl-5">
                  {preview === undefined && <p className="text-muted-foreground">正在读取实际文件并计算…</p>}
                  {typeof preview === "string" && <p role="alert" className="text-destructive">{preview}</p>}
                  {preview !== undefined && typeof preview !== "string" && (
                    <>
                      {preview.files.length === 0 && <p className="text-muted-foreground">配置文件不变。</p>}
                      {preview.files.map((file) => (
                        <details key={file.path}>
                          <summary className="cursor-pointer break-all font-mono text-muted-foreground">删除后的 {file.path}</summary>
                          <pre className="mt-1 max-h-60 overflow-auto whitespace-pre-wrap break-all rounded bg-muted p-2 font-mono" data-testid="mcp-remove-preview-content">{file.content}</pre>
                        </details>
                      ))}
                      {preview.warnings.map((warning) => <p key={warning} className="text-warning">{warning}</p>)}
                    </>
                  )}
                </div>
              )}
            </div>
          );
        })}
        <p className="text-muted-foreground">只读预览；文件原文可能含敏感值，仅在本机查看。</p>
      </section>
    </ConfirmModal>
  );
}
