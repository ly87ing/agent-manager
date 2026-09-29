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

/** 实例名常带「助手 · 」前缀（旧写法）；同一分组里已经知道是哪个助手，去掉它。 */
function shortName(displayName: string, agentLabel: string): string {
  const prefix = `${agentLabel} · `;
  return displayName.startsWith(prefix) ? displayName.slice(prefix.length) : displayName;
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
      const label = spec.display_name ?? agentId;
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
            name: shortName(pSpec.display_name || profile.id.replace(/^profiles:/, ""), label),
            isMain: false,
            command: launch[0]?.name ?? null,
            configDir,
            type: api ? "api" : "official",
            apiHost: api ? hostOf(pSpec.base_url) : null,
            model: pSpec.model ?? null,
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
