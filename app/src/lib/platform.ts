/** 界面按平台显示的差异：macOS 的快捷键写 ⌘，其他系统写 Ctrl+。 */
export const isMac = typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent);

/** 快捷键的修饰键前缀（⌘K / Ctrl+K）。 */
export const MOD = isMac ? "⌘" : "Ctrl+";

/** Windows：启动命令写成 ~/.agent-manager/bin 里的 .cmd 脚本，而不是 ~/.zshrc 里的 alias。 */
export const isWindows = typeof navigator !== "undefined" && /Win/.test(navigator.platform || navigator.userAgent);

/** 启动命令写在哪里（给「保存即写入…」一类说明用）。 */
export const LAUNCH_COMMANDS_HOME = isWindows ? "~/.agent-manager/bin 里的命令脚本" : "~/.zshrc 里由本工具管理的那几行";
