import { accountGroups, accountNameMap, buildAccounts, launchCommandsOfAccount, type AccountGroup } from "./accounts";
import type { EngineElement, StatusData } from "./contract";
import { useEngineQuery } from "./queries";

/** 账号页用的三份只读数据：主助手、实例与启动命令。 */
function useAccountSources(): [EngineElement[], EngineElement[], Record<string, string>] {
  const agents = useEngineQuery<StatusData>("agents:status");
  const profiles = useEngineQuery<StatusData>("profiles:status");
  const aliases = useEngineQuery<StatusData>("aliases:status");
  const lines = (aliases.data?.ok ? (aliases.data.data.aliases ?? []) : []) as { name: string; value: string | null }[];
  return [
    agents.data?.ok ? agents.data.data.elements : [],
    profiles.data?.ok ? profiles.data.data.elements : [],
    Object.fromEntries(lines.filter((line) => line.value).map((line) => [line.name, line.value as string])),
  ];
}

/** 技能、MCP 等页面把助手 / 实例 id 显示成账号名。 */
export function useAccountNames(): (agent: string) => string {
  return accountNameMap(...useAccountSources());
}

/** 技能、MCP 表格的表头：把格子里出现的账号键按助手分组。 */
export function useAccountGroups(): (ids: string[]) => AccountGroup[] {
  const groups = buildAccounts(...useAccountSources());
  return (ids) => accountGroups(groups, ids);
}

/** 启动命令启动的是哪个账号：按命令里的配置目录变量认，直接调用助手命令的是主账号。 */
export function useCommandAccounts(
  managed: Record<string, string>,
  external: { name: string; value: string | null }[],
): (name: string) => { kind: "claude" | "codex"; label: string } | null {
  const sources = useAccountSources();
  const byName = new Map<string, { kind: "claude" | "codex"; label: string }>();
  for (const group of buildAccounts(sources[0], sources[1], managed)) {
    for (const account of group.accounts) {
      for (const command of launchCommandsOfAccount(account, group, managed, external)) {
        byName.set(command.name, { kind: group.kind, label: account.isMain ? `${group.label} · 主号` : `${group.label} · ${account.name}` });
      }
    }
  }
  return (name) => byName.get(name) ?? null;
}
