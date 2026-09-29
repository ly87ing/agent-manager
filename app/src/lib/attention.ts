import { buildAccounts } from "./accounts";
import { missingVariables } from "./mcpVariables";
import type { EngineElement, StatusData } from "./contract";
import { useEngineQuery } from "./queries";
import type { AliasesObserved, McpObserved, RulesObserved, SkillObserved, SkillSpec } from "./observed";
import { rulesFillRows } from "./rulesFill";
import { skillChip } from "./skillChip";

/**
 * 导航角标：各页已经标黄、需要用户处理的项数。只汇总页面里已有的判断，不引入新规则；
 * 数据都来自只读的 status 查询。
 */

/** 技能：有黄色开关（损坏、占用、来源已删、外部链接）或新发现待添加的技能数。 */
export function skillsAttention(elements: EngineElement[]): number {
  return elements.filter((element) => {
    if (!(element.spec as SkillSpec).collection) return false;
    const observed = element.observed as SkillObserved;
    if (observed.discovery_inbox === true) return true;
    return Object.values(observed.agents ?? {}).some((cell) => skillChip(cell).tone === "warn");
  }).length;
}

/** MCP：条目无法识别、找不到要执行的命令或变量没有取值的服务数。 */
export function mcpAttention(elements: EngineElement[]): number {
  return elements.filter((element) => {
    const cells = Object.values((element.observed as McpObserved).agents ?? {});
    return (
      cells.some((cell) => cell.state === "invalid" || (cell.state === "enabled" && cell.command_available === false)) ||
      missingVariables(element).length > 0
    );
  }).length;
}

/** 规则：来源有新版本、改过且来源也有新版，或还没有规则文件、可以从主号补齐的账号数。 */
export function rulesAttention(elements: EngineElement[]): number {
  const fill = new Set(rulesFillRows(elements).map((row) => row.elementId));
  return elements.filter((element) => {
    const follow = (element.observed as unknown as RulesObserved).source_follow;
    const changedSinceFollow =
      Boolean(follow) && !follow!.file_unchanged && Boolean(follow!.current_revision) && follow!.current_revision !== follow!.applied_revision;
    return fill.has(element.id) || Boolean(follow?.update_available) || changedSinceFollow;
  }).length;
}

/** 账号：目录被外部改过、没有启动命令、登记读不了或主账号混进第三方接入的账号数。 */
export function accountsAttention(agents: EngineElement[], profiles: EngineElement[], aliases: Record<string, string>): number {
  return buildAccounts(agents, profiles, aliases)
    .flatMap((group) => group.accounts)
    .filter((account) => account.drifted || account.blockedReason || account.mainHasThirdParty || (!account.isMain && !account.command)).length;
}

/** 启动命令：两个终端配置文件里写法不同，或只在其中一个里有的命令数。 */
export function commandsAttention(targets: EngineElement[]): number {
  const writable = targets.map((target) => target.observed as unknown as AliasesObserved).filter((observed) => observed.base !== null);
  const lines = new Map<string, string[]>();
  for (const observed of writable) {
    for (const entry of observed.entries ?? []) lines.set(entry.name, [...(lines.get(entry.name) ?? []), entry.line]);
  }
  return [...lines.values()].filter((found) => found.length < writable.length || new Set(found).size > 1).length;
}

/** 各导航项的待处理数；查询还没回来的按 0。 */
export function useAttentionCounts(): Record<string, number> {
  const skills = useEngineQuery<StatusData>("skills:status");
  const mcp = useEngineQuery<StatusData>("mcp:status");
  const rules = useEngineQuery<StatusData>("rules:status");
  const agents = useEngineQuery<StatusData>("agents:status");
  const profiles = useEngineQuery<StatusData>("profiles:status");
  const aliases = useEngineQuery<StatusData>("aliases:status");
  const elementsOf = (query: { data?: { ok: boolean; data: StatusData } }) => (query.data?.ok ? query.data.data.elements ?? [] : []);
  const aliasLines = elementsOf(aliases).flatMap((element) => (element.observed as unknown as AliasesObserved).entries ?? []);
  return {
    skills: skillsAttention(elementsOf(skills)),
    mcp: mcpAttention(elementsOf(mcp)),
    "rules-hooks": rulesAttention(elementsOf(rules)),
    accounts: accountsAttention(
      elementsOf(agents),
      elementsOf(profiles),
      Object.fromEntries(aliasLines.filter((line) => line.value).map((line) => [line.name, line.value as string])),
    ),
    commands: commandsAttention(elementsOf(aliases)),
  };
}
