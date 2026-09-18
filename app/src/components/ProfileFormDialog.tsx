import { useState } from "react";
import { X } from "lucide-react";
import { hasPresetArgs, presetsForCommand, togglePresetArgs } from "../lib/agentPresets";
import { cn } from "../lib/utils";
import {
  PROFILE_ID_RE,
  defaultConfigDir,
  profileKindOf,
  profileLaunchValue,
  type ProfileFormInput,
} from "../lib/profileDefinition";

/**
 * 助手实例表单：为同一个助手再配一套独立配置。
 *
 * 实例有自己的配置目录，启动时用环境变量指过去，所以多个实例可以同时开着互不干扰，
 * 全局配置一个字节都不动。鉴权只填变量名，取值留在环境变量里，不进本工具的库。
 */
export function ProfileFormDialog({
  baseAgents,
  initial,
  isEdit: isEditProp,
  existingIds,
  baseBinOf,
  onSubmit,
  onClose,
}: {
  /** 可作为归属的助手（目前支持 Claude Code 与 Codex）。 */
  baseAgents: { id: string; displayName: string }[];
  initial?: ProfileFormInput;
  /** 复制实例时带着预填内容但仍是新建：名称与配置目录必须可改。 */
  isEdit?: boolean;
  existingIds: string[];
  baseBinOf: (agentId: string) => string;
  onSubmit: (input: ProfileFormInput) => void;
  onClose: () => void;
}) {
  const isEdit = isEditProp ?? Boolean(initial);
  const firstAgent = baseAgents[0]?.id ?? "claude-code";
  const [id, setId] = useState(initial?.id ?? "");
  const [baseAgent, setBaseAgent] = useState(initial?.baseAgent ?? firstAgent);
  const [displayName, setDisplayName] = useState(initial?.displayName ?? "");
  const [configDir, setConfigDir] = useState(initial?.configDir ?? "");
  const [baseUrl, setBaseUrl] = useState(initial?.baseUrl ?? "");
  const [tokenVar, setTokenVar] = useState(initial?.tokenVar ?? "");
  const [model, setModel] = useState(initial?.model ?? "");
  const [extraArgs, setExtraArgs] = useState(initial?.extraArgs ?? "");
  const [error, setError] = useState<string | null>(null);

  const kind = profileKindOf(baseAgent);
  const effectiveConfigDir = configDir.trim() || defaultConfigDir(kind, id);
  const input: ProfileFormInput = {
    id,
    displayName,
    baseAgent,
    kind,
    configDir: effectiveConfigDir,
    baseUrl,
    tokenVar,
    model,
    extraArgs,
  };
  const launch = id ? profileLaunchValue(input, baseBinOf(baseAgent)) : "";
  /** 预设按该实例实际执行的命令给（实例的 bin 来自归属助手）。 */
  const presets = presetsForCommand(baseBinOf(baseAgent));
  const activeDangerPreset = presets.find((preset) => preset.danger && hasPresetArgs(extraArgs, preset));

  const submit = () => {
    if (!PROFILE_ID_RE.test(id)) {
      setError("实例名只能用小写字母、数字与连字符，且以字母或数字开头（它同时是终端里输入的命令名）");
      return;
    }
    if (!isEdit && existingIds.includes(id)) {
      setError("已有同名实例或助手，换一个名字");
      return;
    }
    if (!/^(~\/|\/)/.test(effectiveConfigDir)) {
      setError("配置目录要填绝对路径或以 ~/ 开头");
      return;
    }
    if (baseUrl.trim() && !/^https:\/\/|^http:\/\/(localhost|127\.0\.0\.1)/.test(baseUrl.trim())) {
      setError("接口地址只允许 https，或本机回环 http://localhost / http://127.0.0.1");
      return;
    }
    if (tokenVar.trim() && !/^[A-Za-z_][A-Za-z0-9_]*$/.test(tokenVar.trim())) {
      setError("鉴权变量名不合法：要以字母或下划线开头，只能用字母、数字与下划线");
      return;
    }
    onSubmit(input);
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-slate-900/40 p-4 animate-fade-in"
      onClick={(event) => event.target === event.currentTarget && onClose()}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-label={isEdit ? "编辑实例" : "新建实例"}
        className="max-h-[90vh] w-full max-w-lg overflow-y-auto rounded-xl border bg-card p-5 shadow-xl animate-zoom-in-95"
      >
        <div className="flex items-start justify-between">
          <div>
            <h3 className="text-base font-semibold text-foreground">{isEdit ? "编辑实例" : "新建实例"}</h3>
            <p className="mt-1 text-xs text-muted-foreground">
              同一个助手的另一套独立配置：自己的配置目录、登录、会话、技能与 MCP，可以和别的实例同时开着。
            </p>
          </div>
          <button type="button" onClick={onClose} aria-label="关闭" className="rounded p-1 text-muted-foreground hover:bg-accent">
            <X className="h-4 w-4" aria-hidden />
          </button>
        </div>

        <div className="mt-4 space-y-3 text-sm">
          <div className="grid grid-cols-2 gap-3">
            <div>
              <label className="mb-1 block text-xs font-medium text-muted-foreground" htmlFor="profile-base-agent">
                归属助手
              </label>
              <select
                id="profile-base-agent"
                value={baseAgent}
                disabled={isEdit}
                onChange={(event) => setBaseAgent(event.target.value)}
                data-testid="profile-base-agent"
                className="w-full rounded border border-input bg-background px-2 py-1.5 text-xs disabled:bg-muted"
              >
                {baseAgents.map((agent) => (
                  <option key={agent.id} value={agent.id}>
                    {agent.displayName}
                  </option>
                ))}
              </select>
            </div>
            <div>
              <label className="mb-1 block text-xs font-medium text-muted-foreground">实例名（终端里输入它启动）</label>
              <input
                value={id}
                disabled={isEdit}
                onChange={(event) => setId(event.target.value)}
                data-testid="profile-id-input"
                placeholder="cc-ly"
                className="w-full rounded border border-input px-2 py-1.5 font-mono text-xs disabled:bg-muted"
              />
            </div>
          </div>

          <div>
            <label className="mb-1 block text-xs font-medium text-muted-foreground">显示名称（可选）</label>
            <input
              value={displayName}
              onChange={(event) => setDisplayName(event.target.value)}
              data-testid="profile-display-name"
              placeholder="Claude Code · 工作号"
              className="w-full rounded border border-input px-2 py-1.5 text-xs"
            />
          </div>

          <div>
            <label className="mb-1 block text-xs font-medium text-muted-foreground">配置目录（留空用默认）</label>
            <input
              value={configDir}
              onChange={(event) => setConfigDir(event.target.value)}
              data-testid="profile-config-dir"
              placeholder={defaultConfigDir(kind, id)}
              className="w-full rounded border border-input px-2 py-1.5 font-mono text-xs"
            />
            <p className="mt-1 text-[11px] text-muted-foreground">
              这套配置的登录、会话、技能与 MCP 都放这里；全局配置不受影响。
            </p>
          </div>

          <div className="grid grid-cols-2 gap-3">
            <div>
              <label className="mb-1 block text-xs font-medium text-muted-foreground">接口地址（可选）</label>
              <input
                value={baseUrl}
                onChange={(event) => setBaseUrl(event.target.value)}
                data-testid="profile-base-url"
                placeholder="https://api.example.com"
                className="w-full rounded border border-input px-2 py-1.5 font-mono text-xs"
              />
            </div>
            <div>
              <label className="mb-1 block text-xs font-medium text-muted-foreground">鉴权变量名（可选）</label>
              <input
                value={tokenVar}
                onChange={(event) => setTokenVar(event.target.value)}
                data-testid="profile-token-var"
                placeholder="MY_API_TOKEN"
                className="w-full rounded border border-input px-2 py-1.5 font-mono text-xs"
              />
            </div>
          </div>
          <p className="text-[11px] text-muted-foreground">
            只填变量名，取值留在你的环境变量里：本工具的库与界面都不会保存或显示它。
          </p>

          <div className="grid grid-cols-2 gap-3">
            <div>
              <label className="mb-1 block text-xs font-medium text-muted-foreground">模型（可选）</label>
              <input
                value={model}
                onChange={(event) => setModel(event.target.value)}
                data-testid="profile-model"
                className="w-full rounded border border-input px-2 py-1.5 font-mono text-xs"
              />
            </div>
            <div>
              <label className="mb-1 block text-xs font-medium text-muted-foreground">启动参数（可选）</label>
              <input
                value={extraArgs}
                onChange={(event) => setExtraArgs(event.target.value)}
                data-testid="profile-extra-args"
                placeholder="--dangerously-skip-permissions"
                className="w-full rounded border border-input px-2 py-1.5 font-mono text-xs"
              />
            </div>
          </div>

          {presets.length > 0 && (
            <div className="space-y-1.5">
              <span className="block text-xs font-medium text-muted-foreground">常用参数（点一下加上或去掉）</span>
              <div className="flex flex-wrap gap-1.5">
                {presets.map((preset) => {
                  const on = hasPresetArgs(extraArgs, preset);
                  return (
                    <button
                      key={preset.id}
                      type="button"
                      data-testid={`profile-preset-${preset.id}`}
                      aria-pressed={on}
                      title={preset.hint}
                      onClick={() => setExtraArgs(togglePresetArgs(extraArgs, preset))}
                      className={cn(
                        "rounded border px-2 py-0.5 text-[11px]",
                        on
                          ? preset.danger
                            ? "border-warning/40 bg-warning/15 text-warning"
                            : "border-primary/30 bg-primary/10 text-primary"
                          : "border text-muted-foreground hover:bg-accent",
                      )}
                    >
                      {preset.label}
                    </button>
                  );
                })}
              </div>
              {activeDangerPreset && (
                <p data-testid="profile-preset-warning" className="rounded-md border border-warning/30 bg-warning/10 px-2 py-1.5 text-[11px] text-warning">
                  {activeDangerPreset.hint}
                </p>
              )}
            </div>
          )}

          <div>
            <span className="mb-1 block text-xs font-medium text-muted-foreground">启动方式</span>
            <pre data-testid="profile-launch-preview" className="overflow-x-auto rounded border bg-muted px-2 py-1.5 font-mono text-[11px] text-foreground">
              {launch ? `alias ${id}='${launch}'` : "填写实例名后显示"}
            </pre>
          </div>

          {error && (
            <p role="alert" data-testid="profile-form-error" className="text-xs text-destructive">
              {error}
            </p>
          )}
        </div>

        <div className="mt-5 flex items-center justify-end gap-2">
          <span className="mr-auto text-xs text-muted-foreground">保存只写定义，之后点「写入配置」才落地</span>
          <button type="button" onClick={onClose} className="rounded-md border px-4 py-2 text-sm text-muted-foreground hover:bg-accent">
            取消
          </button>
          <button
            type="button"
            onClick={submit}
            data-testid="profile-form-submit"
            className="rounded-md bg-primary px-4 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90"
          >
            保存
          </button>
        </div>
      </div>
    </div>
  );
}
