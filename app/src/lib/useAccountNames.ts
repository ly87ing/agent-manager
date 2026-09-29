import { accountGroups, accountNameMap, buildAccounts, type AccountGroup } from "./accounts";
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
