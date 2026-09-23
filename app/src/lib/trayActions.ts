/**
 * 托盘动作的界面侧处理。
 *
 * 托盘的接入快切仍交给界面侧现有执行通道处理，因此锁、确认、审计与双回读不另起一套。
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
}

export type TrayAction = "search" | "takeover" | ({ type: "switch-connection" } & Omit<TrayConnection, "current">);

const TRAY_ACTION_EVENT = "tray-action";

export function isTrayAction(value: unknown): value is TrayAction {
  if (value === "search" || value === "takeover") return true;
  if (typeof value !== "object" || value === null) return false;
  const action = value as Record<string, unknown>;
  return action.type === "switch-connection"
    && typeof action.agentId === "string"
    && typeof action.agentName === "string"
    && typeof action.profile === "string"
    && typeof action.profileLabel === "string";
}

/** 用脱敏的 id/显示名刷新托盘菜单；浏览器预览中静默跳过。 */
export async function publishTrayConnections(connections: TrayConnection[]): Promise<void> {
  if (!isTauri()) return;
  try {
    await invoke("tray_set_connections", { connections });
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
