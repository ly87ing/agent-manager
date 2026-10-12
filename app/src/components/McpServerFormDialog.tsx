import { useEffect, useRef, useState } from "react";
import { X } from "lucide-react";
import type { McpTarget } from "../lib/contract";
import type { McpAgentCell, McpSavedDefinition } from "../lib/observed";
import { useDialogFocus } from "../lib/dialogFocus";
import { FileDiff } from "./FileDiff";
import { parsePastedServer } from "../lib/mcpPaste";
import { Select } from "./Select";

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
const KEEP = "<literal>";
/**
 * 表单里「保留原值」的写法；保存时换回引擎约定的 <literal>。引擎不交出像凭据的明文，
 * 但给一个认得出的提示（如 sk-…6789），写成「（保留原值：sk-…6789）」；提示只用来看，不会写进文件。
 */
const KEEP_LABEL = "（保留原值）";
const KEEP_RE = /^（保留原值(：.*)?）$/;
const isKeep = (value: string) => value === KEEP || KEEP_RE.test(value);

function lines(map: Record<string, string> | undefined, hints?: Record<string, string>): string {
  return Object.entries(map ?? {})
    .map(([key, value]) => `${key}=${value === KEEP ? (hints?.[key] ? `（保留原值：${hints[key]}）` : KEEP_LABEL) : value}`)
    .join("\n");
}

/** 提交给引擎的表单内容（只含已建模字段；超时由调用方按助手决定带不带）。 */
export interface McpFormPayload {
  name: string;
  /** 新增时选中的助手；编辑时为该条目所在的助手。 */
  agents: string[];
  spec: Record<string, unknown>;
  /** Codex 启动超时（秒）；null = 不写。 */
  startupTimeoutSec: number | null;
}

export interface McpFilePreview {
  agent: string;
  files: { path: string; exists: boolean; content: string; original?: string | null }[];
  error?: string;
}

export type McpFormMode =
  | {
      kind: "add";
      targets: McpTarget[];
      /** 从「已保存的定义」添加时预填。 */
      initialName?: string;
      initial?: McpSavedDefinition["definition"];
    }
  | {
      kind: "edit";
      name: string;
      /** 表单预填取自这个账号的条目。 */
      agent: string;
      cell: McpAgentCell;
      format: string;
      /** 保存时写入的全部账号（含 agent），各自带写入基准。 */
      targets: { agent: string; cell: McpAgentCell; format: string }[];
      /** 各账号的内容不一样：保存后统一成表单里这一份。 */
      differs: boolean;
    };

/**
 * MCP 条目表单：新增直接写入所选助手的配置文件；编辑只改这一条的已建模字段，
 * 条目里其他官方字段原样保留。env 的 `<literal>` 表示保留原值（界面拿不到明文），
 * `${VAR}` 为变量引用（写入时按助手规则解析或引用），其余为新写的普通取值
 * （疑似凭据会被引擎拒绝）。header 只接受变量引用或保留原值。
 */
