import { t } from "./i18n";
/**
 * API Key 存在哪、这个变量现在有没有值：给接入表单和密钥页用一句话说清。
 *
 * 取值只写进用户的外部密钥文件（KEY=VALUE，仅本人可读），本应用的数据库只存文件路径与变量名。
 * 还没有密钥文件时，第一次填 API Key 就在默认位置新建一个，不用先去建文件、改权限。
 */
import type { StatusData } from "./contract";
import type { SecretsMcpEnvObserved } from "./observed";

/** 没有密钥文件时新建的默认位置（不在本应用数据目录里）。 */
export const DEFAULT_SECRETS_PATH = "~/.config/agent-manager/secrets.env";

export interface SecretSourceState {
  configured: boolean;
  ready: boolean;
  path: string | null;
  error: string | null;
  /** 文件里已有的变量名。 */
  names: string[];
}

export function secretSourceOf(status: StatusData | undefined | null): SecretSourceState | null {
  const element = status?.elements.find((item) => item.id === "secrets:mcp-env");
  if (!element) return null;
  const source = (element.observed as SecretsMcpEnvObserved).source ?? {};
  return {
    configured: source.configured === true,
    ready: source.ready === true,
    path: source.path ?? null,
    error: source.error ?? null,
    names: source.names ?? [],
  };
}

/** 接入表单里 API Key 下方的说明：有没有值、填了写到哪。 */
export function keyStatus(
  source: SecretSourceState | null,
  variable: string,
): { tone: "ok" | "warn" | "info"; text: string } {
  const name = variable.trim();
  if (!source) {
    return { tone: "info", text: t("填了只写进你的密钥文件（仅本人可读），不进本应用的数据库。") };
  }
  if (source.configured && !source.ready) {
    return {
      tone: "warn",
      text: t("密钥文件现在用不了（{0}）：先到「设置 → 密钥」处理，否则 API Key 存不进去。", source.error ?? t("读不到")),
    };
  }
  if (!source.configured) {
    return {
      tone: "info",
      text: t("还没有密钥文件：保存时会新建 {0}（仅本人可读），API Key 只写进这个文件。也可以先到「设置 → 密钥」选已有的文件。", DEFAULT_SECRETS_PATH),
    };
  }
  if (!name) return { tone: "info", text: t("填了只写进 {0}（仅本人可读）。", source.path) };
  if (source.names.includes(name)) {
    return { tone: "ok", text: t("{0} 里已有 {1} 的值：留空保持不变，填了就替换。", source.path, name) };
  }
  return { tone: "warn", text: t("{0} 里还没有 {1}：填上才能用；只写进这个文件，不进本应用的数据库。", source.path, name) };
}
