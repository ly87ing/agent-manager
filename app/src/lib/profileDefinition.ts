/**
 * 助手实例（profile）的定义生成与回填（纯函数）。
 *
 * 一个实例 = 一个独立配置目录 + 一条启动命令。实例定义就是一条 agent 定义（多一个 profile 段），
 * 所以技能与 MCP 的逐助手分配天然适用于它。
 *
 * 目标文件分工：
 * - Claude：本工具写 `<配置目录>/settings.json`，MCP 写 `<配置目录>/.claude.json`
 * - Codex：本工具与 MCP 都写 `<配置目录>/config.toml`，各自按键原位修改——本工具只动
 *   `model`、`model_provider` 与 `[model_providers.agent-manager]`，MCP 只动 `mcp_servers`
 */

import type { ConnectionEntry } from "./agentDefinitionForm";

export type ProfileKind = "claude" | "codex";


export interface ProfileFormInput {
  id: string;
  displayName: string;
  baseAgent: string;
  /** 建立该环境的 Provider 唯一名称；只用于关联，不写助手配置。 */
  sourceConnection?: string;
  kind: ProfileKind;
  configDir: string;
  /** 接口地址，留空表示用该助手的默认地址。 */
  baseUrl: string;
  /** 鉴权取值所在的环境变量名；库里只存这个名字，永远不存取值。 */
  tokenVar: string;
  /** Claude 凭据走哪个请求头：token = Authorization: Bearer，api_key = x-api-key。 */
  authHeader: "token" | "api_key";
  model: string;
  /** 启动命令每次都带上的参数。 */
  extraArgs: string;
  /** 表单不单独编辑的其他 env 变量（小模型、模型别名、超时等），编辑时原样保留。 */
  extraEnv: Record<string, string>;
}

/** 官方设置：子进程（Bash 工具、hooks、MCP 服务）的环境里去掉凭据。第三方环境默认开启。 */
export const SUBPROCESS_SCRUB_ENV = "CLAUDE_CODE_SUBPROCESS_ENV_SCRUB";

/** 表单自己管理的 Claude env 变量；其余 env 进 extraEnv 原样往返。 */
const CLAUDE_FORM_ENV = ["ANTHROPIC_BASE_URL", "ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_API_KEY"];

/** Claude 凭据请求头对应的变量名（官方网关文档的两种写法）。 */
export function claudeCredentialEnv(authHeader: ProfileFormInput["authHeader"]): string {
  return authHeader === "api_key" ? "ANTHROPIC_API_KEY" : "ANTHROPIC_AUTH_TOKEN";
}

/** 实例名规则：同时是启动命令名，与别名域一致。 */
export const PROFILE_ID_RE = /^[a-z0-9][a-z0-9-]*$/;

export function profileKindOf(baseAgent: string): ProfileKind {
  return baseAgent === "codex" ? "codex" : "claude";
}

/** 默认配置目录：`~/.claude-<名称>` / `~/.codex-<名称>`，一眼看出是哪个助手的哪套配置。 */
export function defaultConfigDir(kind: ProfileKind, id: string): string {
  return `~/.${kind}-${id || "new"}`;
}

/** 该实例的技能目录：随配置目录走（助手会在配置目录下找 skills）。 */
export function profileSkillsDir(configDir: string): string {
  return `${configDir.replace(/\/$/, "")}/skills`;
}

/** 该实例的 MCP 目标文件：Claude 是 .claude.json，Codex 是 config.toml。 */
export function profileMcpConfig(kind: ProfileKind, configDir: string): { path: string; format: string } {
  const dir = configDir.replace(/\/$/, "");
  return kind === "codex"
    ? { path: `${dir}/config.toml`, format: "toml" }
    : { path: `${dir}/.claude.json`, format: "json" };
}

/** 生成整条实例定义（保存到 agents.definitions.<id>）。 */
export function buildProfileDefinition(input: ProfileFormInput, baseBin: string): Record<string, unknown> {
  const configDir = input.configDir.replace(/\/$/, "");
  const profile: Record<string, unknown> = {
    base_agent: input.baseAgent,
    kind: input.kind,
    config_dir: configDir,
  };
  if (input.sourceConnection) profile.source_connection = input.sourceConnection;
  if (input.kind === "claude") {
    const env: Record<string, string> = {};
    for (const [name, value] of Object.entries(input.extraEnv)) {
      if (!CLAUDE_FORM_ENV.includes(name)) env[name] = value;
    }
    if (input.baseUrl.trim()) env.ANTHROPIC_BASE_URL = input.baseUrl.trim();
    if (input.tokenVar.trim()) env[claudeCredentialEnv(input.authHeader)] = `\${${input.tokenVar.trim()}}`;
    if (Object.keys(env).length > 0) profile.env = env;
  } else {
    if (input.baseUrl.trim()) profile.base_url = input.baseUrl.trim();
    if (input.tokenVar.trim()) profile.env_key = input.tokenVar.trim();
    profile.wire_api = "responses";
  }
  if (input.model.trim()) profile.model = input.model.trim();

  return {
    display_name: input.displayName.trim() || input.id,
    bin: baseBin,
    skills_dir: profileSkillsDir(configDir),
    mcp_config: profileMcpConfig(input.kind, configDir),
    profile,
  };
}

