import type { EngineElement, ProfileObserved, ProfileSpec } from "./contract";
import { agentManagementOf, connectionKindOf } from "./agentManagement";
import { isOfficial } from "./defaultEnvironment";
import { parseAliasValue } from "./aliasCommand";
import type { AgentsObserved, AgentsSpec, AgentUsage } from "./observed";
import { launchCommandsForProfile, profileKindOf, type ProfileKind } from "./profileDefinition";

/**
 * 账号 = 一个配置目录 + 一条启动命令（见 docs/MULTI-ACCOUNT-DESIGN.md）。
 *
 * 主账号来自 agents:status 的主助手（`~/.claude` / `~/.codex`，命令就是助手本身）；
 * 其余账号来自 profiles:status 的实例。这里只做只读拼装，不引入新的状态。
 */
export interface Account {
  /** 引擎元素 id：主账号 `agents:<助手>`，其余 `profiles:<实例>`；auth-status 直接用它。 */
  elementId: string;
  name: string;
  isMain: boolean;
  /** 终端里输入的命令；实例还没有启动命令时为 null。 */
  command: string | null;
  configDir: string;
  /** api = 第三方接口或凭据；official = 在目录里登录官方账号。 */
  type: "official" | "api";
  /** API 账号的接口主机与模型，给一行里的简短说明用。 */
  apiHost: string | null;
  model: string | null;
  usage: AgentUsage | null;
  /** 目录内容与登记的不一致（外部改过）。 */
  drifted: boolean;
  /** 主账号里混进了第三方接入：按产品规则主账号只放官方登录。 */
  mainHasThirdParty: boolean;
  /** 登记本身按现有规则读不了。 */
  blockedReason: string | null;
  /** 目录配置文件的写入基准（profiles:status observed.base）；主账号为 null。 */
  profileBase: string | null;
}

export interface AgentAccounts {
  agentId: string;
  label: string;
  kind: ProfileKind;
  bin: string;
  accounts: Account[];
}

function hostOf(url: string | null | undefined): string | null {
  if (!url) return null;
  try {
    return new URL(url).host || url;
  } catch {
    return url;
  }
}

/** 界面上的助手名：Claude Code 简称 Claude。 */
export function agentLabel(displayName: string): string {
  return displayName === "Claude Code" ? "Claude" : displayName;
}

/** 实例名常带「助手 · 」前缀（旧写法）；同一分组里已经知道是哪个助手，去掉它。 */
function shortName(displayName: string, labels: string[]): string {
  const prefix = labels.map((label) => `${label} · `).find((candidate) => displayName.startsWith(candidate));
  return prefix ? displayName.slice(prefix.length) : displayName;
}

export function buildAccounts(
  agentElements: EngineElement[],
  profileElements: EngineElement[],
  aliases: Record<string, string>,
): AgentAccounts[] {
  const profileIds = new Set(profileElements.map((element) => element.id.replace(/^profiles:/, "")));
  return agentElements
    .filter((element) => !profileIds.has(element.id.replace(/^agents:/, "")))
    .filter((element) => connectionKindOf(element) !== null && agentManagementOf(element)?.supports_profiles)
    .map((element) => {
      const agentId = element.id.replace(/^agents:/, "");
      const kind = connectionKindOf(element) as ProfileKind;
      const spec = element.spec as AgentsSpec;
      const observed = element.observed as AgentsObserved;
      const fullLabel = spec.display_name ?? agentId;
      const label = agentLabel(fullLabel);
      const bin = spec.bin ?? (kind === "codex" ? "codex" : "claude");
      const main: Account = {
        elementId: element.id,
        name: label,
        isMain: true,
        command: bin,
        configDir: kind === "codex" ? "~/.codex" : "~/.claude",
        type: "official",
        apiHost: null,
        model: null,
        usage: observed.usage ?? null,
        drifted: false,
        mainHasThirdParty: Boolean(observed.current_config?.current) && !isOfficial(observed.current_config?.current ?? null),
        blockedReason: null,
        profileBase: null,
      };
      const others = profileElements
        .filter((profile) => (profile.spec as unknown as ProfileSpec).base_agent === agentId)
        .map((profile): Account => {
          const pSpec = profile.spec as unknown as ProfileSpec;
          const pObserved = profile.observed as unknown as ProfileObserved;
          const configDir = pObserved.config_dir ?? pSpec.config_dir;
          const launch = launchCommandsForProfile(
            aliases,
            profileKindOf(pSpec.base_agent),
            configDir,
            (value) => parseAliasValue(value).otherEnv,
          );
          const api = pObserved.third_party ?? Boolean(pSpec.base_url);
          return {
            elementId: profile.id,
            name: shortName(pSpec.display_name || profile.id.replace(/^profiles:/, ""), [fullLabel, label]),
            isMain: false,
            command: launch[0]?.name ?? null,
            configDir,
            type: api ? "api" : "official",
            apiHost: api ? hostOf(pObserved.current_connection ? pObserved.current_connection.base_url : pSpec.base_url) : null,
            model: pObserved.current_connection ? pObserved.current_connection.model : pSpec.model ?? null,
            usage: pObserved.usage ?? null,
            drifted: profile.state === "drifted",
            mainHasThirdParty: false,
            blockedReason: pObserved.blocked_reason ?? null,
            profileBase: pObserved.base ?? null,
          };
        })
        .sort((a, b) => a.name.localeCompare(b.name));
      return { agentId, label, kind, bin, accounts: [main, ...others] };
    });
}

