import { useState } from "react";
import { X } from "lucide-react";
import type { EngineElement, PolicyLayers } from "../lib/contract";
import { definitionOrigin } from "../lib/definitionOverlay";
import type { McpSpec } from "../lib/observed";
import { OVERLAY_COPY } from "../lib/ui-copy";

/**
 * MCP 服务新增/编辑表单。写入的是定义覆盖层片段（mcp.definitions.<name>）；
 * 编辑基线条目时整段写覆盖片段并注明「仓库默认值不受影响」。
 */
export function McpServerFormDialog({
  agentIds,
  /** 编辑时传入既有元素；新增传 null。 */
  editing,
  layers,
  onSubmit,
  onClose,
}: {
  agentIds: string[];
  editing: EngineElement | null;
  layers: PolicyLayers | null;
  onSubmit: (name: string, definition: Record<string, unknown>, isNew: boolean) => void;
  onClose: () => void;
}) {
  const editingSpec = editing ? (editing.spec as McpSpec) : null;
  const [name, setName] = useState(editing ? editing.id.replace(/^mcp:/, "") : "");
  const [command, setCommand] = useState(editingSpec?.command ?? "");
  const [argsText, setArgsText] = useState((editingSpec?.args ?? []).join("\n"));
  const [envText, setEnvText] = useState(Object.keys(editingSpec?.env ?? {}).join("\n"));
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
    const env: Record<string, string> = {};
    for (const line of envText.split("\n").map((item) => item.trim()).filter(Boolean)) {
      const varName = line.split("=")[0].trim();
      if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(varName)) {
        setError(`非法变量名:${line}`);
        return;
      }
      env[varName] = `\${${varName}}`;
    }
    const definition: Record<string, unknown> = {
      command: command.trim(),
      args,
      startup_timeout_sec: timeout,
      agents: selectedAgents.length > 0 ? selectedAgents : null,
      env,
    };
    onSubmit(name, definition, isNew);
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-slate-900/40 p-4" onClick={(e) => e.target === e.currentTarget && onClose()}>
      <div role="dialog" aria-modal="true" aria-label={isNew ? "新增服务" : `编辑 ${name}`} className="max-h-[90vh] w-full max-w-xl overflow-y-auto rounded-xl border border-slate-200 bg-white p-5 shadow-xl">
        <div className="flex items-start justify-between">
          <h3 className="text-base font-semibold text-slate-800">{isNew ? "新增 MCP 服务" : `编辑服务 ${name}`}</h3>
          <button type="button" onClick={onClose} aria-label="关闭" className="rounded p-1 text-slate-400 hover:bg-slate-100">
            <X className="h-4 w-4" aria-hidden />
          </button>
        </div>

        {!isNew && origin === "baseline" && (
          <p data-testid="baseline-overlay-note" className="mt-2 rounded-md border border-sky-200 bg-sky-50 px-3 py-2 text-xs text-sky-800">
            {OVERLAY_COPY.editBaselineNote}
          </p>
        )}

        <div className="mt-4 space-y-3 text-sm">
          <div>
            <label className="mb-1 block text-xs font-medium text-slate-500">名称（唯一，小写字母/数字/连字符）</label>
            <input
              value={name}
              onChange={(e) => setName(e.target.value)}
              disabled={!isNew}
              data-testid="server-name-input"
              className="w-full rounded border border-slate-300 px-2 py-1.5 font-mono text-xs disabled:bg-slate-50 disabled:text-slate-400"
              placeholder="my-server"
            />
          </div>
          <div>
            <label className="mb-1 block text-xs font-medium text-slate-500">启动命令（如 npx / uvx）</label>
            <input
              value={command}
              onChange={(e) => setCommand(e.target.value)}
              data-testid="server-command-input"
              className="w-full rounded border border-slate-300 px-2 py-1.5 font-mono text-xs"
              placeholder="npx"
            />
          </div>
          <div>
            <label className="mb-1 block text-xs font-medium text-slate-500">参数（每行一个）</label>
            <textarea
              value={argsText}
              onChange={(e) => setArgsText(e.target.value)}
              rows={3}
              data-testid="server-args-input"
              className="w-full rounded border border-slate-300 px-2 py-1.5 font-mono text-xs"
              placeholder={"-y\nchrome-devtools-mcp"}
            />
          </div>
          <div>
            <label className="mb-1 block text-xs font-medium text-slate-500">密钥变量名（每行一个；只写变量名，值从密钥仓库取，永不显示）</label>
            <textarea
              value={envText}
              onChange={(e) => setEnvText(e.target.value)}
              rows={2}
              data-testid="server-env-input"
              className="w-full rounded border border-slate-300 px-2 py-1.5 font-mono text-xs"
              placeholder="GROK_API_KEY"
            />
          </div>
          <div>
            <span className="mb-1 block text-xs font-medium text-slate-500">应用到哪些助手（都不勾 = 全部）</span>
            <div className="flex flex-wrap gap-3">
              {agentIds.map((agent) => (
                <label key={agent} className="flex items-center gap-1.5 text-xs text-slate-700">
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
          <div>
            <label className="mb-1 block text-xs font-medium text-slate-500">启动超时（秒）</label>
            <input
              value={timeoutText}
              onChange={(e) => setTimeoutText(e.target.value)}
              data-testid="server-timeout-input"
              className="w-28 rounded border border-slate-300 px-2 py-1.5 font-mono text-xs"
            />
          </div>
          {error && <p className="text-xs text-red-600">{error}</p>}
        </div>

        <div className="mt-5 flex justify-end gap-2">
          <button type="button" onClick={onClose} className="rounded-md border border-slate-200 px-4 py-2 text-sm text-slate-600 hover:bg-slate-50">
            取消
          </button>
          <button
            type="button"
            onClick={submit}
            data-testid="server-form-submit"
            className="rounded-md bg-indigo-600 px-4 py-2 text-sm font-medium text-white hover:bg-indigo-700"
          >
            {isNew ? "保存并预览同步" : "保存修改并预览"}
          </button>
        </div>
      </div>
    </div>
  );
}
