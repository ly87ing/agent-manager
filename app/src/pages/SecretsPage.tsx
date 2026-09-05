import { Check, EyeOff, X } from "lucide-react";
import { ErrorPanel } from "../components/ErrorPanel";
import { PageError, PageHeader, PageLoading } from "../components/PageState";
import { StateBadge } from "../components/StateBadge";
import type { StatusData } from "../lib/contract";
import type { SecretsMcpEnvObserved, SecretsRepoObserved } from "../lib/observed";
import { useEngineQuery } from "../lib/queries";
import { cn } from "../lib/utils";

export default function SecretsPage() {
  const query = useEngineQuery<StatusData>("secrets:status");

  if (query.isPending) return <PageLoading />;
  if (query.isError) return <PageError title="secrets:status 调用失败" error={query.error} />;

  const elements = query.data.data.elements;
  const repo = elements.find((element) => element.id === "secrets:repo");
  const mcpEnv = elements.find((element) => element.id === "secrets:mcp-env");
  const repoObserved = (repo?.observed ?? {}) as SecretsRepoObserved;
  const envObserved = (mcpEnv?.observed ?? {}) as SecretsMcpEnvObserved;
  const variables = envObserved.variables ?? [];
  const unresolved = envObserved.unresolved ?? variables.filter((item) => item.resolved === false).length;
  const servers = Array.from(new Set(variables.map((item) => item.server ?? "?")));

  return (
    <div className="space-y-4">
      <PageHeader title="Secrets" description="secrets 仓库状态与 MCP env 占位符解析矩阵（只读）。" />
      {query.data.errors.length > 0 && <ErrorPanel title="secrets:status 错误" errors={query.data.errors} />}

      <div className="flex items-start gap-2 rounded-lg border border-slate-200 bg-slate-100 px-4 py-2.5 text-xs text-slate-600">
        <EyeOff className="mt-0.5 h-4 w-4 shrink-0 text-slate-500" aria-hidden />
        <p>
          本页只展示占位符解析状态，变量值永不显示。secrets 仓库由独立 git 仓库管理，编辑请走该仓库本身的流程。
        </p>
      </div>

      {repo && (
        <section data-testid="secrets-repo-card" className="rounded-lg border border-slate-200 bg-white p-5 shadow-sm">
          <div className="flex items-center justify-between">
            <h3 className="text-sm font-semibold text-slate-700">secrets 仓库</h3>
            <StateBadge state={repo.state} />
          </div>
          <dl className="mt-3 space-y-1.5 text-xs text-slate-600">
            <div className="flex gap-2">
              <dt className="text-slate-400">路径</dt>
              <dd className="font-mono break-all">{repoObserved.path ?? "—"}</dd>
            </div>
            <div className="flex gap-2">
              <dt className="text-slate-400">存在</dt>
              <dd>{repoObserved.exists ? "是" : "否"}</dd>
            </div>
            <div className="flex gap-2">
              <dt className="text-slate-400">git 仓库</dt>
              <dd>{repoObserved.is_git ? "是" : "否"}</dd>
            </div>
            <div className="flex gap-2">
              <dt className="text-slate-400">env 文件</dt>
              <dd className="flex flex-wrap gap-1">
                {(repoObserved.env_files ?? []).map((file) => (
                  <code key={file} className="rounded bg-slate-100 px-1.5 py-0.5 font-mono">
                    {file}
                  </code>
                ))}
              </dd>
            </div>
          </dl>
        </section>
      )}

      {mcpEnv && (
        <section data-testid="secrets-env-matrix" className="rounded-lg border border-slate-200 bg-white p-5 shadow-sm">
          <div className="mb-3 flex items-center justify-between">
            <h3 className="text-sm font-semibold text-slate-700">MCP env 占位符解析矩阵</h3>
            <span
              data-testid="env-unresolved-summary"
              className={cn(
                "rounded-full px-2.5 py-0.5 text-xs font-medium",
                unresolved > 0 ? "bg-red-100 text-red-700" : "bg-emerald-100 text-emerald-700",
              )}
            >
              {unresolved > 0 ? `${unresolved} 个占位符未解析` : `全部 ${variables.length} 个占位符已解析`}
            </span>
          </div>
          <table className="w-full text-left text-sm">
            <thead>
              <tr className="border-b border-slate-200 text-xs text-slate-500">
                <th className="py-2 pr-4 font-medium">Server</th>
                <th className="py-2 pr-4 font-medium">变量（占位符名）</th>
                <th className="py-2 font-medium">解析</th>
              </tr>
            </thead>
            <tbody>
              {servers.map((server) => {
                const rows = variables.filter((item) => (item.server ?? "?") === server);
                return rows.map((item, index) => (
                  <tr key={`${server}-${item.env_key}`} className="border-b border-slate-100 last:border-0">
                    {index === 0 && (
                      <td rowSpan={rows.length} className="py-2 pr-4 align-top font-mono text-xs text-slate-600">
                        {server}
                      </td>
                    )}
                    <td className="py-2 pr-4 font-mono text-xs text-slate-700">{item.placeholder ?? item.env_key}</td>
                    <td className="py-2">
                      {item.resolved ? (
                        <span className="inline-flex items-center gap-1 text-xs text-emerald-600">
                          <Check className="h-3.5 w-3.5" aria-hidden /> 已解析
                        </span>
                      ) : (
                        <span className="inline-flex items-center gap-1 text-xs text-red-600">
                          <X className="h-3.5 w-3.5" aria-hidden /> 未解析
                        </span>
                      )}
                    </td>
                  </tr>
                ));
              })}
            </tbody>
          </table>
          {variables.length === 0 && <p className="py-4 text-center text-sm text-slate-400">无 env 占位符</p>}
        </section>
      )}
    </div>
  );
}
