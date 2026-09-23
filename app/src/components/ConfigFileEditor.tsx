import { useEffect, useState } from "react";
import { runEngine } from "../lib/engine";
import { useExecutions } from "../lib/execution";
import { diffLines, hasChanges } from "../lib/textDiff";
import { cn } from "../lib/utils";

/** agents:config-file 的返回：配置文件原文与整文件写入基准。 */
export interface AgentConfigFile {
  element: string;
  path: string;
  format: "json" | "toml" | string;
  exists: boolean;
  content: string | null;
  base: string;
  blocked_reason: string | null;
}

/**
 * 直接编辑助手的实际配置文件全文（Claude settings.json / Codex config.toml），显示与写回的都是原文。
 *
 * 表单只覆盖少量常用参数，其余设置在这里改：写入前可查看改动，按打开时的文件摘要写回，
 * 文件在别处被改过就拒绝并重新读取。
 */
export function ConfigFileEditor({ agentId }: { agentId: string }) {
  const { execute } = useExecutions();
  const [file, setFile] = useState<AgentConfigFile | null>(null);
  const [text, setText] = useState("");
  const [mode, setMode] = useState<"edit" | "diff">("edit");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

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
    // 只在打开时读取一次；写入后或被拒后由 load 显式刷新
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
        await load();
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
    <div data-testid="config-file-editor" className="space-y-3 px-3 pb-3">
      <p className="text-xs text-muted-foreground">
        这里是文件原文。表单没覆盖的设置在这里改；写入前会核对文件没被别处改过（包括助手自己改），被改过就不写。
      </p>
      {!file && !error && <p className="text-sm text-muted-foreground">正在读取…</p>}
      {file?.blocked_reason && (
        <p data-testid="config-file-blocked" className="rounded border border-warning/30 bg-warning/10 px-3 py-2 text-sm text-warning">
          {file.blocked_reason}
        </p>
      )}
      {file && !file.blocked_reason && (
        <>
          <div className="flex flex-wrap items-center gap-2" role="group" aria-label="查看方式">
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
                className={cn("rounded-md border px-3 py-1 text-xs", mode === id ? "border-primary bg-primary/10 text-primary" : "text-muted-foreground hover:bg-accent")}
              >
                {label}
              </button>
            ))}
          </div>
          {mode === "edit" ? (
            <textarea
              data-testid="config-file-text"
              aria-label="配置文件内容"
              spellCheck={false}
              value={text}
              placeholder={file.format === "json" ? "{\n  \n}" : ""}
              onChange={(event) => setText(event.target.value)}
              className="h-80 w-full resize-y rounded-md border bg-background p-3 font-mono text-xs leading-relaxed text-foreground"
            />
          ) : (
            <pre data-testid="config-file-diff" className="max-h-80 overflow-auto rounded border p-2 font-mono text-xs whitespace-pre-wrap">
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
        </>
      )}
      {error && (
        <p role="alert" data-testid="config-file-error" className="text-sm text-destructive">
          {error}
        </p>
      )}
      <div className="flex justify-end gap-2">
        <button
          type="button"
          data-testid="config-file-reset"
          disabled={busy || !changed}
          onClick={() => {
            setText(original);
            setError(null);
          }}
          className="rounded-md border px-3 py-1.5 text-sm text-muted-foreground hover:bg-accent disabled:opacity-50"
        >
          放弃改动
        </button>
        <button
          type="button"
          data-testid="config-file-submit"
          disabled={busy || !file || Boolean(file.blocked_reason) || !changed}
          onClick={() => void submit()}
          className="rounded-md bg-primary px-4 py-1.5 text-sm font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
        >
          {busy ? "写入中…" : "写入配置文件"}
        </button>
      </div>
    </div>
  );
}
