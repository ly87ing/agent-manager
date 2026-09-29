import { accountNameMap } from "./accounts";
import type { StatusData } from "./contract";
import { useEngineQuery } from "./queries";

/** 技能、MCP 等页面把助手 / 实例 id 显示成账号名。 */
export function useAccountNames(): (agent: string) => string {
  const agents = useEngineQuery<StatusData>("agents:status");
  const profiles = useEngineQuery<StatusData>("profiles:status");
  const aliases = useEngineQuery<StatusData>("aliases:status");
  const lines = (aliases.data?.ok ? (aliases.data.data.aliases ?? []) : []) as { name: string; value: string | null }[];
  return accountNameMap(
    agents.data?.ok ? agents.data.data.elements : [],
    profiles.data?.ok ? profiles.data.data.elements : [],
    Object.fromEntries(lines.filter((line) => line.value).map((line) => [line.name, line.value as string])),
  );
}
