/** 界面按平台显示的差异：macOS 的快捷键写 ⌘，其他系统写 Ctrl+。 */
export const isMac = typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent);

/** 快捷键的修饰键前缀（⌘K / Ctrl+K）。 */
export const MOD = isMac ? "⌘" : "Ctrl+";

