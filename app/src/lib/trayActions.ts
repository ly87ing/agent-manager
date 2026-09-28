/**
 * 托盘动作的界面侧处理。
 *
 * 托盘列出官方账号与独立环境。前者交给界面侧现有切换通道，后者只打开精确入口；
 * 因此锁、确认、审计与双回读不另起一套。
 * 非 Tauri 环境（浏览器预览、单测）拿不到事件通道，直接静默跳过。
 */
import { invoke, isTauri } from "@tauri-apps/api/core";

export interface TrayConnection {
  agentId: string;
  agentName: string;
  profile: string;
  profileLabel: string;
  /** 与客户端实际配置一致的那一项（引擎按内容识别）。 */
  current: boolean;
  /** 来自该助手实际配置目录的本机额度缓存；没有有效窗口时为 null。 */
  usageSummary: string | null;
}

export interface TrayEnvironment {
  agentId: string;
  agentName: string;
  environmentId: string;
  environmentName: string;
  /** 来自实际别名文件；没有可用命令时只打开环境卡片。 */
  launchName: string | null;
  /** 来自这个独立环境配置目录的本机额度缓存。 */
  usageSummary: string | null;
}

export type TrayAction = "search" | "takeover"
  | ({ type: "switch-connection" } & Omit<TrayConnection, "current" | "usageSummary">)
  | ({ type: "open-environment" } & Omit<TrayEnvironment, "usageSummary">);

const TRAY_ACTION_EVENT = "tray-action";

export function isTrayAction(value: unknown): value is TrayAction {
  if (value === "search" || value === "takeover") return true;
  if (typeof value !== "object" || value === null) return false;
  const action = value as Record<string, unknown>;
  if (action.type === "switch-connection") {
    return typeof action.agentId === "string"
      && typeof action.agentName === "string"
      && typeof action.profile === "string"
      && typeof action.profileLabel === "string";
  }
  return action.type === "open-environment"
    && typeof action.agentId === "string"
    && typeof action.agentName === "string"
    && typeof action.environmentId === "string"
    && typeof action.environmentName === "string"
    && (action.launchName === null || typeof action.launchName === "string");
}

/** 用脱敏的 id/显示名刷新托盘菜单；浏览器预览中静默跳过。 */
export async function publishTrayConnections(connections: TrayConnection[], environments: TrayEnvironment[]): Promise<void> {
  if (!isTauri()) return;
  try {
    await invoke("tray_set_connections", { connections, environments });
  } catch (error) {
    // 原生菜单更新失败应留下诊断；它不影响主界面继续工作。
    console.error("更新托盘接入菜单失败", error);
  }
}

/** 订阅托盘动作；返回取消订阅函数（拿不到事件通道时返回空函数）。 */
export function subscribeTrayActions(onAction: (action: TrayAction) => void): () => void {
  let disposed = false;
  let unlisten: (() => void) | undefined;
  void (async () => {
    try {
      const { listen } = await import("@tauri-apps/api/event");
      const stop = await listen<unknown>(TRAY_ACTION_EVENT, (event) => {
        if (isTrayAction(event.payload)) onAction(event.payload);
      });
      if (disposed) stop();
      else unlisten = stop;
    } catch {
      // 浏览器环境没有托盘，不是错误
    }
  })();
  return () => {
    disposed = true;
    unlisten?.();
  };
}
