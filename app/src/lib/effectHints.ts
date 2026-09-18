/**
 * 「什么时候真正生效」的提示。
 *
 * 写入配置文件不等于助手立刻用上：Claude Code 与 Codex 都在进程启动时读配置，
 * 终端别名要新开窗口才加载。执行成功后把这句话直接说给用户，省掉「为什么没变化」的困惑。
 * 文案按目标而不是按域给：同一个域写不同目标，生效条件不一样。
 */
export function effectHint(command: string, args: readonly string[] = []): string | null {
  const only = (() => {
    const index = args.indexOf("--only");
    return index >= 0 ? (args[index + 1] ?? "") : "";
  })();

  switch (command) {
    case "aliases:apply":
      return "新开一个终端窗口后生效；已经开着的窗口不受影响。";
    case "profiles:apply":
      return "下次用这个实例的命令启动时生效；已经在跑的会话不受影响。";
    case "mcp:apply":
      return "助手下次启动时读取；正在运行的会话要重开才能用上。";
    case "rules:apply":
      return only.includes("codex")
        ? "Codex 下次启动时读取新规则。"
        : "Claude Code 下次启动时读取新规则；正在跑的会话不受影响。";
    case "hooks:apply":
      return "助手下次启动时加载；正在跑的会话不受影响。";
    case "skills:apply":
      return "助手下次启动时扫描技能目录；正在跑的会话看不到刚加的技能。";
    case "agents:apply":
      // 安装/升级/卸载本身就是即时结果，不需要额外说明
      return null;
    default:
      return null;
  }
}
