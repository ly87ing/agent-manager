import { QueryClient, useMutation, useQuery, useQueryClient, type UseQueryResult } from "@tanstack/react-query";
import { toast } from "sonner";
import { engineErrorMessage, runEngine } from "./engine";
import type { Envelope, RefreshUpstreamData } from "./contract";

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

/** 以引擎命令为 key 的只读查询（status/audit 均为 read 级、零网络）。 */
export function useEngineQuery<T>(command: string, args?: string[]): UseQueryResult<Envelope<T>> {
  return useQuery({
    queryKey: engineKeys.command(command),
    queryFn: () => runEngine<T>(command, args),
  });
}

/**
 * 「刷新上游状态」——本阶段唯一主动作（system:refresh-upstream，network 级探测缓存，不写配置）。
 * 传 only 时单元素重探（--only <element-id>）。成功后全部引擎查询失效重拉。
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
              ? `已重新探测 ${only}：${entry.state}${entry.upstream ? `（上游 ${entry.upstream}）` : ""}`
              : `已重新探测 ${only}`,
          );
        } else {
          toast.success(
            `上游探测完成：已是最新 ${summary.current} · 落后 ${summary.behind} · 领先 ${summary.ahead} · 未知 ${summary.unknown}`,
          );
        }
      } else {
        toast.error(envelope.errors[0]?.message ?? "上游探测失败");
      }
      void queryClient.invalidateQueries({ queryKey: engineKeys.all });
    },
    onError: (err) => {
      toast.error(`上游探测调用失败：${engineErrorMessage(err)}`);
    },
  });
}
