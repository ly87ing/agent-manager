import { t } from "./i18n";
import { QueryClient, useIsMutating, useMutation, useQuery, useQueryClient, type UseQueryResult } from "@tanstack/react-query";
import { toast } from "sonner";
import { engineErrorMessage, runEngine } from "./engine";
import type { Envelope, RefreshUpstreamData } from "./contract";

export const engineKeys = {
  all: ["engine"] as const,
  command: (command: string) => ["engine", command] as const,
  request: (command: string, args: readonly string[] = []) => ["engine", command, ...args] as const,
};

export function createQueryClient(options?: { retry?: number | false }) {
  return new QueryClient({
    defaultOptions: {
      queries: {
        staleTime: 30_000,
        gcTime: 60_000,
        retry: options?.retry ?? 1,
        refetchOnWindowFocus: false,
      },
    },
  });
}

/** 以引擎命令为 key 的只读查询（status/audit 均为 read 级、零网络）。 */
export function useEngineQuery<T>(
  command: string,
  args?: string[],
  options?: { enabled?: boolean; refetchInterval?: number },
): UseQueryResult<Envelope<T>> {
  const normalizedArgs = args ? [...args] : [];
  return useQuery({
    queryKey: engineKeys.request(command, normalizedArgs),
    queryFn: () => runEngine<T>(command, normalizedArgs),
    enabled: options?.enabled ?? true,
    refetchInterval: options?.refetchInterval,
    refetchIntervalInBackground: false,
  });
}

/**
 * 「检查更新」——刷新各组件的官方最新版本（system:refresh-upstream，需要联网，
 * 只更新本地检查记录，不改配置）。传 only 时只重新检查一个元素（--only <元素id>）。
 */
export const REFRESH_UPSTREAM_KEY = ["refresh-upstream"] as const;

/** 是否有「检查新版本」正在进行（任何页面、命令面板发起的都算）。 */
export function useCheckingUpdates(): boolean {
  return useIsMutating({ mutationKey: REFRESH_UPSTREAM_KEY }) > 0;
}

export function useRefreshUpstream() {
  const queryClient = useQueryClient();
  return useMutation({
    // 带键：别处（如程序页）据此知道「正在检查新版本」，检查完成前先不让升级
    mutationKey: REFRESH_UPSTREAM_KEY,
    mutationFn: (only?: string) =>
      runEngine<RefreshUpstreamData>("system:refresh-upstream", only ? ["--only", only] : []),
    onSuccess: (envelope, only) => {
      if (envelope.ok) {
        const summary = envelope.data.summary;
        if (only) {
          const entry = envelope.data.elements.find((item) => item.id === only);
          toast.success(
            entry
              ? t("已重新检查 {0}{1}", only, entry.upstream ? t("：官方最新 {0}", entry.upstream) : "")
              : t("已重新检查 {0}", only),
          );
        } else {
          toast.success(
            t("检查更新完成：已是最新 {0} · 可更新 {1} · 比官方新 {2} · 未检查 {3}", summary.current, summary.behind, summary.ahead, summary.unknown),
          );
        }
      } else {
        toast.error(envelope.errors[0]?.message ?? t("检查更新失败"));
      }
      void queryClient.invalidateQueries({ queryKey: engineKeys.all });
    },
    onError: (err) => {
      toast.error(t("检查更新调用失败：{0}", engineErrorMessage(err)));
    },
  });
}

/** Conservative cross-domain dependencies; unknown writers refresh everything. */
export function affectedDomains(command: string): string[] | null {
  const domain = command.replaceAll(".", ":").split(":")[0];
  const dependencies: Record<string, string[]> = {
    mcp: ["mcp", "agents", "profiles", "secrets", "programs"],
    skills: ["skills", "rules", "aliases", "programs"],
    rules: ["rules", "agents", "profiles"],
    aliases: ["aliases", "accounts"],
    runs: ["runs"],
  };
  return dependencies[domain] ? [...new Set([...dependencies[domain], "runs", "system"])] : null;
}

export function invalidateEngineQueries(client: QueryClient, command: string): Promise<void> {
  const domains = affectedDomains(command);
  return client.invalidateQueries({ queryKey: engineKeys.all, predicate: query => !domains || domains.includes(String(query.queryKey[1] ?? "").split(":")[0]) });
}
