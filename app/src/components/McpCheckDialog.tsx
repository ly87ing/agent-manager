import { t } from "../lib/i18n";
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
  base: string;
  cwd: string | null;
  cwd_source: "config" | "selected" | "home" | null;
  startup_timeout_sec: number;
  tool_timeout_sec: number;
  exit_code: number | null;
}

export const CHECK_STAGE = { get variables() { return t("变量取值"); }, get start() { return t("启动命令"); }, get initialize() { return t("初始化"); }, get tools() { return t("读取工具"); }, get done() { return t("完成"); } };

/** 能测的助手：文件里实际有、结构认得出的条目。 */
export function checkableAgents(element: EngineElement): string[] {
  const cells = (element.observed as McpObserved).agents ?? {};
  return Object.entries(cells)
    .filter(([, cell]: [string, McpAgentCell]) => cell.source === "file" && cell.state !== "invalid")
    .sort(([, a], [, b]) => Number(b.state === "enabled") - Number(a.state === "enabled"))
    .map(([agent]) => agent);
}

/**
 * 测试连接：像助手那样连一次某个助手里的这条 MCP，看它能不能起来、给出哪些工具。
 * 会运行条目里的命令（npx 一类可能先下载）或请求它的地址；不改任何配置，结果不保存。
 */
export function McpCheckDialog({
  element,
  nameOf,
  onClose,
  initialAgent,
  initialResult,
  onResult,
  onEdit,
}: {
  element: EngineElement;
  nameOf: (agent: string) => string;
  onClose: () => void;
  initialAgent?: string;
  initialResult?: McpCheckResult;
  onResult: (agent: string, result: McpCheckResult | null) => void;
  onEdit: (agent: string) => void;
}) {
  const agents = checkableAgents(element);
  const [agent, setAgent] = useState(initialAgent ?? agents[0] ?? "");
  const [running, setRunning] = useState(false);
  const [result, setResult] = useState<McpCheckResult | null>(initialResult ?? null);
  const [cwd, setCwd] = useState(initialResult?.cwd_source === "selected" ? initialResult.cwd ?? "" : "");
  const [error, setError] = useState<string | null>(null);
  const name = element.id.replace(/^mcp:/, "");
  const cell = (element.observed as McpObserved).agents?.[agent];
  const configuredCwd = typeof cell?.extra?.cwd === "string" ? cell.extra.cwd : undefined;

  const run = async () => {
    if (!cell) return;
    setRunning(true);
    setResult(null);
    setError(null);
    onResult(agent, null);
    try {
      const envelope = await runEngine<McpCheckResult>("mcp:check", ["--only", element.id, "--agent", agent, "--base", cell.base, ...(cwd.trim() && !configuredCwd ? ["--cwd", cwd.trim()] : [])]);
      if (envelope.ok) {
        setResult(envelope.data);
        onResult(agent, envelope.data);
      }
      else setError(envelope.errors[0]?.message ?? t("测试失败"));
    } catch (reason) {
      setError(engineErrorMessage(reason));
    } finally {
      setRunning(false);
    }
  };

  return (
    <ConfirmModal
      title={t("测试连接 · {0}", name)}
      description={t("按所选助手的实际配置检查初始化和工具读取。命令型会运行命令，npx 一类可能下载依赖；地址型会发请求。不改任何配置，结果仅在当前页面保留；通过也不代表助手已经加载。")}
      risk="exec-local"
      confirmLabel={running ? t("测试中…") : result || error ? t("再测一次") : t("测试")}
      confirmDisabled={!agent}
      executing={running}
      onConfirm={() => void run()}
      onClose={onClose}
    >
      {agents.length > 1 && (
        <div className="mt-3">
          <p className="mb-1.5 text-xs text-muted-foreground">{t("用哪个助手里的配置")}</p>
          <div role="radiogroup" aria-label={t("助手")} className="flex flex-wrap gap-1.5">
            {agents.map((item) => (
              <button
                key={item}
                type="button"
                role="radio"
                aria-checked={agent === item}
                data-testid={`mcp-check-agent-${item}`}
                disabled={running}
                onClick={() => { setAgent(item); setResult(null); setError(null); setCwd(""); }}
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
      {cell && (
        <div className="mt-3 space-y-2 text-xs text-muted-foreground">
          <p>{t("助手：")}{nameOf(agent)}</p>
          <p className="break-all font-mono">{t("配置：")}{cell.target}</p>
          <p className="break-all font-mono">{cell.entry?.url ?? [cell.entry?.command, ...(cell.entry?.args ?? [])].join(" ")}</p>
          {cell.transport === "stdio" && (
            <label className="block space-y-1">
              <span>{t("工作目录")}{configuredCwd ? t("（来自配置）") : t("（留空使用本工具的 HOME）")}</span>
              <input data-testid="mcp-check-cwd" value={configuredCwd ?? cwd} disabled={running || Boolean(configuredCwd)} onChange={(event) => { setCwd(event.target.value); setResult(null); setError(null); }} placeholder={t("项目目录的绝对路径")} className="w-full rounded border bg-background px-2 py-1.5 font-mono text-xs" />
            </label>
          )}
          <p>{t("取值使用本工具的密钥文件及运行环境，可能与助手进程不同。")}</p>
        </div>
      )}
      {agents.length === 0 && <p className="mt-3 text-xs text-warning">{t("没有助手里有能识别的这条配置，测不了。")}</p>}
      {error && <p role="alert" className="mt-3 text-sm text-destructive">{error}</p>}
      {result && (
        <div data-testid="mcp-check-result" className={cn("mt-3 rounded-md border p-3 text-sm", result.ok ? "border-success/30 bg-success/5" : "border-destructive/30 bg-destructive/5")}>
          <div className="flex items-start gap-2">
            {result.ok ? <CheckCircle2 className="h-4 w-4 shrink-0 text-success" aria-hidden /> : <XCircle className="h-4 w-4 shrink-0 text-destructive" aria-hidden />}
            <span className="min-w-0 break-words font-medium">{result.message}</span>
            <span className="ml-auto shrink-0 text-xs text-muted-foreground">{(result.elapsed_ms / 1000).toFixed(1)}{(" " + t("秒"))}</span>
          </div>
          <p className="mt-1.5 text-xs text-muted-foreground">{t("阶段：")}{CHECK_STAGE[result.stage]}{result.exit_code != null ? (" " + t("· 退出码：{0}", result.exit_code)) : ""}</p>
          <p className="mt-1 text-xs text-muted-foreground">{(t("超时：初始化") + " ")}{result.startup_timeout_sec}{(" " + t("秒 · 工具") + " ")}{result.tool_timeout_sec}{(" " + t("秒"))}</p>
          {result.cwd && <p className="mt-1 break-all font-mono text-xs text-muted-foreground">{t("工作目录：")}{result.cwd}{result.cwd_source === "home" ? "（HOME）" : ""}</p>}
          {result.server_info?.name && (
            <p className="mt-1.5 text-xs text-muted-foreground">{t("服务：")}{result.server_info.title ?? result.server_info.name}
              {result.server_info.version ? ` ${result.server_info.version}` : ""}
            </p>
          )}
          {result.tools && result.tools.length > 0 && (
            <p className="mt-1.5 break-words font-mono text-xs text-muted-foreground">
              {result.tools.join("、")}
              {result.tool_count !== null && result.tool_count > result.tools.length ? (" " + t("等 {0} 个", result.tool_count)) : ""}
            </p>
          )}
          {result.detail && (
            <details className="mt-2" open={!result.ok}>
              <summary className="cursor-pointer text-xs text-muted-foreground">{t("错误输出（已脱敏）")}</summary>
              <pre className="mt-1 max-h-40 overflow-auto whitespace-pre-wrap break-all rounded bg-muted p-2 font-mono text-[11px] text-muted-foreground">{result.detail}</pre>
            </details>
          )}
          {!result.ok && <button type="button" onClick={() => onEdit(agent)} className="mt-2 text-xs text-primary hover:underline">{t("编辑服务配置")}</button>}
        </div>
      )}
    </ConfirmModal>
  );
}
