/**
 * 助手实例（profile）的定义生成与回填（纯函数）。
 *
 * 一个实例 = 一个独立配置目录 + 一条启动命令。实例定义就是一条 agent 定义（多一个 profile 段），
 * 所以技能与 MCP 的逐助手分配天然适用于它。
 *
 * 目标文件分工（避免两个域写同一个文件互相抹掉）：
 * - Claude：本工具写 `<配置目录>/settings.json`，MCP 写 `<配置目录>/.claude.json`
 * - Codex：本工具写 `<配置目录>/agent-manager.config.toml`（启动时 `-p` 叠加），
 *   MCP 写 `<配置目录>/config.toml`
 */

export type ProfileKind = "claude" | "codex";

/** Codex 叠加配置的固定名字，与引擎侧保持一致。 */
export const CODEX_PROFILE_NAME = "agent-manager";

export interface ProfileFormInput {
  id: string;
  displayName: string;
  baseAgent: string;
  kind: ProfileKind;
  configDir: string;
  /** 接口地址，留空表示用该助手的默认地址。 */
  baseUrl: string;
  /** 鉴权取值所在的环境变量名；库里只存这个名字，永远不存取值。 */
  tokenVar: string;
  model: string;
  /** 启动命令每次都带上的参数。 */
  extraArgs: string;
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
  if (input.kind === "claude") {
    const env: Record<string, string> = {};
    if (input.baseUrl.trim()) env.ANTHROPIC_BASE_URL = input.baseUrl.trim();
    if (input.tokenVar.trim()) env.ANTHROPIC_AUTH_TOKEN = `\${${input.tokenVar.trim()}}`;
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

/** 该实例的启动命令取值（与别名域取值格式一致）。 */
export function profileLaunchValue(input: ProfileFormInput, baseBin: string): string {
  const parts = [
    `${configEnvOf(input.kind)}="${shellConfigDir(input.configDir)}"`,
    baseBin,
  ];
  if (input.kind === "codex") parts.push("-p", CODEX_PROFILE_NAME);
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

export interface ProfileLaunchCommand {
  name: string;
  value: string;
  /** 配置目录写成了不会展开的 `"~/..."`，需要订正。 */
  needsTildeFix: boolean;
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
    .map(([name, value]) => ({ name, value, needsTildeFix: launchCommandNeedsTildeFix(value) }))
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
  const tokenRef = kind === "claude" ? env.ANTHROPIC_AUTH_TOKEN : (profile.env_key as string | undefined);
  const tokenVar = tokenRef ? tokenRef.replace(/^\$\{/, "").replace(/\}$/, "") : "";
  return {
    id,
    displayName: typeof definition.display_name === "string" ? definition.display_name : id,
    baseAgent,
    kind,
    configDir: typeof profile.config_dir === "string" ? profile.config_dir : defaultConfigDir(kind, id),
    baseUrl: kind === "claude" ? (env.ANTHROPIC_BASE_URL ?? "") : ((profile.base_url as string | undefined) ?? ""),
    tokenVar,
    model: typeof profile.model === "string" ? profile.model : "",
    extraArgs: "",
  };
}
