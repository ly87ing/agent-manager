import { useState } from "react";
import { ArrowLeft, Bot, Folder, KeyRound, Play, Terminal } from "lucide-react";
import { hasPresetArgs, presetsForCommand, togglePresetArgs } from "../lib/agentPresets";
import { cn } from "../lib/utils";
import {
  PROFILE_ID_RE,
  configEnvOf,
  defaultConfigDir,
  profileMcpConfig,
  profileLaunchValue,
  profileSkillsDir,
  type ProfileFormInput,
  type ProfileKind,
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
  baseAgents: { id: string; displayName: string; kind: ProfileKind }[];
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

  const selectedAgent = baseAgents.find((agent) => agent.id === baseAgent);
  const kind = selectedAgent?.kind ?? baseAgents[0]?.kind ?? "claude";
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
  const managedConfigFile = `${effectiveConfigDir.replace(/\/$/, "")}/${kind === "codex" ? "agent-manager.config.toml" : "settings.json"}`;
  const mcpConfigFile = profileMcpConfig(kind, effectiveConfigDir).path;
  const skillsDir = profileSkillsDir(effectiveConfigDir);
  /** 预设按该实例实际执行的命令给（实例的 bin 来自归属助手）。 */
  const presets = presetsForCommand(baseBinOf(baseAgent));
  const activeDangerPreset = presets.find((preset) => preset.danger && hasPresetArgs(extraArgs, preset));

  const submit = () => {
    if (!PROFILE_ID_RE.test(id)) {
      setError("环境名只能用小写字母、数字与连字符，且以字母或数字开头（它同时是终端里输入的命令名）");
      return;
    }
    if (!isEdit && existingIds.includes(id)) {
      setError("已有同名环境或助手，换一个名字");
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
      role="dialog"
      aria-modal="true"
      aria-label={isEdit ? "编辑独立环境" : "新建独立环境"}
      data-testid="profile-form-panel"
      className="fixed inset-0 z-50 flex flex-col bg-background animate-fade-in"
    >
      <header className="flex h-16 flex-none items-center border-b bg-card px-6">
        <div className="mx-auto flex w-full max-w-6xl items-center gap-4">
          <button
            type="button"
            onClick={onClose}
            aria-label="返回"
            className="flex h-10 w-10 items-center justify-center rounded-lg border bg-background text-muted-foreground hover:bg-accent hover:text-foreground"
          >
            <ArrowLeft className="h-5 w-5" aria-hidden />
          </button>
          <div>
            <h2 className="text-lg font-semibold text-foreground">{isEdit ? "编辑独立环境" : "新建独立环境"}</h2>
            <p className="text-xs text-muted-foreground">为同一个助手建立独立账号、接口与运行环境</p>
          </div>
        </div>
      </header>

      <main className="min-h-0 flex-1 overflow-y-auto">
        <div className="mx-auto w-full max-w-6xl px-6 py-6">
          <div className="grid items-start gap-5 min-[900px]:grid-cols-[minmax(0,1fr)_20rem]">
            <div className="space-y-5">
              <section className="rounded-xl border bg-card p-5 shadow-sm" aria-labelledby="profile-agent-heading">
                <div className="mb-4 flex items-start gap-3">
                  <span className="flex h-9 w-9 flex-none items-center justify-center rounded-lg bg-primary/10 text-primary">
                    <Bot className="h-5 w-5" aria-hidden />
                  </span>
                  <div>
                    <h3 id="profile-agent-heading" className="font-semibold text-foreground">1. 选择归属助手</h3>
                    <p className="mt-0.5 text-xs text-muted-foreground">独立环境会复用这个助手的程序，但使用自己的配置目录与登录状态。</p>
                  </div>
                </div>
                <div className="grid gap-2 sm:grid-cols-2" data-testid="profile-base-agent">
                  {baseAgents.map((agent) => {
                    const selected = agent.id === baseAgent;
                    return (
                      <button
                        key={agent.id}
                        type="button"
                        aria-pressed={selected}
                        disabled={isEdit}
                        onClick={() => setBaseAgent(agent.id)}
                        className={cn(
                          "flex min-h-12 items-center gap-3 rounded-lg border px-4 py-3 text-left transition-colors disabled:cursor-not-allowed disabled:opacity-70",
                          selected
                            ? "border-primary bg-primary/10 text-primary"
                            : "bg-background text-foreground hover:bg-accent",
                        )}
                      >
                        <Terminal className="h-4 w-4 flex-none" aria-hidden />
                        <span>
                          <span className="block text-sm font-medium">{agent.displayName}</span>
                          <span className="mt-0.5 block font-mono text-[11px] opacity-70">{baseBinOf(agent.id)}</span>
                        </span>
                      </button>
                    );
                  })}
                </div>
              </section>

              <section className="rounded-xl border bg-card p-5 shadow-sm" aria-labelledby="profile-identity-heading">
                <div className="mb-4 flex items-start gap-3">
                  <span className="flex h-9 w-9 flex-none items-center justify-center rounded-lg bg-violet/10 text-violet">
                    <Folder className="h-5 w-5" aria-hidden />
                  </span>
                  <div>
                    <h3 id="profile-identity-heading" className="font-semibold text-foreground">2. 命名与存放位置</h3>
                    <p className="mt-0.5 text-xs text-muted-foreground">命令名负责启动；显示名称只影响界面；所有独立数据都跟着配置目录走。</p>
                  </div>
                </div>

                <div className="grid gap-4 md:grid-cols-2">
                  <label className="block text-sm font-medium text-foreground" htmlFor="profile-id-input">
                    环境名
                    <span className="ml-2 text-xs font-normal text-muted-foreground">终端里输入它启动</span>
                    <input
                      id="profile-id-input"
                      value={id}
                      disabled={isEdit}
                      onChange={(event) => setId(event.target.value)}
                      data-testid="profile-id-input"
                      placeholder="cc-work"
                      className="mt-2 h-11 w-full rounded-lg border border-input bg-background px-3 font-mono text-sm disabled:bg-muted"
                    />
                  </label>
                  <label className="block text-sm font-medium text-foreground" htmlFor="profile-display-name">
                    显示名称
                    <span className="ml-2 text-xs font-normal text-muted-foreground">可选</span>
                    <input
                      id="profile-display-name"
                      value={displayName}
                      onChange={(event) => setDisplayName(event.target.value)}
                      data-testid="profile-display-name"
                      placeholder="Claude Code · 工作号"
                      className="mt-2 h-11 w-full rounded-lg border border-input bg-background px-3 text-sm"
                    />
                  </label>
                </div>

                <label className="mt-4 block text-sm font-medium text-foreground" htmlFor="profile-config-dir">
                  配置目录
                  <span className="ml-2 text-xs font-normal text-muted-foreground">留空使用下方默认值</span>
                  <input
                    id="profile-config-dir"
                    value={configDir}
                    onChange={(event) => setConfigDir(event.target.value)}
                    data-testid="profile-config-dir"
                    placeholder={defaultConfigDir(kind, id)}
                    className="mt-2 h-11 w-full rounded-lg border border-input bg-background px-3 font-mono text-sm"
                  />
                </label>
                <p className="mt-2 text-xs text-muted-foreground">登录、会话、技能与 MCP 都放在这里；不会覆盖助手的全局配置。</p>
              </section>

              <section className="rounded-xl border bg-card p-5 shadow-sm" aria-labelledby="profile-access-heading">
                <div className="mb-4 flex items-start gap-3">
                  <span className="flex h-9 w-9 flex-none items-center justify-center rounded-lg bg-info/10 text-info">
                    <KeyRound className="h-5 w-5" aria-hidden />
                  </span>
                  <div>
                    <h3 id="profile-access-heading" className="font-semibold text-foreground">3. 接口、账号与模型</h3>
                    <p className="mt-0.5 text-xs text-muted-foreground">使用官方登录时可以全部留空；使用第三方接口时再填写。</p>
                  </div>
                </div>

                <label className="block text-sm font-medium text-foreground" htmlFor="profile-base-url">
                  接口地址
                  <span className="ml-2 text-xs font-normal text-muted-foreground">可选</span>
                  <input
                    id="profile-base-url"
                    value={baseUrl}
                    onChange={(event) => setBaseUrl(event.target.value)}
                    data-testid="profile-base-url"
                    placeholder="https://api.example.com"
                    className="mt-2 h-11 w-full rounded-lg border border-input bg-background px-3 font-mono text-sm"
                  />
                </label>
                <div className="mt-4 grid gap-4 md:grid-cols-2">
                  <label className="block text-sm font-medium text-foreground" htmlFor="profile-token-var">
                    鉴权变量名
                    <span className="ml-2 text-xs font-normal text-muted-foreground">可选</span>
                    <input
                      id="profile-token-var"
                      value={tokenVar}
                      onChange={(event) => setTokenVar(event.target.value)}
                      data-testid="profile-token-var"
                      placeholder="WORK_API_TOKEN"
                      className="mt-2 h-11 w-full rounded-lg border border-input bg-background px-3 font-mono text-sm"
                    />
                  </label>
                  <label className="block text-sm font-medium text-foreground" htmlFor="profile-model">
                    模型
                    <span className="ml-2 text-xs font-normal text-muted-foreground">可选</span>
                    <input
                      id="profile-model"
                      value={model}
                      onChange={(event) => setModel(event.target.value)}
                      data-testid="profile-model"
                      placeholder="使用助手默认模型"
                      className="mt-2 h-11 w-full rounded-lg border border-input bg-background px-3 font-mono text-sm"
                    />
                  </label>
                </div>
                <p className="mt-3 rounded-lg border border-info/20 bg-info/5 px-3 py-2 text-xs text-muted-foreground">
                  这里只保存环境变量名，不保存密钥值。真正的密钥继续留在你的环境变量中。
                </p>
              </section>

              <section className="rounded-xl border bg-card p-5 shadow-sm" aria-labelledby="profile-launch-heading">
                <div className="mb-4 flex items-start gap-3">
                  <span className="flex h-9 w-9 flex-none items-center justify-center rounded-lg bg-orange/10 text-orange">
                    <Play className="h-5 w-5" aria-hidden />
                  </span>
                  <div>
                    <h3 id="profile-launch-heading" className="font-semibold text-foreground">4. 运行参数</h3>
                    <p className="mt-0.5 text-xs text-muted-foreground">常用模式可以一点选中；完整参数始终可以直接查看和编辑。</p>
                  </div>
                </div>

                {presets.length > 0 && (
                  <div className="mb-4 space-y-2">
                    <span className="block text-sm font-medium text-foreground">常用模式</span>
                    <div className="grid gap-2 sm:grid-cols-2">
                      {presets.map((preset) => {
                        const on = hasPresetArgs(extraArgs, preset);
                        return (
                          <button
                            key={preset.id}
                            type="button"
                            data-testid={`profile-preset-${preset.id}`}
                            aria-pressed={on}
                            onClick={() => setExtraArgs(togglePresetArgs(extraArgs, preset))}
                            className={cn(
                              "rounded-lg border px-3 py-3 text-left",
                              on
                                ? preset.danger
                                  ? "border-warning/50 bg-warning/10 text-warning"
                                  : "border-primary/40 bg-primary/10 text-primary"
                                : "bg-background text-foreground hover:bg-accent",
                            )}
                          >
                            <span className="block text-sm font-medium">{preset.label}</span>
                            <span className="mt-1 block text-xs opacity-75">{preset.hint}</span>
                          </button>
                        );
                      })}
                    </div>
                  </div>
                )}

                <label className="block text-sm font-medium text-foreground" htmlFor="profile-extra-args">
                  完整启动参数
                  <span className="ml-2 text-xs font-normal text-muted-foreground">可选，可自由修改</span>
                  <textarea
                    id="profile-extra-args"
                    value={extraArgs}
                    onChange={(event) => setExtraArgs(event.target.value)}
                    data-testid="profile-extra-args"
                    rows={4}
                    placeholder="例如：--dangerously-skip-permissions"
                    className="mt-2 w-full resize-y rounded-lg border border-input bg-background px-3 py-3 font-mono text-sm leading-6"
                  />
                </label>
                {activeDangerPreset && (
                  <p data-testid="profile-preset-warning" className="mt-3 rounded-lg border border-warning/30 bg-warning/10 px-3 py-2 text-xs text-warning">
                    {activeDangerPreset.hint}
                  </p>
                )}
              </section>

              {error && (
                <p role="alert" data-testid="profile-form-error" className="rounded-lg border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
                  {error}
                </p>
              )}
            </div>

            <aside className="space-y-4 min-[900px]:sticky min-[900px]:top-6" aria-label="独立环境结果预览">
              <section className="rounded-xl border bg-card p-5 shadow-sm">
                <div className="flex items-center gap-3">
                  <span className="flex h-9 w-9 items-center justify-center rounded-lg bg-success/10 text-success">
                    <Terminal className="h-5 w-5" aria-hidden />
                  </span>
                  <div>
                    <h3 className="font-semibold text-foreground">将创建的独立环境</h3>
                    <p className="text-xs text-muted-foreground">所有实际值随输入实时更新</p>
                  </div>
                </div>

                <dl className="mt-5 space-y-4 text-sm">
                  <div>
                    <dt className="text-xs text-muted-foreground">归属助手</dt>
                    <dd className="mt-1 font-medium text-foreground">{selectedAgent?.displayName ?? baseAgent}</dd>
                  </div>
                  <div>
                    <dt className="text-xs text-muted-foreground">配置目录变量</dt>
                    <dd className="mt-1 break-all font-mono text-xs text-foreground">{configEnvOf(kind)}={effectiveConfigDir}</dd>
                  </div>
                  <div>
                    <dt className="text-xs text-muted-foreground">运行配置</dt>
                    <dd className="mt-1 break-all font-mono text-xs text-foreground">{managedConfigFile}</dd>
                  </div>
                  <div>
                    <dt className="text-xs text-muted-foreground">MCP 配置</dt>
                    <dd className="mt-1 break-all font-mono text-xs text-foreground">{mcpConfigFile}</dd>
                  </div>
                  <div>
                    <dt className="text-xs text-muted-foreground">技能目录</dt>
                    <dd className="mt-1 break-all font-mono text-xs text-foreground">{skillsDir}</dd>
                  </div>
                  <div>
                    <dt className="text-xs text-muted-foreground">接口 / 模型 / 鉴权来源</dt>
                    <dd className="mt-1 space-y-1 font-mono text-xs text-foreground">
                      <span className="block break-all">{baseUrl.trim() || "助手默认接口"}</span>
                      <span className="block break-all">{model.trim() || "助手默认模型"}</span>
                      <span className="block break-all">{tokenVar.trim() ? `环境变量 ${tokenVar.trim()}` : "助手默认登录"}</span>
                    </dd>
                  </div>
                </dl>
              </section>

              <section className="rounded-xl border bg-card p-5 shadow-sm">
                <h3 className="text-sm font-semibold text-foreground">终端启动命令</h3>
                <pre data-testid="profile-launch-preview" className="mt-3 whitespace-pre-wrap break-all rounded-lg border bg-muted px-3 py-3 font-mono text-xs leading-5 text-foreground">
                  {launch ? `alias ${id}='${launch}'` : "填写环境名后显示完整命令"}
                </pre>
              </section>

              <p className="rounded-xl border border-primary/20 bg-primary/5 px-4 py-3 text-xs leading-5 text-muted-foreground">
                保存只记录这套定义，不会改配置文件。回到助手页点击「写入配置」后才会落地。
              </p>
            </aside>
          </div>
        </div>
      </main>

      <footer className="flex-none border-t bg-card px-6 py-4">
        <div className="mx-auto flex w-full max-w-6xl items-center justify-between gap-4">
          <span className="hidden text-xs text-muted-foreground sm:block">保存定义后，你仍可在写入前继续检查和修改</span>
          <div className="ml-auto flex items-center gap-3">
            <button type="button" onClick={onClose} className="rounded-lg border bg-background px-5 py-2.5 text-sm text-muted-foreground hover:bg-accent hover:text-foreground">
              取消
            </button>
            <button
              type="button"
              onClick={submit}
              data-testid="profile-form-submit"
              className="rounded-lg bg-primary px-5 py-2.5 text-sm font-medium text-primary-foreground hover:bg-primary/90"
            >
              {isEdit ? "保存修改" : "创建独立环境"}
            </button>
          </div>
        </div>
      </footer>
    </div>
  );
}
