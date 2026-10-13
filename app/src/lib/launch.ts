import { runEngine } from "./engine";

export interface LaunchRecipe {
  platform: "macos" | "linux" | "windows";
  shell: "zsh" | "bash" | "powershell";
  kind: "app" | "command";
  target: string;
  args: string[];
  variants: { name: string; timezone: string }[];
}
export interface LaunchEntry { name: string; timezone: string; command: string; definition: string }
export interface LaunchCatalog { platform: LaunchRecipe["platform"]; shell: LaunchRecipe["shell"]; apps: { name: string; path: string }[] }
export interface LaunchStatus {
  platform: string; shell: string; path: string; base: string;
  entries: { name: string; command: string | null; definition: string; path: string }[];
  external_names: string[];
}
export interface LaunchPreview { base: string; files: { path: string; original: string | null; content: string; deleted: boolean }[] }

export async function launchRequest<T>(operation: string, args: string[] = []): Promise<T> {
  const result = await runEngine<T>(`launch:${operation}`, args,
    operation === "apply" || operation === "run" ? { runId: crypto.randomUUID() } : undefined);
  if (!result.ok) throw new Error(result.errors.map((error) => error.message).join("\n"));
  return result.data;
}
