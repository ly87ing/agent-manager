import { emptyConnection, type ConnectionEntry } from "./agentDefinitionForm";

/**
 * 接入配置预设：把各家官方文档里的地址与模型名填进表单，省得去翻文档。
 *
 * 只收**厂商自营、且有官方文档写明**的端点。各类聚合中转站不收：它们的地址随运营变动，
 * 这份表没有维护通道，过期的地址比没有更糟——那种情况用「自定义」自己填。
 *
 * 两家形态的端点不通用：Claude Code 走 Anthropic 协议，Codex 走 OpenAI Responses 协议，
 * 同一厂商的两个地址不一样，混用会直接 404，所以每条都标了适用形态。
 *
 * 模型名比地址更容易过期（各家发新模型就改）。选完预设后字段都可以改；
 * 报「模型不存在」时以各家官方文档为准。
 *
 * 核对日期：2026-09-20。
 */

export interface ConnectionPreset {
  id: string;
  label: string;
  /** 这条只对这种形态的助手有意义。 */
  kind: "claude" | "codex";
  baseUrl: string;
  model?: string;
  smallFastModel?: string;
  aliasOpus?: string;
  aliasSonnet?: string;
  aliasHaiku?: string;
  /** 建议的环境变量名；取值仍要自己在密钥页填。 */
  tokenVar: string;
  /** 这条取自哪家的什么文档，便于自己去核对。 */
  source: string;
}

export const CONNECTION_PRESETS: ConnectionPreset[] = [
  // ---- Claude Code：Anthropic 协议端点 ----
  {
    id: "glm-cn-claude",
    label: "智谱 GLM（国内）",
    kind: "claude",
    baseUrl: "https://open.bigmodel.cn/api/anthropic",
    model: "glm-5.2[1m]",
    aliasOpus: "glm-5.2[1m]",
    aliasSonnet: "glm-5.2[1m]",
    aliasHaiku: "glm-4.7",
    tokenVar: "GLM_API_KEY",
    source: "智谱开放平台文档 docs.bigmodel.cn",
  },
  {
    id: "glm-global-claude",
    label: "智谱 GLM（国际 Z.ai）",
    kind: "claude",
    baseUrl: "https://api.z.ai/api/anthropic",
    model: "glm-5.2[1m]",
    aliasHaiku: "glm-4.7",
    tokenVar: "GLM_API_KEY",
    source: "Z.ai 文档 docs.z.ai",
  },
  {
    id: "kimi-cn-claude",
    label: "Kimi（国内）",
    kind: "claude",
    baseUrl: "https://api.moonshot.cn/anthropic",
    model: "kimi-k3[1m]",
    aliasOpus: "kimi-k3[1m]",
    aliasSonnet: "kimi-k3[1m]",
    aliasHaiku: "kimi-k2.7-code",
    tokenVar: "KIMI_API_KEY",
    source: "Kimi 开放平台文档 platform.kimi.com",
  },
  {
    id: "kimi-global-claude",
    label: "Kimi（国际）",
    kind: "claude",
    baseUrl: "https://api.moonshot.ai/anthropic",
    model: "kimi-k3[1m]",
    aliasOpus: "kimi-k3[1m]",
    aliasSonnet: "kimi-k3[1m]",
    aliasHaiku: "kimi-k2.7-code",
    tokenVar: "KIMI_API_KEY",
    source: "Kimi 开放平台文档 platform.kimi.ai",
  },
  {
    id: "deepseek-claude",
    label: "DeepSeek",
    kind: "claude",
    baseUrl: "https://api.deepseek.com/anthropic",
    model: "deepseek-flash[1m]",
    aliasOpus: "deepseek-flash[1m]",
    aliasSonnet: "deepseek-flash[1m]",
    aliasHaiku: "deepseek-flash",
    tokenVar: "DEEPSEEK_API_KEY",
    source: "DeepSeek API 文档 api-docs.deepseek.com",
  },

  // ---- Codex：OpenAI Responses 协议端点（与上面的地址不是同一个）----
  {
    id: "glm-cn-codex",
    label: "智谱 GLM（国内）",
    kind: "codex",
    baseUrl: "https://open.bigmodel.cn/api/v1",
    model: "glm-5.3",
    tokenVar: "GLM_API_KEY",
    source: "智谱开放平台文档 docs.bigmodel.cn",
  },
  {
    id: "glm-global-codex",
    label: "智谱 GLM（国际 Z.ai）",
    kind: "codex",
    baseUrl: "https://api.z.ai/api/v1",
    model: "glm-5.3",
    tokenVar: "GLM_API_KEY",
    source: "Z.ai 文档 docs.z.ai",
  },
  {
    id: "kimi-cn-codex",
    label: "Kimi（国内）",
    kind: "codex",
    baseUrl: "https://api.kimi.com/coding/v1",
    model: "k3-256k",
    tokenVar: "KIMI_API_KEY",
    source: "Kimi Code 文档 platform.kimi.com",
  },
  {
    id: "kimi-global-codex",
    label: "Kimi（国际）",
    kind: "codex",
    baseUrl: "https://api.kimi.ai/coding/v1",
    model: "k3-256k",
    tokenVar: "KIMI_API_KEY",
    source: "Kimi Code 文档 platform.kimi.ai",
  },
  {
    id: "deepseek-codex",
    label: "DeepSeek",
    kind: "codex",
    baseUrl: "https://api.deepseek.com/",
    model: "deepseek-flash",
    tokenVar: "DEEPSEEK_API_KEY",
    source: "DeepSeek API 文档 api-docs.deepseek.com",
  },
];

/** 某种形态能用的预设。 */
export function presetsFor(kind: "claude" | "codex" | null): ConnectionPreset[] {
  if (!kind) return [];
  return CONNECTION_PRESETS.filter((preset) => preset.kind === kind);
}

/**
 * 预设 → 一套接入配置。
 *
 * `name` 是写进定义的键名，要在同一个助手里唯一；调用方负责避让已有的名字。
 */
export function connectionFromPreset(preset: ConnectionPreset, name: string): ConnectionEntry {
  return {
    ...emptyConnection(name),
    label: preset.label,
    baseUrl: preset.baseUrl,
    // 这几家的文档都用 Bearer；Codex 侧没有这个选项，写了也不生效
    auth: "token",
    tokenVar: preset.tokenVar,
    model: preset.model ?? "",
    smallFastModel: preset.smallFastModel ?? "",
    aliasOpus: preset.aliasOpus ?? "",
    aliasSonnet: preset.aliasSonnet ?? "",
    aliasHaiku: preset.aliasHaiku ?? "",
  };
}

/** 预设 id → 不与现有配置重名的键名。 */
export function presetConnectionName(preset: ConnectionPreset, taken: string[]): string {
  // 预设 id 里带着形态后缀，键名去掉它更短也更好认
  const base = preset.id.replace(/-(claude|codex)$/, "");
  if (!taken.includes(base)) return base;
  let n = 2;
  while (taken.includes(`${base}${n}`)) n += 1;
  return `${base}${n}`;
}
