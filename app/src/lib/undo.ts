import { toast } from "sonner";

/**
 * 带「撤销」的成功提示。
 *
 * 只用于真正可逆的那类操作——把刚删掉的定义原样写回去就能复原。已经写进配置文件的
 * 落地动作不在此列：那类要靠「外部修改检测 + 重新应用」，不能假装一键回到从前，
 * 所以调用方必须在 description 里说清楚撤销恢复的是什么。
 */
export function toastWithUndo(
  message: string,
  options: { description?: string; onUndo: () => void | Promise<void> },
): void {
  toast.success(message, {
    description: options.description,
    duration: 10_000,
    action: {
      label: "撤销",
      onClick: () => void options.onUndo(),
    },
  });
}
