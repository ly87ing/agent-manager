import { useEffect, useRef, useState } from "react";
import { ArrowLeft, Bot, CopyPlus, Folder, KeyRound, Play, Terminal } from "lucide-react";
import { hasPresetArgs, presetsForCommand, togglePresetArgs } from "../lib/agentPresets";
import { cn } from "../lib/utils";
import { useDialogFocus } from "../lib/dialogFocus";
import { detectSystemPlatform, urlHasUserinfo } from "../lib/agentDefinitionForm";
import { ALL_SEED_ITEMS, type SeedItem } from "../lib/profileSeed";
import { keyStatus, type SecretSourceState } from "../lib/secretSource";
import { ProfileSeedPicker } from "./ProfileSeedPicker";
import type { AliasFilePreview } from "../lib/useAliasSource";
import {
  PROFILE_ID_RE,
  SUBPROCESS_SCRUB_ENV,
  buildProfileDefinition,
  configEnvOf,
  defaultConfigDir,
  isThirdPartyProfile,
  profileMcpConfig,
  profileLaunchValue,
  profileSkillsDir,
  type ProfileDraftPreview,
  type ProfileFormInput,
  type ProfileKind,
} from "../lib/profileDefinition";

/**
 * 助手实例表单：为同一个助手再配一套独立配置。
 *
 * 实例有自己的配置目录，启动时用环境变量指过去，所以多个实例可以同时开着互不干扰，
 * 全局配置一个字节都不动。鉴权只填变量名，库里不存取值；写入时解析成取值，只写进这个实例
 * 自己的配置文件。
 */
/** Claude Code 认作「打开」的环境变量取值（与官方 isEnvTruthy 一致：1 / true / yes / on）。 */
function envTruthy(value: string): boolean {
  return ["1", "true", "yes", "on"].includes(value.trim().toLowerCase());
}

