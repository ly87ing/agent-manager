import { PageError, PageHeader, PageLoading } from "../components/PageState";
import type { StatusData } from "../lib/contract";
import type { AgentsObserved, AgentsSpec } from "../lib/observed";
import { useEngineQuery } from "../lib/queries";

const INSTALL_SOURCE_LABEL: Record<string, string> = {
  native: "官方安装器",
  brew: "Homebrew",
  npm: "npm 全局安装",
  system: "其他系统路径",
};

/** AI CLI 客户端的只读诊断；不提供安装、升级、卸载或脚本编辑入口。 */
export default function ClientDiagnosticsPage() {
  const query = useEngineQuery<StatusData>("agents:status");
  if (query.isPending) return <PageLoading />;
  if (query.isError) return <PageError title="客户端状态读取失败" error={query.error} />;

  return (
    <div className="space-y-4">
      <PageHeader
        title="客户端诊断"
        description="只读查看 AI CLI 是否可用、当前版本与安装来源；软件生命周期交给官方安装器或包管理器。"
      />
      {query.data.data.elements.length === 0 ? (
        <p className="rounded-lg border bg-card p-5 text-sm text-muted-foreground">没有检测到可诊断的客户端。</p>
      ) : (
        <div className="grid grid-cols-1 gap-3 lg:grid-cols-2">
          {query.data.data.elements.map((element) => {
            const spec = element.spec as AgentsSpec;
            const observed = element.observed as AgentsObserved;
            const installed = observed.installed === true;
            return (
              <article key={element.id} className="rounded-lg border bg-card p-4 shadow-sm">
                <div className="flex items-start justify-between gap-3">
                  <div className="min-w-0">
                    <h2 className="font-medium text-foreground">
                      {spec.display_name ?? element.id.replace(/^agents:/, "")}
                    </h2>
                    <p className="mt-1 break-all font-mono text-xs text-muted-foreground">{spec.bin}</p>
                  </div>
                  <span className={installed
                    ? "rounded-full bg-success/15 px-2.5 py-1 text-xs font-medium text-success"
                    : "rounded-full bg-muted px-2.5 py-1 text-xs font-medium text-muted-foreground"
                  }>
                    {installed ? "已安装" : "未安装"}
                  </span>
                </div>
                <dl className="mt-4 grid grid-cols-2 gap-3 border-t pt-3 text-sm">
                  <div>
                    <dt className="text-xs text-muted-foreground">当前版本</dt>
                    <dd className="mt-1 break-all font-mono text-xs text-foreground">
                      {installed ? observed.version ?? element.freshness.current ?? "未读取到" : "—"}
                    </dd>
                  </div>
                  <div>
                    <dt className="text-xs text-muted-foreground">安装来源</dt>
                    <dd className="mt-1 break-all font-mono text-xs text-foreground">
                      {installed
                        ? INSTALL_SOURCE_LABEL[observed.install_source ?? ""] ?? observed.install_source ?? "未知"
                        : "—"}
                    </dd>
                  </div>
                </dl>
              </article>
            );
          })}
        </div>
      )}
      <p className="text-xs text-muted-foreground">
        本页不会联网检查新版本，也不会执行安装命令。需要安装或更新时，请使用对应客户端的官方说明。
      </p>
    </div>
  );
}
