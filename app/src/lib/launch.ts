import { runEngine } from "./engine";

export interface LaunchRecipe {
  platform: "macos" | "linux" | "windows";
  shell: "zsh" | "bash" | "powershell";
  kind: "app" | "command";
  target: string;
  args: string[];
  env: { name: string; value: string }[];
  variants: { name: string; timezone: string }[];
}
export interface LaunchEntry { name: string; timezone: string; command: string; definition: string }
export interface LaunchCatalog { platform: LaunchRecipe["platform"]; shell: LaunchRecipe["shell"]; apps: { id?: string; name: string; path: string; args?: string[] }[] }
export interface SavedLaunch { id: string; revision: string; recipe: LaunchRecipe; entries: LaunchEntry[] }

export async function launchRequest<T>(operation: string, args: string[] = []): Promise<T> {
  const result = await runEngine<T>(`launch:${operation}`, args,
    ["save", "delete", "run"].includes(operation) ? { runId: crypto.randomUUID() } : undefined);
  if (!result.ok) throw new Error(result.errors.map((error) => error.message).join("\n"));
  return result.data;
}

const COMMON_TIMEZONES = [
  "America/Los_Angeles",
  "America/New_York",
  "Asia/Shanghai",
  "Asia/Hong_Kong",
  "Asia/Singapore",
  "Asia/Tokyo",
  "Europe/London",
  "Europe/Berlin",
  "UTC",
];

/**
 * 可选时区列表：常用时区置顶（否则要在几百项里翻找），其后接运行环境的完整 IANA 列表；
 * 环境不提供完整列表时只返回常用时区。
 */
export function listTimeZones(): string[] {
  const supported = (Intl as { supportedValuesOf?: (key: string) => string[] }).supportedValuesOf;
  let all: string[] = [];
  if (typeof supported === "function") {
    try {
      const values = supported("timeZone");
      if (Array.isArray(values)) all = values;
    } catch {
      all = [];
    }
  }
  return Array.from(new Set([...COMMON_TIMEZONES, ...all]));
}
