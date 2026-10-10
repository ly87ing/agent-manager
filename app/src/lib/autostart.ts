import { invoke } from "@tauri-apps/api/core";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { mockModeFromSearch } from "./engine";

export interface AutostartClient {
  get(): Promise<boolean>;
  set(enabled: boolean): Promise<boolean>;
}

const nativeClient: AutostartClient = {
  get: () => invoke<boolean>("autostart_get"),
  set: (enabled) => invoke<boolean>("autostart_set", { enabled }),
};
let activeClient = nativeClient;
export function setAutostartClient(client: AutostartClient | null) { activeClient = client ?? nativeClient; }

let mockEnabled = false;
const mockClient: AutostartClient = {
  get: async () => mockEnabled,
  set: async (enabled) => (mockEnabled = enabled),
};
function client() {
  return mockModeFromSearch(window.location.search, import.meta.env.DEV) !== null ? mockClient : activeClient;
}

const key = ["autostart"] as const;
export function useAutostart() { return useQuery({ queryKey: key, queryFn: () => client().get() }); }
export function useSetAutostart() {
  const cache = useQueryClient();
  return useMutation({
    mutationFn: (enabled: boolean) => client().set(enabled),
    onSuccess: (enabled) => cache.setQueryData(key, enabled),
    onSettled: () => { void cache.invalidateQueries({ queryKey: key }); },
  });
}
