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
export interface SavedLaunch { id: string; revision: string; recipe: LaunchRecipe; entries: LaunchEntry[] }

export async function launchRequest<T>(operation: string, args: string[] = []): Promise<T> {
  const result = await runEngine<T>(`launch:${operation}`, args,
    ["save", "delete", "run"].includes(operation) ? { runId: crypto.randomUUID() } : undefined);
  if (!result.ok) throw new Error(result.errors.map((error) => error.message).join("\n"));
  return result.data;
}
