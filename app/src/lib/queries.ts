import { QueryClient, useMutation, useQuery, useQueryClient, type UseQueryResult } from "@tanstack/react-query";
import { toast } from "sonner";
import { engineErrorMessage, runEngine } from "./engine";
import type { AuditData, Envelope, RefreshUpstreamData } from "./contract";

export const engineKeys = {
  all: ["engine"] as const,
  command: (command: string) => ["engine", command] as const,
};

export function createQueryClient(options?: { retry?: number | false }) {
  return new QueryClient({
    defaultOptions: {
      queries: {
        staleTime: 30_000,
        retry: options?.retry ?? 1,
        refetchOnWindowFocus: false,
      },
    },
  });
}

/** 全局 offline 模式（读 system:audit envelope.mode，缓存共享）。 */
export function useOfflineMode(): boolean {
  const audit = useEngineQuery<AuditData>("system:audit");
  return audit.data?.mode.offline ?? false;
}

/** 以引擎命令为 key 的只读查询（status/audit 均为 read 级、零网络）。 */
export function useEngineQuery<T>(command: string, args?: string[]): UseQueryResult<Envelope<T>> {
  return useQuery({
    queryKey: engineKeys.command(command),
    queryFn: () => runEngine<T>(command, args),
  });
}

/**
 * 「检查更新」——刷新各组件的官方最新版本（system:refresh-upstream，需要联网，
 * 只更新本地检查记录，不改配置）。传 only 时只重新检查一个元素（--only <元素id>）。
 */
export function useRefreshUpstream() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (only?: string) =>
      runEngine<RefreshUpstreamData>("system:refresh-upstream", only ? ["--only", only] : []),
    onSuccess: (envelope, only) => {
      if (envelope.ok) {
        const summary = envelope.data.summary;
        if (only) {
          const entry = envelope.data.elements.find((item) => item.id === only);
          toast.success(
            entry
              ? `已重新检查 ${only}${entry.upstream ? `：官方最新 ${entry.upstream}` : ""}`
              : `已重新检查 ${only}`,
          );
        } else {
          toast.success(
            `检查更新完成：已是最新 ${summary.current} · 可更新 ${summary.behind} · 比官方新 ${summary.ahead} · 未检查 ${summary.unknown}`,
          );
        }
      } else {
        toast.error(envelope.errors[0]?.message ?? "检查更新失败");
      }
      void queryClient.invalidateQueries({ queryKey: engineKeys.all });
    },
    onError: (err) => {
      toast.error(`检查更新调用失败：${engineErrorMessage(err)}`);
    },
  });
}