/**
 * 资源页用的账号名：技能 / MCP 格子里的键是助手 id（主账号）或实例 id，换成账号页上的名字。
 * 读不到的键原样返回。
 */
export function accountNameMap(
  agentElements: EngineElement[],
  profileElements: EngineElement[],
  aliases: Record<string, string>,
): (agent: string) => string {
  const names = new Map<string, string>();
  for (const group of buildAccounts(agentElements, profileElements, aliases)) {
    for (const account of group.accounts) {
      names.set(account.isMain ? group.agentId : account.elementId.replace(/^profiles:/, ""), account.isMain ? group.label : account.name);
    }
  }
  return (agent) => names.get(agent) ?? agent;
}

export interface AccountLaunchCommand {
  name: string;
  value: string;
  /** false = 写在 Agent Manager 区块外、你自己维护的 alias：只读。 */
  managed: boolean;
  needsTildeFix: boolean;
}

/**
 * 指向这个账号的全部启动命令：独立账号按命令里的配置目录变量认；主账号是直接调用助手命令、
 * 没有指定配置目录的那些（如 `alias claude='TZ=… claude --…'`）。
 */
export function launchCommandsOfAccount(
  account: Account,
  group: AgentAccounts,
  managed: Record<string, string>,
  external: { name: string; value: string | null }[],
): AccountLaunchCommand[] {
  const envName = group.kind === "codex" ? "CODEX_HOME" : "CLAUDE_CONFIG_DIR";
  const binName = group.bin.split("/").pop() ?? group.bin;
  const collect = (entries: [string, string][], isManaged: boolean): AccountLaunchCommand[] => {
    if (account.isMain) {
      return entries
        .filter(([, value]) => {
          const parsed = parseAliasValue(value);
          return (parsed.command.split("/").pop() ?? parsed.command) === binName && parsed.otherEnv[envName] === undefined;
        })
        .map(([name, value]) => ({ name, value, managed: isManaged, needsTildeFix: false }));
    }
    return launchCommandsForProfile(Object.fromEntries(entries), group.kind, account.configDir, (value) => parseAliasValue(value).otherEnv)
      .map((command) => ({ ...command, managed: isManaged }));
  };
  const externalEntries = external
    .filter((line): line is { name: string; value: string } => typeof line.value === "string" && managed[line.name] === undefined)
    .map((line) => [line.name, line.value] as [string, string]);
  return [...collect(Object.entries(managed), true), ...collect(externalEntries, false)];
}

/** 资源表格的一列：一个账号。 */
export interface AccountColumn {
  /** 技能 / MCP 格子里的键：主账号是助手 id，其余是实例 id。 */
  id: string;
  /** 表头第二层显示的短名：主账号叫「主号」。 */
  name: string;
  /** 悬停提示里的全名，如「Claude · 工作」。 */
  fullName: string;
}

/** 表头第一层：一个助手及其账号列。 */
export interface AccountGroup {
  key: string;
  label: string;
  kind: ProfileKind | null;
  columns: AccountColumn[];
}

/**
 * 把资源格子里出现的账号键按助手分组，顺序同账号页（主账号在前）。
 * 账号页不认识的键（如不支持多账号的助手）各自成组，列名用「主号」。
 */
export function accountGroups(groups: AgentAccounts[], ids: string[]): AccountGroup[] {
  const wanted = new Set(ids);
  const used = new Set<string>();
  const result: AccountGroup[] = [];
  for (const group of groups) {
    const columns = group.accounts
      .map((account) => {
        const id = account.isMain ? group.agentId : account.elementId.replace(/^profiles:/, "");
        return {
          id,
          name: account.isMain ? "主号" : account.name,
          fullName: account.isMain ? group.label : `${group.label} · ${account.name}`,
        };
      })
      .filter((column) => wanted.has(column.id));
    columns.forEach((column) => used.add(column.id));
    if (columns.length) result.push({ key: group.agentId, label: group.label, kind: group.kind, columns });
  }
  for (const id of ids) {
    if (used.has(id)) continue;
    used.add(id);
    result.push({ key: id, label: id, kind: null, columns: [{ id, name: "主号", fullName: id }] });
  }
  return result;
}
