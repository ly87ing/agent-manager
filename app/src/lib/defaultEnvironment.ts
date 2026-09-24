/**
 * 默认环境（主配置）现在处于什么状态、切回官方账号会动什么——用用户的话说清楚。
 *
 * 主配置只放官方账号：账号看「连到哪、用谁的凭据」（Claude 的第三方地址与令牌，Codex 的接入方式），
 * 模型、别名与超时只是官方账号上的设置，不算另一套接入（引擎 `current.official`）。
 */
import type { AgentCurrentConfig, AgentCurrentConnection } from "./observed";

export type AgentKind = "claude" | "codex";

export interface DefaultEnvState {
  /** unknown：实际配置读不了。 */
  account: "official" | "third-party" | "unknown";
  accountText: string;
  modelText: string;
  /** 模型取自哪里；用助手默认模型时为 null。 */
  modelSource: string | null;
  /** 官方登录保存在哪、谁管。 */
  loginNote: string;
}

const PRODUCT: Record<AgentKind, string> = { claude: "Claude Code", codex: "Codex" };

/** 官方登录的存放位置（官方文档与 Codex 源码），本工具从不读写。 */
export function officialLoginStore(kind: AgentKind): string {
  return kind === "codex"
    ? "~/.codex/auth.json 或系统钥匙串"
    : "系统钥匙串或 ~/.claude/.credentials.json，账号信息在 ~/.claude.json";
}

/** 老版本引擎没有 official 字段时按 default 推断。 */
export function isOfficial(current: AgentCurrentConnection | null): boolean {
  return current ? current.official ?? current.default : false;
}

export function defaultEnvState(
  kind: AgentKind,
  current: AgentCurrentConnection | null,
  config: AgentCurrentConfig | null | undefined,
): DefaultEnvState {
  const product = PRODUCT[kind];
  const loginNote = `登录由 ${product} 自己保存在 ${officialLoginStore(kind)}，本工具不读取也不改动。`;
  const account: DefaultEnvState["account"] = !current ? "unknown" : isOfficial(current) ? "official" : "third-party";
  const endpoint =
    config?.base_url || (config?.provider && config.provider !== "openai" && config.provider !== "anthropic" ? `接入方式 ${config.provider}` : "");
  const accountText =
    account === "official"
      ? "官方账号"
      : account === "third-party"
        ? `第三方接入${endpoint ? ` · ${endpoint}` : ""}`
        : "读不到实际配置";
  let modelText = `${product} 默认模型`;
  let modelSource: string | null = null;
  if (kind === "codex" && config?.model) {
    modelText = config.model;
    modelSource = "config.toml 里的 model";
  } else if (kind === "claude" && config?.model) {
    modelText = config.model;
    modelSource = config.settings_model
      ? `settings.json env 里的 ANTHROPIC_MODEL，压过 /model 保存的 ${config.settings_model}`
      : "settings.json env 里的 ANTHROPIC_MODEL，优先于 /model 的选择";
  } else if (kind === "claude" && config?.settings_model) {
    modelText = config.settings_model;
    modelSource = "settings.json 里的 model（/model 保存的）";
  }
  return { account, accountText, modelText, modelSource, loginNote };
}

/** 「切回官方账号」前逐条说明：会撤掉什么、切之前做什么、不会动什么、之后怎样。 */
export function switchBackEffects(kind: AgentKind): { removes: string; before: string; keeps: string[]; after: string } {
  const product = PRODUCT[kind];
  return {
    removes:
      kind === "codex"
        ? "config.toml 里的 model_provider 与 model，以及本工具写的那张接入表"
        : "settings.json 的 env 里的接口地址、令牌（ANTHROPIC_AUTH_TOKEN / ANTHROPIC_API_KEY）、模型、别名与超时",
    before: "这份第三方配置先保存成一套接入：令牌写进你在「设置 → 密钥」选的文件，库里只存变量名；之后可以把它建成独立环境继续用。",
    keeps: [
      `官方账号的登录（${officialLoginStore(kind)}）`,
      "运行设置（自动化档位、推理强度等）",
      "MCP、Hooks 与配置文件里你自己的其他设置",
      "其他已保存的接入与密钥变量",
    ],
    after:
      kind === "codex"
        ? `已经打开的 ${product} 要重启才生效；没登录过官方账号的，在终端运行 codex login 登录。`
        : `已经打开的 ${product} 要重启才生效；没登录过官方账号的，启动 claude 后按提示登录。`,
  };
}
