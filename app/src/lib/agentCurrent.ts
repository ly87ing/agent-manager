import type { EngineElement, Envelope } from "./contract";
import type { AgentCurrentConfigStatus, AgentCurrentConnection, AgentsObserved } from "./observed";

/**
 * 助手的「当前」以实际配置文件为准：引擎在 status 里按内容识别出当前接入并给出写入基准。
 * 库里的 active 只是上次写入记录，界面不再拿它断言当前是谁。
 */

export function currentConfigOf(element: EngineElement): AgentCurrentConfigStatus | null {
  return (element.observed as AgentsObserved).current_config ?? null;
}

/** 按内容识别出的当前接入；实际配置读不了时为 null。 */
export function currentConnectionOf(element: EngineElement): AgentCurrentConnection | null {
  return currentConfigOf(element)?.current ?? null;
}

/** 写入基准。没有它（文件不可读、不是普通文件等）就不能写。 */
export function configureBaseOf(element: EngineElement): string | null {
  const base = currentConfigOf(element)?.base;
  return typeof base === "string" && base ? base : null;
}

/** 实际配置对不上任何已保存接入，也不是助手默认账号。 */
export function hasUnsavedLive(current: AgentCurrentConnection | null): boolean {
  return Boolean(current && current.profile === null && !current.default);
}

export interface ConfigureRequest {
  agentId: string;
  base: string;
  /** 切换目标；空字符串 = 助手默认账号。 */
  use?: string;
  /** 与 use 同时给：先用这份定义替换目标接入再切换（编辑当前接入）。 */
  connection?: Record<string, unknown>;
  /** 运行参数补丁：只含改动的字段。 */
  runtime?: Record<string, unknown>;
  /** 只把实际配置存成接入配置，不写助手文件。 */
  saveCurrent?: boolean;
  allowYolo?: boolean;
  /** 放弃无法完整保存的实际接入，直接切换。 */
  discardLive?: boolean;
  /** 放弃还没保存的实际令牌。 */
  discardLiveToken?: boolean;
}

export function configureArgs(request: ConfigureRequest): string[] {
  const args = ["--only", `agents:${request.agentId}`, "--base", request.base];
  if (request.runtime) args.push("--runtime", JSON.stringify(request.runtime));
  if (request.use !== undefined) args.push(...(request.use ? ["--use", request.use] : ["--use-default"]));
  if (request.connection) args.push("--connection", JSON.stringify(request.connection));
  if (request.saveCurrent) args.push("--save-current");
  if (request.allowYolo) args.push("--allow-yolo");
  if (request.discardLive) args.push("--discard-live");
  if (request.discardLiveToken) args.push("--discard-live-token");
  return args;
}

/** 引擎拒绝写入的原因里，需要界面给出专门出路的几种。 */
export type ConfigureRefusal = "changed" | "live-token" | "unrepresentable";

export function configureRefusalOf(envelope: Envelope): ConfigureRefusal | null {
  switch (envelope.errors[0]?.code) {
    case "AGENT_CONFIG_CHANGED":
      return "changed";
    case "AGENT_LIVE_TOKEN_UNSAVED":
      return "live-token";
    case "AGENT_LIVE_UNREPRESENTABLE":
      return "unrepresentable";
    default:
      return null;
  }
}

/**
 * 回填被拒时弹窗的正文：用界面自己的话，不把引擎给命令行用户的参数提示（--discard-live 等）
 * 展示出来。存不下的具体原因仍然列出，用户据此决定先去改配置文件还是放弃。
 */
export function liveRefusalDescription(kind: "live-token" | "unrepresentable", message: string): string {
  if (kind === "live-token") {
    return "当前实际配置里的令牌还没有保存为变量，切换会丢失它。先到“设置 → 密钥”选择外部密钥文件，令牌会写进该文件、库里只存变量名；或者放弃这个令牌继续切换。";
  }
  const reasons = /丢失这些内容：([\s\S]+?)。确认放弃/.exec(message)?.[1]?.trim();
  const detail = reasons && !reasons.includes("--") ? `：${reasons}` : "";
  return `当前实际配置里有本工具无法原样保存的内容，切换后会丢失${detail}。可以先在配置文件里处理后再切换，或者放弃当前实际配置继续切换。`;
}

export interface ConfigureBackfill {
  name: string;
  created: boolean;
  label?: string | null;
  token: "none" | "kept" | "reused" | "adopted" | "discarded" | string;
  token_var?: string | null;
}

/** 切换前实际配置被存到了哪里，给成功提示用；没有回填时为 null。 */
export function backfillNotice(data: unknown): string | null {
  const backfill = (data as { backfill?: ConfigureBackfill | null } | null | undefined)?.backfill;
  if (!backfill) return null;
  const target = backfill.label || backfill.name;
  const where = backfill.created ? `切换前的实际配置已保存为「${target}」` : `外部改动已保存进「${target}」`;
  if (backfill.token === "adopted" && backfill.token_var) {
    return `${where}，令牌写进了密钥文件的变量 ${backfill.token_var}`;
  }
  if (backfill.token === "discarded") return `${where}，按你的选择没有保存原来的令牌`;
  return where;
}
