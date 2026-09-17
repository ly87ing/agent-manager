import { useState } from "react";
import { X } from "lucide-react";
import type { EngineElement, PolicyLayers } from "../lib/contract";
import { definitionOrigin } from "../lib/definitionOverlay";
import type { McpSpec } from "../lib/observed";
import { OVERLAY_COPY } from "../lib/ui-copy";

type McpTransport = "stdio" | "remote";

/** 远程 url 前端预检（与引擎白名单同语义；引擎为权威校验）：https:// 任意主机，或本机回环 http。 */
export function isValidRemoteUrl(url: string): boolean {
  if (!url || /\s/.test(url)) return false;
  const match = url.match(/^(https?):\/\/([^/?#]*)/);
  if (!match) return false;
  const authority = match[2];
  if (!authority || authority.includes("@")) return false;
  const host = authority.split(":")[0];
  if (match[1] === "https") return host.length > 0;
  return host === "localhost" || host === "127.0.0.1";
}

const HEADER_NAME_RE = /^[!#$%&'*+.^_`|~A-Za-z0-9-]+$/;
const VAR_NAME_RE = /^[A-Za-z_][A-Za-z0-9_]*$/;
const PLACEHOLDER_RE = /^\$\{([A-Za-z_][A-Za-z0-9_]*)\}$/;

/**
 * MCP 服务新增/编辑表单。写入的是定义覆盖层片段（mcp.definitions.<name>）；
 * 编辑内置条目时整段写覆盖片段并注明「内置默认值不受影响」。
 * 支持本地 stdio 与远程 HTTP 两种传输，字段互斥：stdio 必填 command；
 * 远程必填 url（https 或本机回环 http），headers 值只接受 ${VAR} 占位符。
 * status 的 spec.env/spec.headers 是脱敏视图（普通字面值被掩码为 <literal>），不能当完整原定义：
 * 编辑时以 fullDefinition（policy:get 的 sqlite 有效定义）为基底，只覆盖表单负责字段，
 * 表单未展示的定义字段与已有密钥取值原样保留；
 * 拿不到完整原定义（无 DB 旧存储）时只提交表单负责字段，由保存方迁移后取完整定义合并。
 */
export function McpServerFormDialog({
  agentIds,
  /** 编辑时传入既有元素；新增传 null。 */
  editing,
  layers,
  /** 完整原定义（policy:get 的 sqlite 有效定义），整条替换基底；新增或无 DB 旧存储为 null。 */
  fullDefinition,
  onSubmit,
  onClose,
}: {
  agentIds: string[];
  editing: EngineElement | null;
  layers: PolicyLayers | null;
  fullDefinition?: Record<string, unknown> | null;
  /** complete=false 表示 definition 只是表单负责字段，需由保存方迁移后取完整定义合并。 */
  onSubmit: (name: string, definition: Record<string, unknown>, isNew: boolean, complete: boolean) => void;
  onClose: () => void;
}) {
  const editingSpec = editing ? (editing.spec as McpSpec) : null;
  const initialTransport: McpTransport =
    typeof fullDefinition?.url === "string" || typeof editingSpec?.url === "string" ? "remote" : "stdio";
  const [transport, setTransport] = useState<McpTransport>(initialTransport);
  const [name, setName] = useState(editing ? editing.id.replace(/^mcp:/, "") : "");
  const [command, setCommand] = useState(editingSpec?.command ?? "");
  const [argsText, setArgsText] = useState((editingSpec?.args ?? []).join("\n"));
  const [envText, setEnvText] = useState(Object.keys((fullDefinition?.env as Record<string, unknown> | undefined) ?? editingSpec?.env ?? {}).join("\n"));
  const [url, setUrl] = useState((fullDefinition?.url as string | undefined) ?? editingSpec?.url ?? "");
  const [headersText, setHeadersText] = useState(
    Object.entries((fullDefinition?.headers as Record<string, unknown> | undefined) ?? editingSpec?.headers ?? {})
      .map(([key, value]) => `${key}=${typeof value === "string" ? value : ""}`)
      .join("\n"),
  );
  const [timeoutText, setTimeoutText] = useState(String(editingSpec?.startup_timeout_sec ?? 120));
  const [selectedAgents, setSelectedAgents] = useState<string[]>(editingSpec?.agents ?? []);
  const [error, setError] = useState<string | null>(null);

  const isNew = editing === null;
  const origin = !isNew ? definitionOrigin(layers, "mcp", name) : null;

  const submit = () => {
    if (!/^[a-z0-9][a-z0-9-]*$/.test(name)) {
      setError("名称只能用小写字母/数字/连字符（如 my-server）");
      return;
    }
    const agents = selectedAgents.length > 0 ? selectedAgents : null;
    let owned: Record<string, unknown>;
    if (transport === "remote") {
      const trimmedUrl = url.trim();
      if (!trimmedUrl) {
        setError("远程服务地址（url）不能为空");
        return;
      }
      if (!isValidRemoteUrl(trimmedUrl)) {
        setError("服务地址仅支持 https://，或本机 http://localhost / http://127.0.0.1（可带端口与路径）");
        return;
      }
      // headers 与 env 同等语义：值只允许 ${VAR} 占位符（可写 VAR 简写），
      // 脱敏视图中的 <literal> 原样带出，由保存方用完整定义重建真实值。
      const fullHeaders = (fullDefinition?.headers ?? null) as Record<string, unknown> | null;
      const specHeaders = editingSpec?.headers ?? {};
      const headers: Record<string, string> = {};
      for (const line of headersText.split("\n").map((item) => item.trim()).filter(Boolean)) {
        const eq = line.indexOf("=");
        const headerName = (eq >= 0 ? line.slice(0, eq) : line).trim();
        if (!HEADER_NAME_RE.test(headerName)) {
          setError(`非法 header 名:${line}`);
          return;
        }
        const valuePart = eq >= 0 ? line.slice(eq + 1).trim() : "";
        if (valuePart) {
          if (valuePart === "<literal>") {
            const fromFull = typeof fullHeaders?.[headerName] === "string" ? (fullHeaders[headerName] as string) : null;
            headers[headerName] = fromFull ?? "<literal>";
          } else if (PLACEHOLDER_RE.test(valuePart)) {
            headers[headerName] = valuePart;
          } else if (VAR_NAME_RE.test(valuePart)) {
            headers[headerName] = `\${${valuePart}}`;
          } else {
            setError(`header 值只能是 \${VAR} 变量引用形式:${line}`);
            return;
          }
        } else {
          const fromFull = typeof fullHeaders?.[headerName] === "string" ? (fullHeaders[headerName] as string) : null;
          const fromSpec = typeof specHeaders[headerName] === "string" ? specHeaders[headerName] : null;
          const restored = fromFull ?? fromSpec;
          if (!restored) {
            setError(`header ${headerName} 缺少取值，请写成 ${headerName}=\${VAR_NAME}`);
            return;
          }
          headers[headerName] = restored;
        }
      }
      owned = { url: trimmedUrl, headers, agents };
    } else {
      if (!command.trim()) {
        setError("启动命令不能为空");
        return;
      }
      const timeout = Number(timeoutText);
      if (!Number.isInteger(timeout) || timeout <= 0) {
        setError("超时秒数要是正整数");
        return;
      }
      const args = argsText.split("\n").map((line) => line.trim()).filter(Boolean);
      // 密钥变量只编辑变量名清单。取值优先用完整原定义的真实值；只有脱敏 spec 时，
      // ${REF} 占位引用原样保留，被掩码为 <literal> 的字面值原样带出，由保存方用完整定义重建。
      const fullEnv = (fullDefinition?.env ?? null) as Record<string, unknown> | null;
      const specEnv = editingSpec?.env ?? {};
      const env: Record<string, string> = {};
      for (const line of envText.split("\n").map((item) => item.trim()).filter(Boolean)) {
        const varName = line.split("=")[0].trim();
        if (!VAR_NAME_RE.test(varName)) {
          setError(`非法变量名:${line}`);
          return;
        }
        const fromFull = typeof fullEnv?.[varName] === "string" ? (fullEnv[varName] as string) : null;
        const fromSpec = typeof specEnv[varName] === "string" ? specEnv[varName] : null;
        env[varName] = fromFull ?? fromSpec ?? `\${${varName}}`;
      }
      owned = {
        command: command.trim(),
        args,
        startup_timeout_sec: timeout,
        agents,
        env,
      };
    }
    // 表单负责字段（可选字段显式给出）；编辑且有完整原定义时以其为基底，
    // 未展示字段原样保留；否则只提交这些字段，由保存方迁移后合并完整定义。
    // 两种传输字段互斥：切换传输时剥离对方字段，避免混合定义被引擎拒绝。
    const complete = isNew || fullDefinition != null;
    const definition: Record<string, unknown> = !isNew && fullDefinition ? { ...fullDefinition, ...owned } : owned;
    if (transport === "remote") {
      for (const key of ["command", "args", "env", "startup_timeout_sec", "extra_args_by_agent", "type"]) {
        delete definition[key];
      }
    } else {
      for (const key of ["url", "headers", "type"]) {
        delete definition[key];
      }
    }
    onSubmit(name, definition, isNew, complete);
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-slate-900/40 p-4 animate-fade-in" onClick={(e) => e.target === e.currentTarget && onClose()}>
      <div role="dialog" aria-modal="true" aria-label={isNew ? "新增服务" : `编辑 ${name}`} className="max-h-[90vh] w-full max-w-xl overflow-y-auto rounded-xl border bg-card p-5 shadow-xl animate-zoom-in-95">
        <div className="flex items-start justify-between">
          <h3 className="text-base font-semibold text-foreground">{isNew ? "新增 MCP 服务" : `编辑服务 ${name}`}</h3>
          <button type="button" onClick={onClose} aria-label="关闭" className="rounded p-1 text-muted-foreground hover:bg-accent">
            <X className="h-4 w-4" aria-hidden />
          </button>
        </div>

        {!isNew && origin === "builtin" && (
          <p data-testid="builtin-overlay-note" className="mt-2 rounded-md border border-info/30 bg-info/10 px-3 py-2 text-xs text-info">
            {OVERLAY_COPY.editBuiltinNote}
          </p>
        )}

        <div className="mt-4 space-y-3 text-sm">
          <div>
            <label className="mb-1 block text-xs font-medium text-muted-foreground">名称（唯一，小写字母/数字/连字符）</label>
            <input
              value={name}
              onChange={(e) => setName(e.target.value)}
              disabled={!isNew}
              data-testid="server-name-input"
              className="w-full rounded border border-input px-2 py-1.5 font-mono text-xs disabled:bg-muted disabled:text-muted-foreground"
              placeholder="my-server"
            />
          </div>
          <div>
            <span className="mb-1 block text-xs font-medium text-muted-foreground">传输方式</span>
            <div className="flex gap-4">
              <label className="flex items-center gap-1.5 text-xs text-foreground">
                <input
                  type="radio"
                  name="mcp-transport"
                  checked={transport === "stdio"}
                  onChange={() => setTransport("stdio")}
                  data-testid="server-transport-stdio"
                />
                本地 stdio（启动命令）
              </label>
              <label className="flex items-center gap-1.5 text-xs text-foreground">
                <input
                  type="radio"
                  name="mcp-transport"
                  checked={transport === "remote"}
                  onChange={() => setTransport("remote")}
                  data-testid="server-transport-remote"
                />
                远程 HTTP（服务地址）
              </label>
            </div>
          </div>
          {transport === "remote" ? (
            <>
              <div>
                <label className="mb-1 block text-xs font-medium text-muted-foreground">服务地址（https://，或本机 http://localhost / http://127.0.0.1）</label>
                <input
                  value={url}
                  onChange={(e) => setUrl(e.target.value)}
                  data-testid="server-url-input"
                  className="w-full rounded border border-input px-2 py-1.5 font-mono text-xs"
                  placeholder="https://example.com/mcp"
                />
              </div>
              <div>
                <label className="mb-1 block text-xs font-medium text-muted-foreground">请求头变量引用（每行一个 Header-Name=${"{VAR}"}；值由 Agent Manager 进程环境提供，不在此保存或显示）</label>
                <textarea
                  value={headersText}
                  onChange={(e) => setHeadersText(e.target.value)}
                  rows={2}
                  data-testid="server-headers-input"
                  className="w-full rounded border border-input px-2 py-1.5 font-mono text-xs"
                  placeholder="Authorization=${GROK_API_KEY}"
                />
              </div>
            </>
          ) : (
            <>
              <div>
                <label className="mb-1 block text-xs font-medium text-muted-foreground">启动命令（如 npx / uvx）</label>
                <input
                  value={command}
                  onChange={(e) => setCommand(e.target.value)}
                  data-testid="server-command-input"
                  className="w-full rounded border border-input px-2 py-1.5 font-mono text-xs"
                  placeholder="npx"
                />
              </div>
              <div>
                <label className="mb-1 block text-xs font-medium text-muted-foreground">参数（每行一个）</label>
                <textarea
                  value={argsText}
                  onChange={(e) => setArgsText(e.target.value)}
                  rows={3}
                  data-testid="server-args-input"
                  className="w-full rounded border border-input px-2 py-1.5 font-mono text-xs"
                  placeholder={"-y\nchrome-devtools-mcp"}
                />
              </div>
              <div>
                <label className="mb-1 block text-xs font-medium text-muted-foreground">密钥变量名（每行一个；只写变量名，值由 Agent Manager 进程环境提供，不在此保存或显示）</label>
                <textarea
                  value={envText}
                  onChange={(e) => setEnvText(e.target.value)}
                  rows={2}
                  data-testid="server-env-input"
                  className="w-full rounded border border-input px-2 py-1.5 font-mono text-xs"
                  placeholder="GROK_API_KEY"
                />
              </div>
            </>
          )}
          <div>
            <span className="mb-1 block text-xs font-medium text-muted-foreground">允许分配的助手（不选则不限）</span>
            <p className="mb-1.5 text-xs text-muted-foreground">这里只保存允许范围，保存后再为各助手分别添加。</p>
            <div className="flex flex-wrap gap-3">
              {agentIds.map((agent) => (
                <label key={agent} className="flex items-center gap-1.5 text-xs text-foreground">
                  <input
                    type="checkbox"
                    checked={selectedAgents.includes(agent)}
                    onChange={(e) =>
                      setSelectedAgents((prev) => (e.target.checked ? [...prev, agent] : prev.filter((item) => item !== agent)))
                    }
                    data-testid={`server-agent-${agent}`}
                  />
                  <span className="font-mono">{agent}</span>
                </label>
              ))}
            </div>
          </div>
          {transport === "stdio" && (
            <div>
              <label className="mb-1 block text-xs font-medium text-muted-foreground">启动超时（秒）</label>
              <input
                value={timeoutText}
                onChange={(e) => setTimeoutText(e.target.value)}
                data-testid="server-timeout-input"
                className="w-28 rounded border border-input px-2 py-1.5 font-mono text-xs"
              />
            </div>
          )}
          {error && <p className="text-xs text-destructive">{error}</p>}
        </div>

        <div className="mt-5 flex items-center justify-end gap-2">
          <span className="mr-auto text-xs text-muted-foreground">只保存到本地资源库，不会自动安装或同步</span>
          <button type="button" onClick={onClose} className="rounded-md border px-4 py-2 text-sm text-muted-foreground hover:bg-accent">
            取消
          </button>
          <button
            type="button"
            onClick={submit}
            data-testid="server-form-submit"
            className="rounded-md bg-primary px-4 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90"
          >
            保存到资源库
          </button>
        </div>
      </div>
    </div>
  );
}