export function ProfileFormDialog({
  baseAgents,
  initial,
  isEdit: isEditProp,
  existingIds,
  baseBinOf,
  onSubmit,
  onPreview,
  onAliasPreview,
  aliasPreviewBase,
  onFetchModels,
  secretSource,
  onClose,
}: {
  /** 可作为归属的助手（目前支持 Claude Code 与 Codex）。 */
  baseAgents: { id: string; displayName: string; kind: ProfileKind }[];
  initial?: ProfileFormInput;
  /** 复制实例时带着预填内容但仍是新建：名称与配置目录必须可改。 */
  isEdit?: boolean;
  existingIds: string[];
  baseBinOf: (agentId: string) => string;
  /** 新建时另带要从默认环境带上的设置（编辑时为空）。 */
  onSubmit: (input: ProfileFormInput, options: { seedItems: SeedItem[]; isEdit: boolean }) => void;
  onPreview: (input: ProfileFormInput, seedItems: SeedItem[]) => Promise<ProfileDraftPreview | string>;
  onAliasPreview: (name: string, value: string) => Promise<AliasFilePreview | string>;
  aliasPreviewBase: string;
  /** 用表单里的地址与凭据变量问接口能用哪些模型；失败时返回原因。只在点按钮时才联网。 */
  onFetchModels?: (draft: { kind: ProfileKind; baseUrl: string; tokenVar: string }) => Promise<string[] | string>;
  /** 密钥文件的状态：说明这个变量现在有没有值。 */
  secretSource?: SecretSourceState | null;
  onClose: () => void;
}) {
  const [fetchedModels, setFetchedModels] = useState<string[] | null>(null);
  const [modelsMessage, setModelsMessage] = useState<string | null>(null);
  const [fetchingModels, setFetchingModels] = useState(false);
  const [previewBusy, setPreviewBusy] = useState(false);
  const previewSerial = useRef(0);
  const [preview, setPreview] = useState<{ key: string; result: ProfileDraftPreview | string } | null>(null);
  const isEdit = isEditProp ?? Boolean(initial);
  const firstAgent = baseAgents[0]?.id ?? "claude-code";
  const [id, setId] = useState(initial?.id ?? "");
  const [baseAgent, setBaseAgent] = useState(initial?.baseAgent ?? firstAgent);
  const [displayName, setDisplayName] = useState(initial?.displayName ?? "");
  const [configDir, setConfigDir] = useState(initial?.configDir ?? "");
  const [baseUrl, setBaseUrl] = useState(initial?.baseUrl ?? "");
  const [tokenVar, setTokenVar] = useState(initial?.tokenVar ?? "");
  const [authHeader, setAuthHeader] = useState<ProfileFormInput["authHeader"]>(initial?.authHeader ?? "token");
  /** 表单不单独编辑的 env（小模型、模型别名、超时等），原样带回定义。 */
  const initialExtraEnv = initial?.extraEnv ?? {};
  /**
   * 子进程看不到凭据（官方 CLAUDE_CODE_SUBPROCESS_ENV_SCRUB）：null 表示跟随默认——第三方环境开、
   * 官方账号环境关。第三方凭据写在这个环境 settings.json 的 env 里，不开的话 Bash 工具、hooks 与
   * MCP 服务都能读到它。
   */
  const initialScrub = initialExtraEnv[SUBPROCESS_SCRUB_ENV];
  const initialScrubOn = initialScrub !== undefined && envTruthy(initialScrub);
  // 已有环境按它现在的值；新建（含从接入建成、复制）没写这个变量时跟随默认
  const [scrubChoice, setScrubChoice] = useState<boolean | null>(
    initialScrub !== undefined ? initialScrubOn : isEdit ? false : null,
  );
  const [model, setModel] = useState(initial?.model ?? "");
  const [extraArgs, setExtraArgs] = useState(initial?.extraArgs ?? "");
  /** 新建时从默认环境带上的设置：默认全选，与默认环境用起来一样；编辑已有环境时不在表单里带。 */
  const [seedItems, setSeedItems] = useState<SeedItem[]>(ALL_SEED_ITEMS);
  const [error, setError] = useState<string | null>(null);
  const { dialogRef, onDialogKeyDown } = useDialogFocus<HTMLDivElement>({ onEscape: onClose });

  const selectedAgent = baseAgents.find((agent) => agent.id === baseAgent);
  const kind = selectedAgent?.kind ?? baseAgents[0]?.kind ?? "claude";
  const effectiveConfigDir = configDir.trim() || defaultConfigDir(kind, id);
  const thirdParty = isThirdPartyProfile({ baseUrl, tokenVar });
  // Linux 上官方实现要用 bubblewrap 隔离子进程，没装时开了这项 Claude 会拒绝启动（2.1.281 实测）；
  // macOS 版不需要。所以只在非 Linux 上默认开
  const onLinux = detectSystemPlatform() === "linux";
  const scrub = kind === "claude" && (scrubChoice ?? (thirdParty && !onLinux));
  const extraEnv: Record<string, string> = { ...initialExtraEnv };
  // 打开时保留原写法（如 "true"），关掉就删掉
  if (scrub) extraEnv[SUBPROCESS_SCRUB_ENV] = initialScrubOn ? initialScrub : "1";
  else delete extraEnv[SUBPROCESS_SCRUB_ENV];
  const input: ProfileFormInput = {
    id,
    displayName,
    baseAgent,
    sourceConnection: baseAgent === initial?.baseAgent ? initial?.sourceConnection : undefined,
    kind,
    configDir: effectiveConfigDir,
    baseUrl,
    tokenVar,
    authHeader,
    model,
    extraArgs,
    extraEnv,
  };
  const launch = id ? profileLaunchValue(input, baseBinOf(baseAgent)) : "";
  const aliasPreviewKey = JSON.stringify([aliasPreviewBase, id, launch]);
  const [aliasPreview, setAliasPreview] = useState<{ key: string; result: AliasFilePreview | string } | null>(null);
  const previewKey = JSON.stringify([buildProfileDefinition(input, baseBinOf(baseAgent)), isEdit ? [] : seedItems]);
  const previewReady = PROFILE_ID_RE.test(id)
    && (isEdit || !existingIds.includes(id))
    && /^(~\/|\/)/.test(effectiveConfigDir)
    && (!baseUrl.trim() || (/^https?:\/\/[^/]/.test(baseUrl.trim()) && !urlHasUserinfo(baseUrl)))
    && (!tokenVar.trim() || /^[A-Za-z_][A-Za-z0-9_]*$/.test(tokenVar.trim()))
    && (kind !== "claude" || !baseUrl.trim() || Boolean(tokenVar.trim()))
    && (kind !== "codex" || !tokenVar.trim() || Boolean(baseUrl.trim()));
  const waitingForPreview = !isEdit && previewReady
    && (previewBusy || preview?.key !== previewKey || typeof preview.result === "string"
      || aliasPreview?.key !== aliasPreviewKey || typeof aliasPreview.result === "string");
  const managedConfigFile = `${effectiveConfigDir.replace(/\/$/, "")}/${kind === "codex" ? "config.toml" : "settings.json"}`;
  const mcpConfigFile = profileMcpConfig(kind, effectiveConfigDir).path;
  const skillsDir = profileSkillsDir(effectiveConfigDir);
  /** 预设按该实例实际执行的命令给（实例的 bin 来自归属助手）。 */
  const presets = presetsForCommand(baseBinOf(baseAgent));
  const activeDangerPreset = presets.find((preset) => preset.danger && hasPresetArgs(extraArgs, preset));

  const refreshPreview = async () => {
    const serial = ++previewSerial.current;
    setPreviewBusy(true);
    try {
      const result = await onPreview(input, isEdit ? [] : seedItems);
      if (serial === previewSerial.current) setPreview({ key: previewKey, result });
    } finally {
      if (serial === previewSerial.current) setPreviewBusy(false);
    }
  };

  useEffect(() => {
    if (!previewReady) {
      setPreviewBusy(false);
      return;
    }
    setPreviewBusy(true);
    const timer = window.setTimeout(() => void refreshPreview(), 300);
    return () => {
      window.clearTimeout(timer);
      previewSerial.current += 1;
    };
  }, [previewKey, previewReady]);

  useEffect(() => {
    if (isEdit || !previewReady || !launch) return;
    let current = true;
    const timer = window.setTimeout(() => {
      void onAliasPreview(id, launch)
        .then((result) => { if (current) setAliasPreview({ key: aliasPreviewKey, result }); })
        .catch((error) => { if (current) setAliasPreview({ key: aliasPreviewKey, result: String(error) }); });
    }, 300);
    return () => { current = false; window.clearTimeout(timer); };
  }, [aliasPreviewKey, isEdit, previewReady]);

  const currentAliasPreview = !isEdit && previewReady && aliasPreview?.key === aliasPreviewKey
    ? aliasPreview.result : null;

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
    if (baseUrl.trim() && !/^https?:\/\/[^/]/.test(baseUrl.trim())) {
      setError("接口地址要以 https:// 或 http:// 开头");
      return;
    }
    if (urlHasUserinfo(baseUrl)) {
      setError("接口地址里不能带账号密码：凭据请用鉴权变量");
      return;
    }
    if (tokenVar.trim() && !/^[A-Za-z_][A-Za-z0-9_]*$/.test(tokenVar.trim())) {
      setError("鉴权变量名不合法：要以字母或下划线开头，只能用字母、数字与下划线");
      return;
    }
    if (kind === "claude" && baseUrl.trim() && !tokenVar.trim()) {
      setError("填了接口地址就要填鉴权变量名：否则 Claude 会把这个环境里的官方账号登录发往该地址");
      return;
    }
    if (kind === "codex" && tokenVar.trim() && !baseUrl.trim()) {
      setError("填了鉴权变量名就要填接口地址：凭据只配合这个环境自己的接口使用");
      return;
    }
    onSubmit(input, { seedItems: isEdit ? [] : seedItems, isEdit });
  };

  return (
    <div
      ref={dialogRef}
      role="dialog"
      aria-modal="true"
      aria-label={isEdit ? "编辑独立环境" : "新建独立环境"}
      data-testid="profile-form-panel"
      tabIndex={-1}
      onKeyDown={onDialogKeyDown}
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
                {isEdit && initial?.configDir && configDir.trim() && configDir.trim().replace(/\/$/, "") !== initial.configDir.replace(/\/$/, "") && (
                  <p data-testid="profile-config-dir-moved" className="mt-2 rounded-lg border border-warning/30 bg-warning/10 px-3 py-2 text-xs text-warning">
                    换了配置目录：官方登录按目录保存（macOS 钥匙串条目也按目录路径区分），原目录里的登录、会话与技能不会跟过来，需要在新目录重新登录；保存定义不会改已有启动命令，请回到环境卡片单独修改。之后不再使用原目录的登录时，先用原来的退出命令在终端退出。
                  </p>
                )}
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
                    API Key 所在的变量
                    <span className="ml-2 text-xs font-normal text-muted-foreground">第三方环境必填</span>
                    <input
                      id="profile-token-var"
                      value={tokenVar}
                      onChange={(event) => setTokenVar(event.target.value)}
                      data-testid="profile-token-var"
                      placeholder="WORK_API_TOKEN"
                      className="mt-2 h-11 w-full rounded-lg border border-input bg-background px-3 font-mono text-sm"
                    />
                    {tokenVar.trim() && secretSource !== undefined && (() => {
                      const status = keyStatus(secretSource, tokenVar);
                      return (
                        <span
                          data-testid="profile-key-status"
                          className={cn(
                            "mt-1 block text-xs font-normal",
                            status.tone === "ok" ? "text-success" : status.tone === "warn" ? "text-warning" : "text-muted-foreground",
                          )}
                        >
                          {status.tone === "ok"
                            ? `密钥文件里已有 ${tokenVar.trim()}，写入环境时用它的值。`
                            : status.tone === "warn" && secretSource?.configured && secretSource.ready
                              ? `密钥文件里还没有 ${tokenVar.trim()}：到「设置 → 密钥」填上，或用「新建环境 → 第三方厂商」在表单里直接填 API Key。`
                              : status.text}
                        </span>
                      );
                    })()}
                  </label>
                  <label className="block text-sm font-medium text-foreground" htmlFor="profile-model">
                    模型
                    <span className="ml-2 text-xs font-normal text-muted-foreground">可选</span>
                    {onFetchModels && (
                      <button
                        type="button"
                        data-testid="profile-fetch-models"
                        disabled={fetchingModels || !baseUrl.trim() || !tokenVar.trim()}
                        title="用上面的地址与凭据变量问接口能用哪些模型；只在点它时联网，不改任何配置"
                        onClick={async (event) => {
                          event.preventDefault();
                          setFetchingModels(true);
                          setModelsMessage(null);
                          try {
                            const result = await onFetchModels({ kind, baseUrl: baseUrl.trim(), tokenVar: tokenVar.trim() });
                            setFetchedModels(typeof result === "string" ? null : result);
                            setModelsMessage(typeof result === "string" ? result : `已获取 ${result.length} 个模型，可在输入框下拉挑选`);
                          } finally {
                            setFetchingModels(false);
                          }
                        }}
                        className="ml-2 rounded border px-2 py-0.5 text-xs font-normal text-muted-foreground hover:bg-accent disabled:opacity-50"
                      >
                        {fetchingModels ? "正在获取…" : "获取模型列表"}
                      </button>
                    )}
                    <input
                      id="profile-model"
                      value={model}
                      onChange={(event) => setModel(event.target.value)}
                      data-testid="profile-model"
                      placeholder="使用助手默认模型"
                      list={fetchedModels ? "profile-model-options" : undefined}
                      className="mt-2 h-11 w-full rounded-lg border border-input bg-background px-3 font-mono text-sm"
                    />
                    {modelsMessage && (
                      <span data-testid="profile-models-message" className={cn("mt-1 block text-xs font-normal", fetchedModels ? "text-muted-foreground" : "text-destructive")}>
                        {modelsMessage}
                      </span>
                    )}
                    {fetchedModels && (
                      <datalist id="profile-model-options">
                        {fetchedModels.map((item) => (
                          <option key={item} value={item} />
                        ))}
                      </datalist>
                    )}
                  </label>
                </div>
                {kind === "claude" && (
                  <label className="mt-4 block text-sm font-medium text-foreground" htmlFor="profile-auth-header">
                    凭据请求头
                    <select
                      id="profile-auth-header"
                      value={authHeader}
                      onChange={(event) => setAuthHeader(event.target.value === "api_key" ? "api_key" : "token")}
                      data-testid="profile-auth-header"
                      className="mt-2 h-11 w-full rounded-lg border border-input bg-background px-3 text-sm"
                    >
                      <option value="token">Authorization: Bearer（ANTHROPIC_AUTH_TOKEN）</option>
                      <option value="api_key">x-api-key（ANTHROPIC_API_KEY）</option>
                    </select>
                    {authHeader === "api_key" && (
                      <span data-testid="profile-api-key-approval" className="mt-1 block text-xs font-normal text-muted-foreground">
                        按官方说明，交互模式第一次启动会询问是否使用这个 API key，选「是」后会记住；不确定用哪种时选 Bearer（官方网关文档的默认建议）。
                      </span>
                    )}
                  </label>
                )}
                {kind === "claude" && (
                  <label className="mt-4 flex items-start gap-2 text-sm text-foreground">
                    <input
                      type="checkbox"
                      data-testid="profile-scrub-subprocess-env"
                      checked={scrub}
                      onChange={(event) => setScrubChoice(event.target.checked)}
                      className="mt-1"
                    />
                    <span>
                      子进程看不到凭据
                      <span className="mt-0.5 block text-xs text-muted-foreground">
                        官方设置 CLAUDE_CODE_SUBPROCESS_ENV_SCRUB：Bash 工具、hooks 与 MCP 服务读不到这个环境的凭据，防提示注入把令牌带出去；
                        Claude 自己照常用它发请求。代价：子进程也看不到 CLAUDE_CONFIG_DIR，Linux 上 Bash 子进程看不到本机其他进程。
                        {onLinux ? "" : "第三方环境默认开启。"}
                      </span>
                      {onLinux && (
                        <span data-testid="profile-scrub-linux-bwrap" className="mt-0.5 block text-xs text-warning">
                          Linux 上要先安装 bubblewrap（bwrap）：没装时开了这项，Claude 在这个环境里会拒绝启动。因此这里默认不开。
                        </span>
                      )}
                    </span>
                  </label>
                )}
                {baseUrl.trim().startsWith("http://") && (
                  <p data-testid="profile-plain-http" className="mt-3 text-xs text-warning">非 https 地址：令牌将明文传输。</p>
                )}
                {Object.keys(extraEnv).filter((name) => name !== SUBPROCESS_SCRUB_ENV).length > 0 && (
                  <p data-testid="profile-extra-env" className="mt-3 text-xs text-muted-foreground">
                    另外保留的环境变量：<span className="font-mono">{Object.keys(extraEnv).filter((name) => name !== SUBPROCESS_SCRUB_ENV).sort().join("、")}</span>
                  </p>
                )}
                <p className="mt-3 rounded-lg border border-info/20 bg-info/5 px-3 py-2 text-xs text-muted-foreground">
                  这里只保存变量名，不保存密钥值。写入环境配置时从「设置 → 密钥」选择的外部文件取值，写进这个环境自己的配置文件（仅本人可读），启动时不用在终端里导出变量。
                </p>
              </section>

              {!isEdit && <section className="rounded-xl border bg-card p-5 shadow-sm" aria-labelledby="profile-launch-heading">
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
              </section>}

              {!isEdit && (
                <section className="rounded-xl border bg-card p-5 shadow-sm" aria-labelledby="profile-seed-heading" data-testid="profile-seed-section">
                  <div className="mb-4 flex items-start gap-3">
                    <span className="flex h-9 w-9 flex-none items-center justify-center rounded-lg bg-primary/10 text-primary">
                      <CopyPlus className="h-5 w-5" aria-hidden />
                    </span>
                    <div>
                      <h3 id="profile-seed-heading" className="font-semibold text-foreground">5. 从默认环境带上</h3>
                      <p className="mt-0.5 text-xs text-muted-foreground">
                        独立环境不读主配置里的设置。勾选的项在创建后从默认环境复制一份进来，之后也可以在卡片上再带。
                      </p>
                    </div>
                  </div>
                  <ProfileSeedPicker kind={kind} selected={seedItems} onChange={setSeedItems} testIdPrefix="profile-seed" />
                </section>
              )}

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

              {!isEdit && <section className="rounded-xl border bg-card p-5 shadow-sm">
                <h3 className="text-sm font-semibold text-foreground">终端启动命令</h3>
                <pre data-testid="profile-launch-preview" className="mt-3 whitespace-pre-wrap break-all rounded-lg border bg-muted px-3 py-3 font-mono text-xs leading-5 text-foreground">
                  {launch ? `alias ${id}='${launch}'` : "填写环境名后显示完整命令"}
                </pre>
                <div className="mt-3 space-y-2 text-xs" data-testid="profile-alias-file-preview">
                  <p className="font-medium">终端配置文件写入后</p>
                  {!previewReady && <p className="text-muted-foreground">填写有效环境名和配置后显示完整文件。</p>}
                  {previewReady && currentAliasPreview === null && <p className="text-muted-foreground">正在读取实际文件并计算…</p>}
                  {typeof currentAliasPreview === "string" && <p role="alert" className="text-destructive">{currentAliasPreview}</p>}
                  {currentAliasPreview && typeof currentAliasPreview !== "string" && <>
                    {currentAliasPreview.files.map((file) => <div key={file.target} className="space-y-1">
                      <p className="break-all font-mono">{file.path}{file.exists ? "（已有文件）" : "（将新建）"}</p>
                      <pre data-testid={`profile-alias-file-${file.target}`} className="max-h-48 overflow-auto whitespace-pre-wrap break-all rounded bg-muted p-2 font-mono">{file.content}</pre>
                    </div>)}
                    {currentAliasPreview.skipped.map((reason) => <p key={reason} className="text-muted-foreground">{reason}</p>)}
                  </>}
                  <p className="text-muted-foreground">只读预览；原文可能含敏感值，仅在本机查看。</p>
                </div>
              </section>}

              <section className="rounded-xl border bg-card p-5 shadow-sm">
                <div className="flex flex-wrap items-center justify-between gap-2">
                  <h3 className="text-sm font-semibold text-foreground">目标配置文件预览</h3>
                  <button type="button" data-testid="profile-draft-preview-button"
                    disabled={previewBusy || !previewReady}
                    onClick={() => void refreshPreview()}
                    className="rounded border px-2.5 py-1 text-xs disabled:opacity-50"
                  >{previewBusy ? "正在预览…" : "重新读取"}</button>
                </div>
                <p className="mt-2 text-xs text-muted-foreground">{isEdit
                  ? "只读预览这套定义对应的目标配置；保存编辑只更新定义，不会写入下列文件。需要生效时请回到环境卡片对比并写入。预览可能含敏感值，请勿直接分享。"
                  : "按保存流程预览目标文件及勾选的共用设置。新建文件的凭据仍以变量引用展示；已有主文件不会自动覆盖，保存后可在环境卡片逐项应用接入字段。预览可能含敏感值，请勿直接分享。"}</p>
                {kind === "codex" && !thirdParty && <p className="mt-1 text-xs text-muted-foreground">Codex 官方接入本身不改 config.toml；勾选的共用设置仍可能写入。</p>}
                {preview && preview.key !== previewKey && previewReady && <p className="mt-2 text-xs text-muted-foreground">表单已更改，正在更新预览。</p>}
                {preview?.key === previewKey && (typeof preview.result === "string"
                  ? <p role="alert" className="mt-2 text-xs text-destructive">{preview.result}</p>
                  : <div className="mt-2 space-y-2">
                      <p className="break-all font-mono text-xs text-muted-foreground">{preview.result.path}{preview.result.will_write === false ? (preview.result.exists ? " · 保持现有文件" : " · 不创建文件") : (preview.result.existing_profile_skipped ? " · 仅补勾选的共用设置" : (preview.result.exists ? " · 合并现有文件" : " · 新文件"))}</p>
                      <pre data-testid="profile-draft-preview-content" className="max-h-80 overflow-auto whitespace-pre-wrap break-all rounded border bg-muted p-3 font-mono text-xs">{preview.result.content}</pre>
                      {preview.result.files?.map((file, index) => <div key={file.path} className="space-y-1">
                        <p className="break-all font-mono text-xs text-muted-foreground">{file.path}{file.exists ? " · 合并现有文件" : " · 新文件"}</p>
                        <pre data-testid={`profile-draft-preview-file-${index}`} className="max-h-80 overflow-auto whitespace-pre-wrap break-all rounded border bg-muted p-3 font-mono text-xs">{file.content}</pre>
                      </div>)}
                      {preview.result.seed_results?.map((item) => <p key={item.item} className="text-xs text-muted-foreground">{item.detail}</p>)}
                    </div>)}
              </section>

              <p className="rounded-xl border border-primary/20 bg-primary/5 px-4 py-3 text-xs leading-5 text-muted-foreground">
                {isEdit
                  ? "保存只更新这套定义；实际配置文件与终端启动命令都不会改。回到卡片上对比配置、单独编辑启动命令。"
                  : "创建后写好这个环境的配置（Codex 官方账号环境没有要写的）与启动命令，再按勾选从默认环境带上设置。"}
              </p>
            </aside>
          </div>
        </div>
      </main>

      <footer className="flex-none border-t bg-card px-6 py-4">
        <div className="mx-auto flex w-full max-w-6xl items-center justify-between gap-4">
          <span className="hidden text-xs text-muted-foreground sm:block">之后可以在卡片上继续检查、修改与写入</span>
          <div className="ml-auto flex items-center gap-3">
            <button type="button" onClick={onClose} className="rounded-lg border bg-background px-5 py-2.5 text-sm text-muted-foreground hover:bg-accent hover:text-foreground">
              取消
            </button>
            <button
              type="button"
              onClick={submit}
              data-testid="profile-form-submit"
              disabled={waitingForPreview}
              className="rounded-lg bg-primary px-5 py-2.5 text-sm font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
            >
              {isEdit ? "保存修改" : "创建独立环境"}
            </button>
          </div>
        </div>
      </footer>
    </div>
  );
}