/** 实例的配置目录环境变量名。 */
export function configEnvOf(kind: ProfileKind): string {
  return kind === "codex" ? "CODEX_HOME" : "CLAUDE_CONFIG_DIR";
}

/**
 * 配置目录写进 shell 命令时的形式：`~` 换成 `$HOME`。
 *
 * 双引号里的 `~` 不会被 shell 展开，`CLAUDE_CONFIG_DIR="~/.claude-x"` 取到的是
 * 字面量，助手会在当前目录下建一个名叫 `~` 的文件夹——和本工具写配置的绝对路径
 * 根本不是同一处。`"$HOME/..."` 在双引号里照常展开，路径带空格也不会被拆开。
 */
export function shellConfigDir(configDir: string): string {
  const trimmed = configDir.replace(/\/$/, "");
  if (trimmed === "~") return "$HOME";
  if (trimmed.startsWith("~/")) return `$HOME/${trimmed.slice(2)}`;
  return trimmed;
}

/**
 * Codex 环境启动时清空的变量：Codex 把它们当作官方账号侧的凭据，优先于目录里存的登录，
 * 空值等同未设置。官方账号环境同样清空，否则 shell 里给别的账号导出的令牌会压过这个目录的登录。
 * 与引擎侧保持一致。
 */
export const CODEX_CLEARED_ENV = ["CODEX_API_KEY", "CODEX_ACCESS_TOKEN"] as const;

/** 用第三方接口的环境：这类环境的配置目录里不能有官方账号登录。 */
export function isThirdPartyProfile(input: Pick<ProfileFormInput, "baseUrl" | "tokenVar">): boolean {
  return Boolean(input.baseUrl.trim() || input.tokenVar.trim());
}

/** 该实例的启动命令取值（与别名域取值格式一致）。 */
export function profileLaunchValue(input: ProfileFormInput, baseBin: string): string {
  const parts: string[] = [];
  if (input.kind === "codex") {
    parts.push(...CODEX_CLEARED_ENV.map((name) => `${name}=`));
  }
  parts.push(`${configEnvOf(input.kind)}="${shellConfigDir(input.configDir)}"`, baseBin);
  const extra = input.extraArgs.trim();
  if (extra) parts.push(extra);
  return parts.join(" ");
}

/**
 * 生成由用户在终端亲自执行的官方认证命令。本应用只复制文本，绝不代为启动登录或退出。
 * 实例必须带独立配置目录，避免认证写到主账号；主助手省略 configDir。
 */
export function authTerminalCommand(
  kind: ProfileKind,
  bin: string,
  action: "login" | "logout",
  configDir?: string,
): string {
  const quotedBin = /^[A-Za-z0-9_./-]+$/.test(bin) ? bin : `'${bin.replace(/'/g, `'"'"'`)}'`;
  const command = kind === "claude" ? `${quotedBin} auth ${action}` : `${quotedBin} ${action}`;
  if (!configDir) return command;
  const rendered = shellConfigDir(configDir);
  const quoted = rendered.startsWith("$HOME")
    ? `"$HOME${rendered.slice(5).replace(/(["\\`$])/g, "\\$1")}"`
    : `'${rendered.replace(/'/g, `'"'"'`)}'`;
  return `${configEnvOf(kind)}=${quoted} ${command}`;
}

/**
 * 这条命令里的配置目录是不是写成了不会展开的 `"~/..."`。
 *
 * 本工具早先生成的命令都是这个形状，已经写进用户终端配置里了，只能回读时认出来
 * 并给一个订正入口，不能指望用户自己发现。
 */
