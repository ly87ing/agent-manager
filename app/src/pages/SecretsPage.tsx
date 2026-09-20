import { useState } from "react";
import { Check, EyeOff, X } from "lucide-react";
import { ErrorPanel } from "../components/ErrorPanel";
import { PageError, PageHeader, PageLoading } from "../components/PageState";
import { SecretValueDialog } from "../components/SecretValueDialog";
import type { StatusData } from "../lib/contract";
import type { SecretsMcpEnvObserved } from "../lib/observed";
import { useEngineQuery } from "../lib/queries";
import { useDirectApply } from "../lib/useDirectApply";
import { cn } from "../lib/utils";

export default function SecretsPage() {
  const query = useEngineQuery<StatusData>("secrets:status");
  const directApply = useDirectApply();
  const [editing, setEditing] = useState<{ variable: string; stored: boolean } | null>(null);

  if (query.isPending) return <PageLoading />;
  if (query.isError) return <PageError title="secrets:status 调用失败" error={query.error} />;

  const elements = query.data.data.elements;
  const mcpEnv = elements.find((element) => element.id === "secrets:mcp-env");
  const envObserved = (mcpEnv?.observed ?? {}) as SecretsMcpEnvObserved;
  const variables = envObserved.variables ?? [];
  const unresolved = envObserved.unresolved ?? variables.filter((item) => item.resolved === false).length;
  const servers = Array.from(new Set(variables.map((item) => item.server ?? "?")));

  return (
    <div className="space-y-4">
      <PageHeader title="密钥" description="MCP 服务需要的取值：在这里填一次即可，界面永不回显。" />
      {query.data.errors.length > 0 && <ErrorPanel title="secrets:status 错误" errors={query.data.errors} />}

      <div className="flex items-start gap-2 rounded-lg border bg-muted px-4 py-2.5 text-xs text-muted-foreground">
        <EyeOff className="mt-0.5 h-4 w-4 shrink-0 text-muted-foreground" aria-hidden />
        <p>
          取值可以在这里直接填，保存在本机数据库里；也可以由外部注入到进程环境。两者都有时以这里填的为准。
          界面任何地方都不回显取值，它也不进操作记录与配置备份；同步时会写进对应助手的配置文件，那份文件里是明文。
          <br />
          从程序坞打开应用时拿不到你在终端里 export 的变量（那是系统的启动方式决定的），所以在这里填是最省事的做法。
        </p>
      </div>

      {mcpEnv && (
        <section data-testid="secrets-env-matrix" className="rounded-lg border bg-card p-5 shadow-sm">
          <div className="mb-3 flex items-center justify-between">
            <h3 className="text-sm font-semibold text-foreground">MCP 服务所需变量</h3>
            <span
              data-testid="env-unresolved-summary"
              className={cn(
                "rounded-full px-2.5 py-0.5 text-xs font-medium",
                unresolved > 0 ? "bg-destructive/15 text-destructive" : "bg-success/15 text-success",
              )}
            >
              {unresolved > 0 ? `${unresolved} 个变量未配置` : `全部 ${variables.length} 个变量已配置`}
            </span>
          </div>
          <table className="w-full text-left text-sm">
            <thead>
              <tr className="border-b text-xs text-muted-foreground">
                <th className="py-2 pr-4 font-medium">Server</th>
                <th className="py-2 pr-4 font-medium">变量名</th>
                <th className="py-2 pr-4 font-medium">配置状态</th>
                <th className="py-2 font-medium">操作</th>
              </tr>
            </thead>
            <tbody>
              {servers.map((server) => {
                const rows = variables.filter((item) => (item.server ?? "?") === server);
                return rows.map((item, index) => (
                  <tr key={`${server}-${item.env_key}-${item.kind ?? "env"}`} className="border-b last:border-0">
                    {index === 0 && (
                      <td rowSpan={rows.length} className="py-2 pr-4 align-top font-mono text-xs text-muted-foreground">
                        {server}
                      </td>
                    )}
                    <td className="py-2 pr-4 font-mono text-xs text-foreground">
                      {item.placeholder ?? item.env_key}
                      {item.kind === "header" && (
                        <span className="ml-1.5 rounded border border-info/30 bg-info/10 px-1 py-0.5 text-[10px] text-info">header</span>
                      )}
                    </td>
                    <td className="py-2 pr-4">
                      {item.resolved ? (
                        <span className="inline-flex items-center gap-1 text-xs text-success">
                          <Check className="h-3.5 w-3.5" aria-hidden />
                          {item.stored ? "已填写" : "已配置（来自环境）"}
                        </span>
                      ) : (
                        <span className="inline-flex items-center gap-1 text-xs text-destructive">
                          <X className="h-3.5 w-3.5" aria-hidden /> 未配置
                        </span>
                      )}
                    </td>
                    <td className="py-2">
                      <button
                        type="button"
                        data-testid={`fill-secret-${item.placeholder ?? item.env_key}`}
                        disabled={directApply.executing}
                        onClick={() =>
                          setEditing({
                            variable: item.placeholder ?? item.env_key ?? "",
                            stored: item.stored === true,
                          })
                        }
                        className="rounded border px-2.5 py-1 text-xs text-muted-foreground hover:bg-accent disabled:opacity-50"
                      >
                        {item.stored ? "改取值" : "填取值"}
                      </button>
                    </td>
                  </tr>
                ));
              })}
            </tbody>
          </table>
          {variables.length === 0 && <p className="py-4 text-center text-sm text-muted-foreground">没有需要配置的变量</p>}
        </section>
      )}

      {editing && (
        <SecretValueDialog
          variable={editing.variable}
          stored={editing.stored}
          busy={directApply.executing}
          onClose={() => setEditing(null)}
          onSubmit={(value) => {
            void directApply
              .apply({
                command: "secrets:set",
                label: value ? `保存取值 ${editing.variable}` : `清除取值 ${editing.variable}`,
                args: ["--var", editing.variable, "--value", value],
              })
              .then(() => setEditing(null));
          }}
        />
      )}
    </div>
  );
}
