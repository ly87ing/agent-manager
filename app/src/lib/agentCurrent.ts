import type { EngineElement } from "./contract";
import type { AgentCurrentConfigStatus, AgentsObserved } from "./observed";

/**
 * 主账号的实际配置以文件为准：引擎在 status 里给出脱敏后的当前值与写入基准。
 * 主账号只放官方登录，这里只剩运行参数写入（如实时额度状态栏）。
 */
export function currentConfigOf(element: EngineElement): AgentCurrentConfigStatus | null {
  return (element.observed as AgentsObserved).current_config ?? null;
}

export interface ConfigureRequest {
  agentId: string;
  base: string;
  /** 完整文件预览时的原文字节基准。 */
  previewFileBase?: string;
  /** 运行参数补丁：只含改动的字段。 */
  runtime: Record<string, unknown>;
  allowYolo?: boolean;
}

export function configureArgs(request: ConfigureRequest): string[] {
  const args = ["--only", `agents:${request.agentId}`, "--base", request.base];
  if (request.previewFileBase) args.push("--preview-file-base", request.previewFileBase);
  args.push("--runtime", JSON.stringify(request.runtime));
  if (request.allowYolo) args.push("--allow-yolo");
  return args;
}