export function launchCommandNeedsTildeFix(value: string): boolean {
  return /\b(CLAUDE_CONFIG_DIR|CODEX_HOME)=(["'])~(\/|\2)/.test(value);
}

/** 把一条命令里不会展开的 `"~/..."` 订正成 `"$HOME/..."`，其余原样。 */
export function fixLaunchCommandTilde(value: string): string {
  return value.replace(
    /\b(CLAUDE_CONFIG_DIR|CODEX_HOME)=(["'])~(\/[^"']*)?\2/g,
    (_m, name: string, quote: string, rest: string | undefined) =>
      `${name}=${quote}$HOME${rest ?? ""}${quote}`,
  );
}

/**
 * 第三方 Codex 环境的这条命令有没有把 Codex 当作官方凭据读取的变量清空。
 * 早先保存的命令没有这段前缀，shell 里有这两个变量时 Codex 会优先用它们。
 */
export function launchCommandNeedsCredentialClear(value: string): boolean {
  return !CODEX_CLEARED_ENV.every((name) => new RegExp(`(^|\\s)${name}=(\\s|$)`).test(value));
}

/** 在命令前面补上清空官方凭据变量的赋值；已有的同名赋值先去掉，其余原样。 */
export function fixLaunchCommandCredentialClear(value: string): string {
  let rest = value;
  for (const name of CODEX_CLEARED_ENV) {
    rest = rest.replace(new RegExp(`(^|\\s)${name}=\\S*`, "g"), "$1");
  }
  // 只去掉开头留下的空白：中间的空白可能在引号里，原样保留
  return [...CODEX_CLEARED_ENV.map((name) => `${name}=`), rest.replace(/^\s+/, "")].join(" ");
}

export interface ProfileLaunchCommand {
  name: string;
  value: string;
  /** 配置目录写成了不会展开的 `"~/..."`，需要订正。 */
  needsTildeFix: boolean;
  /** Codex 环境的命令没有清空会压过目录登录的凭据变量，需要订正。 */
  needsCredentialClear: boolean;
}

/**
 * 找出属于某个实例的全部启动命令。
 *
 * 归属不额外存字段，靠命令里的配置目录变量反查——别名始终只有别名域一个写者，
 * 实例只是帮你生成第一条。一个实例可以挂任意多条（日常一条、全自动一条…）。
 */
export function launchCommandsForProfile(
  aliases: Record<string, string>,
  kind: ProfileKind,
  configDir: string,
  parseEnv: (value: string) => Record<string, string>,
): ProfileLaunchCommand[] {
  const envName = configEnvOf(kind);
  const want = normalizeConfigDir(configDir);
  return Object.entries(aliases)
    .filter(([, value]) => {
      const dir = parseEnv(value)[envName];
      return dir !== undefined && normalizeConfigDir(dir) === want;
    })
    .map(([name, value]) => ({
      name,
      value,
      needsTildeFix: launchCommandNeedsTildeFix(value),
      needsCredentialClear: kind === "codex" && launchCommandNeedsCredentialClear(value),
    }))
    .sort((a, b) => a.name.localeCompare(b.name));
}

/** 归一化配置目录，用于判断一条命令属于哪个实例（`~/x`、`$HOME/x` 视为同一处）。 */
export function normalizeConfigDir(dir: string): string {
  const trimmed = dir.trim().replace(/\/$/, "");
  if (trimmed === "~" || trimmed === "$HOME") return "~";
  if (trimmed.startsWith("$HOME/")) return `~/${trimmed.slice(6)}`;
  return trimmed;
}

/** 从既有定义回填表单（编辑实例时用）。 */
export function parseProfileDefinition(id: string, definition: Record<string, unknown>): ProfileFormInput {
  const profile = (definition.profile ?? {}) as Record<string, unknown>;
  const baseAgent = typeof profile.base_agent === "string" ? profile.base_agent : "claude-code";
  const kind: ProfileKind = profile.kind === "codex" || baseAgent === "codex" ? "codex" : "claude";
  const env = (profile.env ?? {}) as Record<string, string>;
  const authHeader: ProfileFormInput["authHeader"] =
    kind === "claude" && env.ANTHROPIC_API_KEY !== undefined && env.ANTHROPIC_AUTH_TOKEN === undefined ? "api_key" : "token";
  const tokenRef = kind === "claude" ? env[claudeCredentialEnv(authHeader)] : (profile.env_key as string | undefined);
  const tokenVar = tokenRef ? tokenRef.replace(/^\$\{/, "").replace(/\}$/, "") : "";
  const extraEnv: Record<string, string> = {};
  if (kind === "claude") {
    for (const [name, value] of Object.entries(env)) {
      if (!CLAUDE_FORM_ENV.includes(name)) extraEnv[name] = value;
    }
  }
  return {
    id,
    displayName: typeof definition.display_name === "string" ? definition.display_name : id,
    baseAgent,
    sourceConnection: typeof profile.source_connection === "string" ? profile.source_connection : undefined,
    kind,
    configDir: typeof profile.config_dir === "string" ? profile.config_dir : defaultConfigDir(kind, id),
    baseUrl: kind === "claude" ? (env.ANTHROPIC_BASE_URL ?? "") : ((profile.base_url as string | undefined) ?? ""),
    tokenVar,
    authHeader,
    model: typeof profile.model === "string" ? profile.model : "",
    extraArgs: "",
    extraEnv,
  };
}

/**
 * 用着这套接入的独立环境：新环境按保存时的 Provider 名称关联；旧定义没有来源标识时，
 * 仍按同一助手、地址和凭据变量推断。来源标识在 Provider 地址变更后仍可定位原环境。
 */
export function instancesUsingConnection(
  definitions: Record<string, unknown>,
  agentId: string,
  connection: { name: string; baseUrl: string; tokenVar: string },
): { id: string; label: string }[] {
  const normalize = (url: string) => url.trim().replace(/\/+$/, "");
  const baseUrl = normalize(connection.baseUrl);
  const out: { id: string; label: string }[] = [];
  for (const [id, raw] of Object.entries(definitions)) {
    const definition = raw as Record<string, unknown> | null;
    if (!definition || typeof definition !== "object" || !definition.profile || definition._delete === true) continue;
    const input = parseProfileDefinition(id, definition);
    if (input.baseAgent !== agentId) continue;
    if (input.sourceConnection !== undefined) {
      if (input.sourceConnection !== connection.name) continue;
    } else if (!baseUrl || normalize(input.baseUrl) !== baseUrl || input.tokenVar !== connection.tokenVar.trim()) {
      continue;
    }
    out.push({ id, label: input.displayName });
  }
  return out.sort((a, b) => a.id.localeCompare(b.id));
}

/** 接入名 → 可用的环境名：小写字母、数字与连字符，且避开已有名字。 */
function profileIdFor(name: string, existingIds: string[]): string {
  const stem =
    name
      .toLowerCase()
      .replace(/[^a-z0-9-]+/g, "-")
      .replace(/-+/g, "-")
      .replace(/^-+|-+$/g, "") || "relay";
  const taken = new Set(existingIds);
  let id = stem;
  let n = 2;
  while (taken.has(id)) {
    id = `${stem}-${n}`;
    n += 1;
  }
  return id;
}

/**
 * 把一套接入配置建成独立环境的表单初值。第三方凭据只放进这个新环境自己的配置目录，
 * 主配置保留给官方账号；凭据仍只引用同一个变量名，不复制取值。
 *
 * 返回表单初值与没能带过去的字段（Codex 独立环境不写超时）。引用 config.toml 里已有
 * 接入方式的 Codex 接入没法搬进独立目录，返回 null。
 */
export function connectionToProfileInput(
  agent: { id: string; kind: ProfileKind },
  connection: ConnectionEntry,
  existingIds: string[],
): { input: ProfileFormInput; dropped: string[] } | null {
  if (connection.providerRef) return null;
  const id = profileIdFor(connection.name, existingIds);
  const extraEnv: Record<string, string> = {};
  const dropped: string[] = [];
  if (agent.kind === "claude") {
    if (connection.aliasOpus.trim()) extraEnv.ANTHROPIC_DEFAULT_OPUS_MODEL = connection.aliasOpus.trim();
    if (connection.aliasSonnet.trim()) extraEnv.ANTHROPIC_DEFAULT_SONNET_MODEL = connection.aliasSonnet.trim();
    // 官方已弃用 ANTHROPIC_SMALL_FAST_MODEL：后台任务改用 Haiku 别名那个模型。旧接入里只填了
    // 小任务模型的，按官方替代写成 Haiku 别名；两者都填了以 Haiku 别名为准。
    if (connection.aliasFable?.trim()) extraEnv.ANTHROPIC_DEFAULT_FABLE_MODEL = connection.aliasFable.trim();
    if (connection.subagentModel?.trim()) extraEnv.CLAUDE_CODE_SUBAGENT_MODEL = connection.subagentModel.trim();
    const haiku = connection.aliasHaiku.trim() || connection.smallFastModel.trim();
    if (haiku) extraEnv.ANTHROPIC_DEFAULT_HAIKU_MODEL = haiku;
    if (connection.aliasHaiku.trim() && connection.smallFastModel.trim()
      && connection.aliasHaiku.trim() !== connection.smallFastModel.trim()) {
      dropped.push("小任务模型（官方已弃用，后台任务改用 Haiku 别名的模型）");
    }
    if (connection.timeoutMs.trim()) extraEnv.API_TIMEOUT_MS = connection.timeoutMs.trim();
  } else if (connection.timeoutMs.trim()) {
    dropped.push("请求超时");
  }
  return {
    input: {
      id,
      displayName: connection.label.trim() || connection.name,
      baseAgent: agent.id,
      sourceConnection: connection.name,
      kind: agent.kind,
      configDir: defaultConfigDir(agent.kind, id),
      baseUrl: connection.baseUrl.trim(),
      tokenVar: connection.tokenVar.trim(),
      authHeader: connection.auth,
      model: connection.model.trim(),
      extraArgs: "",
      extraEnv,
    },
    dropped,
  };
}