export function McpServerFormDialog({
  mode,
  onSubmit,
  onPreview,
  onClose,
  nameOf = (agent) => agent,
  onSwitchBase,
}: {
  mode: McpFormMode;
  onSubmit: (payload: McpFormPayload) => void;
  onPreview: (payload: McpFormPayload) => Promise<McpFilePreview[] | string>;
  onClose: () => void;
  /** 账号键 → 账号名。 */
  nameOf?: (agent: string) => string;
  /** 各账号内容不同时，换成以另一个账号的内容为准重新预填。 */
  onSwitchBase?: (agent: string) => void;
}) {
  const isNew = mode.kind === "add";
  const editEntry = mode.kind === "edit" ? (mode.cell.entry ?? {}) : null;
  const initial = mode.kind === "add" ? mode.initial : undefined;
  const writableTargets =
    mode.kind === "add" ? mode.targets.filter((target) => target.error === null && target.agent_enabled) : [];

  const [transport, setTransport] = useState<McpTransport>(
    mode.kind === "edit"
      ? mode.cell.transport === "remote"
        ? "remote"
        : "stdio"
      : typeof initial?.url === "string"
        ? "remote"
        : "stdio",
  );
  const [name, setName] = useState(mode.kind === "edit" ? mode.name : (mode.initialName ?? ""));
  const [selectedAgents, setSelectedAgents] = useState<string[]>(
    // 新增默认给全部账号（产品规则：新服务默认分配给所有账号），可以取消
    mode.kind === "edit" ? [mode.agent] : writableTargets.map((target) => target.agent),
  );
  const [command, setCommand] = useState(editEntry?.command ?? initial?.command ?? "");
  const [argsText, setArgsText] = useState((editEntry?.args ?? initial?.args ?? []).join("\n"));
  const hints = mode.kind === "edit" ? mode.cell.literal_hints : undefined;
  const [envText, setEnvText] = useState(lines(editEntry?.env ?? initial?.env, hints?.env));
  const [url, setUrl] = useState(editEntry?.url ?? initial?.url ?? "");
  const [headersText, setHeadersText] = useState(lines(editEntry?.headers ?? initial?.headers, hints?.headers));
  const [timeoutText, setTimeoutText] = useState(
    // 不替用户加原来没配的字段：新增时也留空（用 Codex 自己的默认），填了才写
    String(editEntry?.startup_timeout_sec ?? initial?.startup_timeout_sec ?? ""),
  );
  const [error, setError] = useState<string | null>(null);
  /** 粘贴配置：null = 收起；notes 是填入后给用户的说明。 */
  const [paste, setPaste] = useState<{ text: string; error: string | null; notes: string[] } | null>(null);
  const [preview, setPreview] = useState<{ key: string; result: McpFilePreview[] | string } | null>(null);
  const [previewBusy, setPreviewBusy] = useState(false);
  const previewSerial = useRef(0);
  const { dialogRef, onDialogKeyDown } = useDialogFocus<HTMLDivElement>({ onEscape: onClose });

  const formatOf = (agent: string) =>
    mode.targets.find((target) => target.agent === agent)?.format ?? "";
  const involvesCodex = selectedAgents.some((agent) => formatOf(agent) === "codex-toml");
  const preserved = mode.kind === "edit" ? Object.keys(mode.cell.extra ?? {}).filter((key) => key !== "type") : [];

  const buildPayload = (): McpFormPayload | string => {
    if (isNew && !/^[A-Za-z0-9_-]+$/.test(name)) {
      return "名称只能用字母、数字、- 与 _（如 my-server）";
    }
    if (selectedAgents.length === 0) {
      return "至少选一个账号";
    }
    let spec: Record<string, unknown>;
    if (transport === "remote") {
      const trimmedUrl = url.trim();
      if (!trimmedUrl) {
        return "远程服务地址（url）不能为空";
      }
      const urlChanged = mode.kind === "add" || trimmedUrl !== (editEntry?.url ?? "");
      if (urlChanged && !isValidRemoteUrl(trimmedUrl)) {
        return "服务地址仅支持 https://，或本机 http://localhost / http://127.0.0.1（可带端口与路径）";
      }
      const headers: Record<string, string> = {};
      for (const line of headersText.split("\n").map((item) => item.trim()).filter(Boolean)) {
        const eq = line.indexOf("=");
        const headerName = (eq >= 0 ? line.slice(0, eq) : line).trim();
        const value = eq >= 0 ? line.slice(eq + 1).trim() : "";
        if (!HEADER_NAME_RE.test(headerName)) {
          return `非法 header 名:${line}`;
        }
        if (isKeep(value) && !isNew) {
          headers[headerName] = KEEP;
        } else if (PLACEHOLDER_RE.test(value)) {
          headers[headerName] = value;
        } else if (VAR_NAME_RE.test(value)) {
          headers[headerName] = `\${${value}}`;
        } else {
          return `请求头的值只能写 \${变量名}${isNew ? "" : "或「（保留原值）」"}：${headerName}`;
        }
      }
      spec = { url: trimmedUrl, headers };
    } else {
      if (!command.trim()) {
        return "启动命令不能为空";
      }
      const env: Record<string, string> = {};
      for (const line of envText.split("\n").map((item) => item.trim()).filter(Boolean)) {
        const eq = line.indexOf("=");
        if (eq <= 0) {
          return `环境变量要写成 KEY=取值:${line}`;
        }
        const key = line.slice(0, eq).trim();
        const value = line.slice(eq + 1).trim();
        if (/\s/.test(key)) {
          return `非法变量名:${key}`;
        }
        if (isKeep(value) && isNew) {
          return `新增时没有可保留的原值，请写 ${key}=\${VAR} 或直接写取值`;
        }
        env[key] = isKeep(value) ? KEEP : value;
      }
      spec = {
        command: command.trim(),
        args: argsText.split("\n").map((line) => line.trim()).filter(Boolean),
        env,
      };
    }
    let startupTimeoutSec: number | null = null;
    if (transport === "stdio" && involvesCodex && timeoutText.trim()) {
      const timeout = Number(timeoutText);
      if (!Number.isInteger(timeout) || timeout <= 0) {
        return "超时秒数要是正整数";
      }
      startupTimeoutSec = timeout;
    }
    return { name, agents: selectedAgents, spec, startupTimeoutSec };
  };
  const draft = buildPayload();
  const previewKey = typeof draft === "string" ? "" : JSON.stringify({
    draft,
    mode: mode.kind,
    base: mode.kind === "edit" ? mode.targets.map((target) => target.cell.base) : null,
  });

  useEffect(() => {
    const serial = ++previewSerial.current;
    if (!previewKey || typeof draft === "string") {
      setPreviewBusy(false);
      return;
    }
    setPreviewBusy(true);
    const timer = window.setTimeout(() => {
      void onPreview(draft)
        .then((result) => {
          if (serial === previewSerial.current) setPreview({ key: previewKey, result });
        })
        .catch((reason) => {
          if (serial === previewSerial.current) setPreview({ key: previewKey, result: String(reason) });
        })
        .finally(() => {
          if (serial === previewSerial.current) setPreviewBusy(false);
        });
    }, 300);
    return () => {
      window.clearTimeout(timer);
      previewSerial.current += 1;
    };
  }, [previewKey]);

  const fillFromPaste = () => {
    if (!paste) return;
    const parsed = parsePastedServer(paste.text);
    if (typeof parsed === "string") {
      setPaste({ ...paste, error: parsed });
      return;
    }
    if (parsed.name) setName(parsed.name);
    setTransport(parsed.transport);
    setCommand(parsed.command);
    setArgsText(parsed.args.join("\n"));
    setEnvText(lines(parsed.env));
    setUrl(parsed.url);
    setHeadersText(lines(parsed.headers));
    setError(null);
    setPaste(parsed.notes.length ? { text: "", error: null, notes: parsed.notes } : null);
  };

  const submit = () => {
    const payload = buildPayload();
    if (typeof payload === "string") {
      setError(payload);
      return;
    }
    onSubmit(payload);
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-slate-900/40 p-4 animate-fade-in" onClick={(e) => e.target === e.currentTarget && onClose()}>
      <div ref={dialogRef} role="dialog" aria-modal="true" aria-label={isNew ? "新增服务" : `编辑 ${name}`} tabIndex={-1} onKeyDown={onDialogKeyDown} className="max-h-[92vh] w-full max-w-5xl overflow-y-auto rounded-xl border bg-card p-5 shadow-xl animate-zoom-in-95 sm:p-6">
        <div className="flex items-start justify-between">
          <h3 className="text-base font-semibold text-foreground">
            {isNew ? "新增 MCP 服务" : `编辑 ${name}`}
          </h3>
          <div className="flex items-center gap-2">
            {isNew && !paste && (
              <button
                type="button"
                data-testid="server-paste-open"
                onClick={() => setPaste({ text: "", error: null, notes: [] })}
                className="rounded-md border border-primary/30 bg-primary/10 px-2.5 py-1 text-xs text-primary hover:bg-primary/15"
              >
                粘贴配置
              </button>
            )}
            <button type="button" onClick={onClose} aria-label="关闭" className="rounded p-1 text-muted-foreground hover:bg-accent">
              <X className="h-4 w-4" aria-hidden />
            </button>
          </div>
        </div>
        {paste && paste.notes.length > 0 && (
          <ul data-testid="server-paste-notes" className="mt-3 space-y-0.5 rounded-md border border-info/30 bg-info/5 px-3 py-2 text-xs">
            {paste.notes.map((note) => (
              <li key={note}>{note}</li>
            ))}
          </ul>
        )}
        {paste && paste.notes.length === 0 && (
          <div className="mt-3 space-y-2 rounded-md border bg-muted/40 p-3">
            <label htmlFor="server-paste-text" className="block text-xs text-muted-foreground">
              把服务说明里的配置整段贴进来（如 {'{"mcpServers": {…}}'}），会填进下面的表单，保存前可以再改。
            </label>
            <textarea
              id="server-paste-text"
              data-testid="server-paste-input"
              value={paste.text}
              onChange={(e) => setPaste({ ...paste, text: e.target.value, error: null })}
              rows={5}
              className="w-full rounded border border-input bg-card px-2 py-1.5 font-mono text-xs"
              placeholder='{"mcpServers": {"my-server": {"command": "npx", "args": ["-y", "my-mcp"]}}}'
            />
            {paste.error && <p role="alert" className="text-xs text-destructive">{paste.error}</p>}
            <div className="flex justify-end gap-2">
              <button type="button" onClick={() => setPaste(null)} className="rounded-md px-3 py-1 text-xs text-muted-foreground hover:bg-accent">
                取消
              </button>
              <button type="button" data-testid="server-paste-fill" disabled={!paste.text.trim()} onClick={fillFromPaste} className="rounded-md bg-primary px-3 py-1 text-xs text-primary-foreground hover:bg-primary/90 disabled:opacity-40">
                填入表单
              </button>
            </div>
          </div>
        )}

        <div className="mt-4 grid grid-cols-1 gap-4 text-sm lg:grid-cols-2">
          <div className="lg:col-span-2">
            <label className="mb-1 block text-xs font-medium text-muted-foreground">名称（字母、数字、- 与 _）</label>
            <input
              value={name}
              onChange={(e) => setName(e.target.value)}
              disabled={!isNew}
              data-testid="server-name-input"
              className="w-full rounded border border-input px-2 py-1.5 font-mono text-xs disabled:bg-muted disabled:text-muted-foreground"
              placeholder="my-server"
            />
          </div>
          {mode.kind === "add" && (
            <div className="lg:col-span-2">
              <span className="mb-1 block text-xs font-medium text-muted-foreground">写入哪些账号</span>
              <div className="flex flex-wrap gap-3">
                {writableTargets.map((target) => (
                  <label key={target.agent} className="flex items-center gap-1.5 text-xs text-foreground">
                    <input
                      type="checkbox"
                      checked={selectedAgents.includes(target.agent)}
                      onChange={(e) =>
                        setSelectedAgents((prev) =>
                          e.target.checked ? [...prev, target.agent] : prev.filter((item) => item !== target.agent),
                        )
                      }
                      data-testid={`server-agent-${target.agent}`}
                    />
                    <span>{nameOf(target.agent)}</span>
                  </label>
                ))}
                {writableTargets.length === 0 && <span className="text-xs text-muted-foreground">没有可写入的助手配置</span>}
              </div>
            </div>
          )}
          {mode.kind === "edit" && (
            <div className="space-y-1.5 text-xs lg:col-span-2" data-testid="server-edit-targets">
              <fieldset className="flex flex-wrap gap-3">
                <legend className="mb-1 text-muted-foreground">保存到哪些助手</legend>
                {mode.targets.map((target) => (
                  <label key={target.agent} className="flex items-center gap-1.5">
                    <input type="checkbox" data-testid={`server-agent-${target.agent}`}
                      checked={selectedAgents.includes(target.agent)}
                      onChange={(event) => setSelectedAgents((previous) => event.target.checked
                        ? [...previous, target.agent] : previous.filter((agent) => agent !== target.agent))} />
                    {nameOf(target.agent)}
                  </label>
                ))}
              </fieldset>
              <p className="text-muted-foreground">
                保存后写入：<span className="text-foreground">{selectedAgents.map(nameOf).join("、") || "尚未选择"}</span>
                。未勾选的助手不受影响；未列出的字段和「保留原值」均保留各助手自己的内容。
              </p>
              {mode.differs && (
                <div className="flex flex-wrap items-center gap-2 rounded-md border border-warning/40 bg-warning/10 px-2.5 py-1.5 text-warning">
                  <span>各助手的内容不同。勾选多个目标时，将使用下面的内容。选择预填来源会重置目标选择：</span>
                  <Select
                    label="以哪个账号的内容为准"
                    testId="server-edit-base"
                    value={mode.agent}
                    onChange={(next) => onSwitchBase?.(next)}
                    className="rounded border bg-card px-1.5 py-0.5"
                    options={mode.targets.map((target) => ({ value: target.agent, label: nameOf(target.agent) }))}
                  />
                </div>
              )}
            </div>
          )}
          <div className="lg:col-span-2">
            <span className="mb-1 block text-xs font-medium text-muted-foreground">传输方式</span>
            <div className="flex gap-4">
              {(["stdio", "remote"] as const).map((kind) => (
                <label key={kind} className="flex items-center gap-1.5 text-xs text-foreground">
                  <input
                    type="radio"
                    name="mcp-transport"
                    checked={transport === kind}
                    disabled={!isNew}
                    onChange={() => setTransport(kind)}
                    data-testid={`server-transport-${kind}`}
                  />
                  {kind === "stdio" ? "本地命令" : "远程地址"}
                </label>
              ))}
            </div>
            {!isNew && <p className="mt-1 text-xs text-muted-foreground">编辑不能切换传输方式；要换请删除后重新新增。</p>}
          </div>
          {transport === "remote" ? (
            <>
              <div className="lg:col-span-2">
                <label className="mb-1 block text-xs font-medium text-muted-foreground">服务地址（https://，或本机 http://localhost / http://127.0.0.1）</label>
                <input
                  value={url}
                  onChange={(e) => setUrl(e.target.value)}
                  data-testid="server-url-input"
                  className="w-full rounded border border-input px-2 py-1.5 font-mono text-xs"
                  placeholder="https://example.com/mcp"
                />
              </div>
              <div className="lg:col-span-2">
                <label className="mb-1 block text-xs font-medium text-muted-foreground">
                  请求头（每行一个 名称=${"{变量名}"}；{isNew ? "" : "「（保留原值）」表示不改；"}取值由变量提供，不在这里显示）
                </label>
                <textarea
                  value={headersText}
                  onChange={(e) => setHeadersText(e.target.value)}
                  rows={5}
                  data-testid="server-headers-input"
                  className="w-full rounded border border-input px-2 py-1.5 font-mono text-xs"
                  placeholder="Authorization=${EXAMPLE_API_KEY}"
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
                  rows={7}
                  data-testid="server-args-input"
                  className="w-full rounded border border-input px-2 py-1.5 font-mono text-xs"
                  placeholder={"-y\nchrome-devtools-mcp"}
                />
              </div>
              <div className="lg:col-span-2">
                <label className="mb-1 block text-xs font-medium text-muted-foreground">
                  环境变量（每行一个 名称=取值；密钥写 ${"{变量名}"}，取值放进密钥文件{isNew ? "" : "；「（保留原值）」表示不改"}）
                </label>
                <textarea
                  value={envText}
                  onChange={(e) => setEnvText(e.target.value)}
                  rows={6}
                  data-testid="server-env-input"
                  className="w-full rounded border border-input px-2 py-1.5 font-mono text-xs"
                  placeholder={"API_KEY=${EXAMPLE_API_KEY}\nNODE_ENV=production"}
                />
              </div>
              {involvesCodex && (
                <div>
                  <label className="mb-1 block text-xs font-medium text-muted-foreground">Codex 启动超时（秒，留空用默认）</label>
                  <input
                    value={timeoutText}
                    onChange={(e) => setTimeoutText(e.target.value)}
                    data-testid="server-timeout-input"
                    className="w-28 rounded border border-input px-2 py-1.5 font-mono text-xs"
                  />
                </div>
              )}
            </>
          )}
          {preserved.length > 0 && (
            <p data-testid="server-preserved-fields" className="text-xs text-muted-foreground lg:col-span-2">
              条目里的这些字段本表单不改动，保存时原样保留：{preserved.join("、")}
            </p>
          )}
          <section className="space-y-2 rounded-lg border p-3 text-xs lg:col-span-2" data-testid="mcp-file-preview">
            <h4 className="font-medium text-foreground">将要改动的配置</h4>
            <p className="text-muted-foreground">按当前实际文件计算，只显示变化的部分；保存时会重新读取并校验。</p>
            {typeof draft === "string" && <p className="text-muted-foreground">{draft}</p>}
            {typeof draft !== "string" && (previewBusy || preview?.key !== previewKey) && <p className="text-muted-foreground">正在计算…</p>}
            {preview?.key === previewKey && typeof preview.result === "string" && <p role="alert" className="text-destructive">{preview.result}</p>}
            {preview?.key === previewKey && Array.isArray(preview.result) && preview.result.map((item) => (
              <div key={item.agent} className="space-y-1 border-t pt-2">
                <p className="font-medium text-foreground">{nameOf(item.agent)}</p>
                {item.error && <p role="alert" className="text-destructive">{item.error}</p>}
                {!item.error && item.files.length === 0 && <p className="text-muted-foreground">文件内容不变。</p>}
                {item.files.map((file) => (
                  <FileDiff
                    key={file.path}
                    path={file.path}
                    original={file.exists ? file.original : null}
                    content={file.content}
                    testId={`mcp-preview-content-${item.agent}`}
                  />
                ))}
              </div>
            ))}
          </section>
          {error && <p role="alert" className="text-xs text-destructive lg:col-span-2">{error}</p>}
        </div>

        <div className="sticky -bottom-5 z-10 -mx-5 -mb-5 border-t bg-card px-5 py-3 mt-5 flex items-center justify-end gap-2 sm:-bottom-6 sm:-mx-6 sm:-mb-6 sm:px-6">
          <span className="mr-auto text-xs text-muted-foreground">
            {isNew ? "直接写入所选账号的配置文件" : "只改这一条，每个账号写入前都会核对是否已被外部修改"}
          </span>
          <button type="button" onClick={onClose} className="rounded-md border px-4 py-2 text-sm text-muted-foreground hover:bg-accent">
            取消
          </button>
          <button
            type="button"
            onClick={submit}
            data-testid="server-form-submit"
            className="rounded-md bg-primary px-4 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90"
          >
            {isNew ? "写入配置" : "保存修改"}
          </button>
        </div>
      </div>
    </div>
  );
}
