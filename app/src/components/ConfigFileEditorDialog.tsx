import { useEffect, useState } from "react";
import { X } from "lucide-react";
import { runEngine } from "../lib/engine";
import { useDialogFocus } from "../lib/dialogFocus";
import { useExecutions } from "../lib/execution";
import { diffLines, hasChanges } from "../lib/textDiff";
import { cn } from "../lib/utils";

/** agents:config-file 的返回：配置文件全文（凭据已遮住）与整文件写入基准。 */
export interface AgentConfigFile {
  element: string;
  path: string;
  format: "json" | "toml" | string;
  exists: boolean;
  content: string | null;
  hidden: number;
  base: string;
  blocked_reason: string | null;
}

/**
 * 直接编辑助手的实际配置文件全文（Claude settings.json / Codex config.toml）。
 *
 * 表单只覆盖少量常用参数，其余设置在这里改：写入前展示改动，按打开时的文件摘要写回，
 * 文件在别处被改过就拒绝。凭据取值显示为 <hidden>，保持原样即原值保留。
 */
export function ConfigFileEditorDialog({
  agentId,
  agentName,
  onClose,
}: {
  agentId: string;
  agentName: string;
  onClose: () => void;
}) {
  const { execute } = useExecutions();
  const [file, setFile] = useState<AgentConfigFile | null>(null);
  const [text, setText] = useState("");
  const [mode, setMode] = useState<"edit" | "diff">("edit");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const { dialogRef, onDialogKeyDown } = useDialogFocus<HTMLElement>({
    onEscape: busy ? undefined : onClose,
    initialFocusSelector: '[data-testid="config-file-cancel"]',
  });

  const load = async (notice: string | null = null) => {
    const envelope = await runEngine<AgentConfigFile>("agents:config-file", ["--only", `agents:${agentId}`]);
    if (!envelope.ok) {
      setError(envelope.errors[0]?.message ?? "读取配置文件失败");
      return;
    }
    setFile(envelope.data);
    setText(envelope.data.content ?? "");
    setMode("edit");
    setError(notice);
  };

  useEffect(() => {
    void load();
    // 只在打开时读取一次；写入被拒后由 load 显式刷新
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [agentId]);

  const original = file?.content ?? "";
  const lines = diffLines(original, text);
  const changed = hasChanges(lines);

  const validate = (): string | null => {
    if (file?.format === "json") {
      try {
        const parsed = JSON.parse(text);
        if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) return "内容必须是 JSON 对象";
      } catch {
        return "不是合法的 JSON";
      }
    }
    return null;
  };

  const submit = async () => {
    if (!file || busy) return;
    const invalid = validate();
    if (invalid) {
      setError(invalid);
      return;
    }
    setBusy(true);
    try {
      const envelope = await execute({
        command: "agents:edit-config",
        args: ["--only", `agents:${agentId}`, "--file-base", file.base, "--value", text],
        label: `写入 ${file.path}`,
      });
      if (envelope?.ok) {
        onClose();
      } else if (envelope?.errors[0]?.code === "AGENT_CONFIG_CHANGED") {
        await load("配置文件已在外部修改，已刷新为最新内容；请在此基础上重新修改。");
      } else if (envelope) {
        setError(envelope.errors[0]?.message ?? "写入失败");
      }
    } finally {
      setBusy(false);
    }
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-slate-900/40 p-3 sm:p-6 animate-fade-in"
      onClick={(event) => event.target === event.currentTarget && !busy && onClose()}
    >
      <section
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-label={`编辑 ${agentName} 配置文件`}
        onKeyDown={onDialogKeyDown}
        className="flex max-h-[92vh] w-full max-w-6xl flex-col overflow-hidden rounded-xl border bg-card shadow-xl animate-zoom-in-95"
      >
        <div className="flex items-start justify-between gap-3 border-b px-5 py-4 sm:px-6">
          <div>
            <h3 className="text-lg font-semibold text-foreground">编辑 {agentName} 配置文件</h3>
            <p className="mt-1 text-sm text-muted-foreground">
              直接编辑实际文件
              {file ? <span className="font-mono">（{file.path}）</span> : null}
              ，写入前可查看改动；打开后文件若在别处被改过（包括助手自己改），写入会被拒绝，不会覆盖。
            </p>
          </div>
          <button type="button" aria-label="关闭" disabled={busy} onClick={onClose} className="rounded p-1 text-muted-foreground hover:bg-accent disabled:opacity-50">
            <X className="h-4 w-4" aria-hidden />
          </button>
        </div>

        <div className="flex flex-wrap items-center gap-2 border-b px-5 py-3 sm:px-6" role="group" aria-label="查看方式">
          {(
            [
              ["edit", "编辑"],
              ["diff", "查看改动"],
            ] as const
          ).map(([id, label]) => (
            <button
              key={id}
              type="button"
              aria-pressed={mode === id}
              data-testid={`config-file-mode-${id}`}
              onClick={() => setMode(id)}
              className={cn("rounded-md border px-3 py-1.5 text-sm", mode === id ? "border-primary bg-primary/10 text-primary" : "text-muted-foreground hover:bg-accent")}
            >
              {label}
            </button>
          ))}
          {file && file.hidden > 0 && (
            <span data-testid="config-file-hidden-note" className="text-xs text-muted-foreground">
              {file.hidden} 处凭据显示为 &lt;hidden&gt;：保持原样即保留原值；要改令牌请在接入 Provider 里改。
            </span>
          )}
        </div>

        <div className="min-h-0 flex-1 overflow-auto px-5 py-4 sm:px-6">
          {!file && !error && <p className="text-sm text-muted-foreground">正在读取…</p>}
          {file?.blocked_reason && (
            <p data-testid="config-file-blocked" className="rounded border border-warning/30 bg-warning/10 px-3 py-2 text-sm text-warning">
              {file.blocked_reason}
            </p>
          )}
          {file && !file.blocked_reason && mode === "edit" && (
            <textarea
              data-testid="config-file-text"
              aria-label="配置文件内容"
              spellCheck={false}
              value={text}
              onChange={(event) => setText(event.target.value)}
              className="h-[55vh] w-full resize-none rounded-md border bg-background p-3 font-mono text-xs leading-relaxed text-foreground"
            />
          )}
          {file && !file.blocked_reason && mode === "diff" && (
            <pre data-testid="config-file-diff" className="max-h-[55vh] overflow-auto rounded border p-2 font-mono text-xs whitespace-pre-wrap">
              {changed
                ? lines.map((line, index) => (
                    <div
                      key={index}
                      data-kind={line.kind}
                      className={cn(line.kind === "add" && "bg-success/10 text-success", line.kind === "del" && "bg-destructive/10 text-destructive")}
                    >
                      {line.kind === "add" ? "+ " : line.kind === "del" ? "- " : "  "}
                      {line.text}
                    </div>
                  ))
                : "还没有改动。"}
            </pre>
          )}
          {error && (
            <p role="alert" data-testid="config-file-error" className="mt-3 text-sm text-destructive">
              {error}
            </p>
          )}
        </div>

        <div className="flex items-center justify-end gap-2 border-t px-5 py-4 sm:px-6">
          <button type="button" data-testid="config-file-cancel" disabled={busy} onClick={onClose} className="rounded-lg border px-4 py-2 text-sm hover:bg-accent disabled:opacity-50">
            取消
          </button>
          <button
            type="button"
            data-testid="config-file-submit"
            disabled={busy || !file || Boolean(file.blocked_reason) || !changed}
            onClick={() => void submit()}
            className="rounded-lg bg-primary px-4 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
          >
            {busy ? "写入中…" : "写入配置文件"}
          </button>
        </div>
      </section>
    </div>
  );
}
