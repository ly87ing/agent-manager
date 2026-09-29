/**
 * 托盘动作的界面侧处理。
 *
 * 托盘按助手列出全部账号（名称 · 启动命令 · 额度）。点中某个账号只打开命令面板并定位到它，
 * 按 Enter 复制启动命令；托盘不写任何配置。
 * 非 Tauri 环境（浏览器预览、单测）拿不到事件通道，直接静默跳过。
 */
import { invoke, isTauri } from "@tauri-apps/api/core";

export interface TrayAccount {
  agentId: string;
  agentName: string;
  /** 账号页的元素 id：主账号 `agents:<助手>`，其余 `profiles:<实例>`。 */
  accountId: string;
  accountName: string;
  /** 启动命令；还没有时为 null。 */
  command: string | null;
  /** 来自这个账号配置目录的本机额度记录；没有有效窗口时为 null。 */
  usageSummary: string | null;
}

export type TrayAction = "search"
  | ({ type: "open-account" } & Pick<TrayAccount, "agentName" | "accountId" | "accountName">);

const TRAY_ACTION_EVENT = "tray-action";

export function isTrayAction(value: unknown): value is TrayAction {
  if (value === "search") return true;
  if (typeof value !== "object" || value === null) return false;
  const action = value as Record<string, unknown>;
  return action.type === "open-account"
    && typeof action.agentName === "string"
    && typeof action.accountId === "string"
    && typeof action.accountName === "string";
}

/** 用脱敏的 id / 显示名刷新托盘菜单；浏览器预览中静默跳过。 */
export async function publishTrayAccounts(accounts: TrayAccount[]): Promise<void> {
  if (!isTauri()) return;
  try {
    await invoke("tray_set_accounts", { accounts });
  } catch (error) {
    // 原生菜单更新失败应留下诊断；它不影响主界面继续工作。
    console.error("更新托盘账号菜单失败", error);
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
