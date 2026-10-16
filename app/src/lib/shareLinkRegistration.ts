import { invoke } from "@tauri-apps/api/core";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { mockModeFromSearch } from "./engine";

export interface ShareLinkPreview {
  base: string;
  executable: string;
  entry: string;
  current_handler: string;
  registered: boolean;
}
export interface ShareLinkRegistrationClient {
  preview(): Promise<ShareLinkPreview | null>;
  register(base: string): Promise<ShareLinkPreview>;
}
const nativeClient: ShareLinkRegistrationClient = {
  preview: () => invoke("share_links_preview"),
  register: (base) => invoke("share_links_register", { base }),
};
let activeClient = nativeClient;
export function setShareLinkRegistrationClient(client: ShareLinkRegistrationClient | null) { activeClient = client ?? nativeClient; }
const mockClient: ShareLinkRegistrationClient = {
  preview: async () => null,
  register: async () => { throw new Error("No AppImage registration in this preview."); },
};
function client() {
  return mockModeFromSearch(window.location.search, import.meta.env.DEV) !== null ? mockClient : activeClient;
}
const key = ["share-link-registration"] as const;
export function useShareLinkRegistration() { return useQuery({ queryKey: key, queryFn: () => client().preview() }); }
export function useRegisterShareLinks() {
  const cache = useQueryClient();
  return useMutation({
    mutationFn: (base: string) => client().register(base),
    onSuccess: (actual) => cache.setQueryData(key, actual),
    onSettled: () => { void cache.invalidateQueries({ queryKey: key }); },
  });
}
