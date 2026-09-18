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

/** 该实例的启动命令取值（与别名域取值格式一致）。 */
export function profileLaunchValue(input: ProfileFormInput, baseBin: string): string {
  const configEnv = input.kind === "codex" ? "CODEX_HOME" : "CLAUDE_CONFIG_DIR";
  const parts = [`${configEnv}="${input.configDir.replace(/\/$/, "")}"`, baseBin];
  if (input.kind === "codex") parts.push("-p", CODEX_PROFILE_NAME);
  const extra = input.extraArgs.trim();
  if (extra) parts.push(extra);
  return parts.join(" ");
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
