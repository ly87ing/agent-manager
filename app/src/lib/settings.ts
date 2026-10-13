import { invoke } from "@tauri-apps/api/core";
import { mockModeFromSearch } from "./engine";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { z } from "zod";
import { useLayoutEffect } from "react";
import type { SettingsData, SettingsPreferences } from "./contract";
import { setLanguage } from "./i18n";

export const settingsKey = ["settings"] as const;

export const settingsSchema = z
  .object({
    preferences: z
      .object({
        language: z.enum(["en", "zh-CN"]).default("en"),
        logRetentionDays: z.number().int().min(0).max(3650),
      })
      .strict(),
    appVersion: z.string(),
    contractVersion: z.string(),
    resourceDir: z.string(),
    dataDir: z.string(),
    updaterConfigured: z.boolean(),
    runtimeWarning: z.string().optional(),
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
  if (!devMockValue) devMockValue = { preferences: { language: "en", logRetentionDays: 30 }, appVersion: "0.1.0-demo", contractVersion: "agent-manager/engine/v1", resourceDir: "/Applications/Agent Manager.app/Contents/Resources", dataDir: "/Users/demo/Library/Application Support/Agent Manager", updaterConfigured: false };
  return { get: async () => devMockValue!, save: async (preferences) => (devMockValue = { ...devMockValue!, preferences }) };
}
function devMockEnabled(): boolean {
  // 与引擎侧共用同一个开关解析，免得新增 mock 模式时两处清单走散
  return mockModeFromSearch(window.location.search, import.meta.env.DEV) !== null;
}
export async function fetchSettings(): Promise<SettingsData> {
  return (devMockEnabled() ? devMockClient() : getSettingsClient()).get();
}
export function useSettings() {
  const query = useQuery({ queryKey: settingsKey, queryFn: fetchSettings });
  useLayoutEffect(() => {
    if (query.data) setLanguage(query.data.preferences.language);
  }, [query.data?.preferences.language]);
  return query;
}

export function useSaveSettings() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (preferences: SettingsPreferences) => (devMockEnabled() ? devMockClient() : getSettingsClient()).save(preferences),
    onSuccess: (data) => { setLanguage(data.preferences.language); queryClient.setQueryData(settingsKey, data); void queryClient.invalidateQueries({ queryKey: settingsKey }); void queryClient.invalidateQueries({ queryKey: ["engine"] }); },
  });
}
