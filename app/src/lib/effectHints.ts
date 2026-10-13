import { t } from "./i18n";
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
    case "mcp:apply":
      return t("助手下次启动时读取；正在运行的会话要重开才能用上。");
    case "rules:apply":
      if (only === "rules:codex_global") return t("Codex 下次启动时读取新规则。");
      if (only === "rules:claude_global") return t("Claude Code 下次启动时读取新规则；正在跑的会话不受影响。");
      // 实例目标（rules:profile:*）的 id 看不出是哪种助手，给通用说法
      return t("助手下次启动时读取新规则；正在跑的会话不受影响。");
    case "skills:apply":
      return t("助手下次启动时扫描技能目录；正在跑的会话看不到刚加的技能。");
    default:
      return null;
  }
}
