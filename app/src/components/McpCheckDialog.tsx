import { CheckCircle2, XCircle } from "lucide-react";
import { useState } from "react";
import type { EngineElement } from "../lib/contract";
import { engineErrorMessage, runEngine } from "../lib/engine";
import type { McpAgentCell, McpObserved } from "../lib/observed";
import { cn } from "../lib/utils";
import { ConfirmModal } from "./ConfirmModal";

/** `mcp:check` 的结果。 */
export interface McpCheckResult {
  ok: boolean;
  stage: "variables" | "start" | "initialize" | "tools" | "done";
  message: string;
  server_info: { name?: string; version?: string; title?: string } | null;
  tool_count: number | null;
  tools: string[] | null;
  detail: string | null;
  elapsed_ms: number;
}

/** 能测的账号：文件里实际有、结构认得出的条目。 */
export function checkableAgents(element: EngineElement): string[] {
  const cells = (element.observed as McpObserved).agents ?? {};
  return Object.entries(cells)
    .filter(([, cell]: [string, McpAgentCell]) => cell.source === "file" && cell.state !== "invalid")
    .sort(([, a], [, b]) => Number(b.state === "enabled") - Number(a.state === "enabled"))
    .map(([agent]) => agent);
}

/**
 * 测试连接：像助手那样连一次某个账号里的这条 MCP，看它能不能起来、给出哪些工具。
 * 会运行条目里的命令（npx 一类可能先下载）或请求它的地址；不改任何配置，结果不保存。
 */
export function McpCheckDialog({
  element,
  nameOf,
  onClose,
}: {
  element: EngineElement;
  nameOf: (agent: string) => string;
  onClose: () => void;
}) {
  const agents = checkableAgents(element);
  const [agent, setAgent] = useState(agents[0] ?? "");
  const [running, setRunning] = useState(false);
  const [result, setResult] = useState<McpCheckResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const name = element.id.replace(/^mcp:/, "");

  const run = async () => {
    setRunning(true);
    setResult(null);
    setError(null);
    try {
      const envelope = await runEngine<McpCheckResult>("mcp:check", ["--only", element.id, "--agent", agent]);
      if (envelope.ok) setResult(envelope.data);
      else setError(envelope.errors[0]?.message ?? "测试失败");
    } catch (reason) {
      setError(engineErrorMessage(reason));
    } finally {
      setRunning(false);
    }
  };

  return (
    <ConfirmModal
      title={`测试连接 · ${name}`}
      description="像助手那样启动一次这条配置（命令型会运行它的命令，npx 一类可能先下载；地址型会请求它的地址），看能不能连上、提供哪些工具。不改任何配置，结果不保存。"
      risk="exec-local"
      confirmLabel={running ? "测试中…" : result || error ? "再测一次" : "测试"}
      confirmDisabled={!agent}
      executing={running}
      onConfirm={() => void run()}
      onClose={onClose}
    >
      {agents.length > 1 && (
        <div className="mt-3">
          <p className="mb-1.5 text-xs text-muted-foreground">用哪个账号里的配置</p>
          <div role="radiogroup" aria-label="账号" className="flex flex-wrap gap-1.5">
            {agents.map((item) => (
              <button
                key={item}
                type="button"
                role="radio"
                aria-checked={agent === item}
                data-testid={`mcp-check-agent-${item}`}
                disabled={running}
                onClick={() => setAgent(item)}
                className={cn(
                  "rounded-md border px-2.5 py-1 text-xs",
                  agent === item ? "border-primary bg-primary/10 text-primary" : "text-muted-foreground hover:bg-accent",
                )}
              >
                {nameOf(item)}
              </button>
            ))}
          </div>
        </div>
      )}
      {agents.length === 0 && <p className="mt-3 text-xs text-warning">没有账号里有能识别的这条配置，测不了。</p>}
      {error && <p role="alert" className="mt-3 text-sm text-destructive">{error}</p>}
      {result && (
        <div data-testid="mcp-check-result" className={cn("mt-3 rounded-md border p-3 text-sm", result.ok ? "border-success/30 bg-success/5" : "border-destructive/30 bg-destructive/5")}>
          <div className="flex items-center gap-2">
            {result.ok ? <CheckCircle2 className="h-4 w-4 text-success" aria-hidden /> : <XCircle className="h-4 w-4 text-destructive" aria-hidden />}
            <span className="font-medium">{result.message}</span>
            <span className="ml-auto text-xs text-muted-foreground">{(result.elapsed_ms / 1000).toFixed(1)} 秒</span>
          </div>
          {result.server_info?.name && (
            <p className="mt-1.5 text-xs text-muted-foreground">
              服务：{result.server_info.title ?? result.server_info.name}
              {result.server_info.version ? ` ${result.server_info.version}` : ""}
            </p>
          )}
          {result.tools && result.tools.length > 0 && (
            <p className="mt-1.5 break-words font-mono text-xs text-muted-foreground">
              {result.tools.join("、")}
              {result.tool_count !== null && result.tool_count > result.tools.length ? ` 等 ${result.tool_count} 个` : ""}
            </p>
          )}
          {result.detail && (
            <pre className="mt-2 max-h-40 overflow-auto whitespace-pre-wrap break-words rounded bg-muted p-2 font-mono text-[11px] text-muted-foreground">{result.detail}</pre>
          )}
        </div>
      )}
    </ConfirmModal>
  );
}
