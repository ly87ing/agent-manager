/**
 * 托盘动作的界面侧处理。
 *
 * 托盘只负责把人带到能做这件事的地方，不直接触发写操作——真正的执行仍在界面上点，
 * 因此锁、确认与审计都走同一条既有路径。非 Tauri 环境（浏览器预览、单测）拿不到
 * 事件通道，直接静默跳过。
 */
export type TrayAction = "search" | "takeover";

const TRAY_ACTION_EVENT = "tray-action";

export function isTrayAction(value: unknown): value is TrayAction {
  return value === "search" || value === "takeover";
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
