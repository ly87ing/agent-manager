import { invoke } from "@tauri-apps/api/core";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { z } from "zod";
import type { SettingsData, SettingsPreferences } from "./contract";

export const settingsKey = ["settings"] as const;

export const settingsSchema = z
  .object({
    preferences: z
      .object({
        offline: z.boolean(),
        upstreamTtlSec: z.number().int().min(0),
        logRetentionDays: z.number().int().min(0).max(3650),
      })
      .strict(),
    appVersion: z.string(),
    contractVersion: z.string(),
    resourceDir: z.string(),
    dataDir: z.string(),
    logDir: z.string(),
    updaterConfigured: z.boolean(),
    maintenanceWarning: z.string().nullable().optional(),
  })
  .strict();

export interface SettingsClient { get(): Promise<SettingsData>; save(preferences: SettingsPreferences): Promise<SettingsData>; }
const tauriSettingsClient: SettingsClient = {
  async get() { return settingsSchema.parse(await invoke<unknown>("settings_get")); },
  async save(preferences) { return settingsSchema.parse(await invoke<unknown>("settings_save", { preferences })); },
};
let activeSettingsClient = tauriSettingsClient;
export function setSettingsClient(client: SettingsClient | null): void { activeSettingsClient = client ?? tauriSettingsClient; }
export function getSettingsClient(): SettingsClient { return activeSettingsClient; }

let devMockValue: SettingsData | null = null;
function devMockClient(): SettingsClient {
  if (!devMockValue) devMockValue = { preferences: { offline: new URLSearchParams(window.location.search).get("mock") === "offline", upstreamTtlSec: 86400, logRetentionDays: 30 }, appVersion: "0.1.0-demo", contractVersion: "agent-manager/engine/v1", resourceDir: "/Applications/Agent Manager.app/Contents/Resources", dataDir: "/Users/demo/Library/Application Support/Agent Manager", logDir: "/Users/demo/Library/Logs/Agent Manager", updaterConfigured: false, maintenanceWarning: null };
  return { get: async () => devMockValue!, save: async (preferences) => (devMockValue = { ...devMockValue!, preferences }) };
}
function devMockEnabled(): boolean {
  const mode = new URLSearchParams(window.location.search).get("mock");
  return import.meta.env.DEV && (mode === "1" || mode === "offline");
}
export async function fetchSettings(): Promise<SettingsData> {
  if (devMockEnabled()) return devMockClient().get();
  return getSettingsClient().get();
}
export function useSettings() { return useQuery({ queryKey: settingsKey, queryFn: fetchSettings }); }

export function useSaveSettings() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (preferences: SettingsPreferences) => (devMockEnabled() ? devMockClient() : getSettingsClient()).save(preferences),
    onSuccess: (data) => { queryClient.setQueryData(settingsKey, data); void queryClient.invalidateQueries({ queryKey: settingsKey }); void queryClient.invalidateQueries({ queryKey: ["engine"] }); },
  });
}
