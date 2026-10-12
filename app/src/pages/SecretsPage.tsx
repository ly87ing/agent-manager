import { useEffect, useState } from "react";
import { Check, EyeOff, X } from "lucide-react";
import { ConfirmModal } from "../components/ConfirmModal";
import { ErrorPanel } from "../components/ErrorPanel";
import { PageError, PageHeader, PageLoading } from "../components/PageState";
import { SecretValueDialog } from "../components/SecretValueDialog";
import type { StatusData } from "../lib/contract";
import type { SecretsMcpEnvObserved } from "../lib/observed";
import { useEngineQuery } from "../lib/queries";
import { useDirectApply } from "../lib/useDirectApply";
import { cn } from "../lib/utils";
import { DEFAULT_SECRETS_PATH } from "../lib/secretSource";

export default function SecretsPage() {
  const query = useEngineQuery<StatusData>("secrets:status");
  const directApply = useDirectApply();
  const [editing, setEditing] = useState<{ variable: string; stored: boolean } | null>(null);
  const [clearing, setClearing] = useState<string | null>(null);
  const [confirmDisconnect, setConfirmDisconnect] = useState(false);
  const [sourcePath, setSourcePath] = useState("");
  const [sourceTouched, setSourceTouched] = useState(false);

  const elements = query.data?.ok ? query.data.data.elements : [];
  const mcpEnv = elements.find((element) => element.id === "secrets:mcp-env");
  const envObserved = (mcpEnv?.observed ?? {}) as SecretsMcpEnvObserved;
  const source = envObserved.source ?? {};

  useEffect(() => {
    if (!sourceTouched) setSourcePath(source.path ?? "");
  }, [source.path, sourceTouched]);

  if (query.isPending) return <PageLoading />;
  if (query.isError) return <PageError title="密钥状态读取失败" error={query.error} />;

  const variables = envObserved.variables ?? [];
  const unresolved = envObserved.unresolved ?? variables.filter((item) => item.resolved === false).length;
  const servers = Array.from(new Set(variables.map((item) => item.server ?? "?")));
  const sourceReady = source.ready === true;
  const currentSourcePath = source.path ?? "";
  const sourceChanged = sourcePath.trim() !== currentSourcePath;

  const saveSource = () => {
    const path = sourcePath.trim();
    if (!path && source.configured) {
      setConfirmDisconnect(true);
      return;
    }
    void directApply
      .apply({
        command: "secrets:source",
        label: "使用外部密钥文件",
        args: ["--path", path],
      })
      .then((envelope) => {
        if (!envelope?.ok) return;
        setSourceTouched(false);
        void query.refetch();
      });
  };

  /** 路径处的文件还不存在时新建一个（0600）；已存在就照常验证并使用，不改它。 */
  const createSource = () => {
    const path = sourcePath.trim() || DEFAULT_SECRETS_PATH;
    void directApply
      .apply({
        command: "secrets:source",
        label: `新建密钥文件 ${path}`,
        args: ["--path", path, "--create"],
      })
      .then((envelope) => {
        if (!envelope?.ok) return;
        setSourceTouched(false);
        void query.refetch();
      });
  };

  const disconnectSource = () => {
    setConfirmDisconnect(false);
    void directApply
      .apply({
        command: "secrets:source",
        label: "停止使用外部密钥文件",
        args: ["--path", ""],
      })
      .then((envelope) => {
        if (!envelope?.ok) return;
        setSourceTouched(false);
        void query.refetch();
      });
  };

  return (
    <div className="space-y-4">
      <PageHeader title="MCP 密钥" description="先选择一个外部密钥文件；本应用只保存路径，不把密钥写进数据库。" />
      {query.data.errors.length > 0 && <ErrorPanel title="密钥状态读取失败" errors={query.data.errors} />}

      <section data-testid="secrets-source-card" className="rounded-lg border bg-card p-5 shadow-sm">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div>
            <h3 className="text-sm font-semibold text-foreground">密钥保存位置</h3>
            <p className="mt-1 text-xs text-muted-foreground">
              手动填写的密钥保存在此文件，也可选择你自己的私有 secrets 仓库中的 .env 文件。无需访问作者的仓库。
            </p>
          </div>
          <span
            data-testid="secrets-source-status"
            className={cn(
              "rounded-full px-2.5 py-1 text-xs font-medium",
              sourceReady
                ? "bg-success/15 text-success"
                : source.configured
                  ? "bg-destructive/15 text-destructive"
                  : "bg-warning/15 text-warning",
            )}
          >
            {sourceReady ? "已连接" : source.configured ? "文件不可用" : "尚未选择"}
          </span>
        </div>

        <label className="mt-4 block text-xs font-medium text-foreground" htmlFor="secrets-source-path">
          外部 .env 文件
        </label>
        <div className="mt-1 flex flex-col gap-2 sm:flex-row">
          <input
            id="secrets-source-path"
            data-testid="secrets-source-path"
            value={sourcePath}
            onChange={(event) => {
              setSourcePath(event.target.value);
              setSourceTouched(true);
            }}
            placeholder={`例如：${DEFAULT_SECRETS_PATH}`}
            spellCheck={false}
            className="min-w-0 flex-1 rounded border border-input px-3 py-2 font-mono text-xs"
          />
          <button
            type="button"
            data-testid="secrets-source-save"
            disabled={directApply.executing || !sourceChanged}
            onClick={saveSource}
            className="rounded bg-primary px-3 py-2 text-xs font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
          >
            {directApply.executing ? "正在验证…" : sourcePath.trim() ? "验证并使用" : "停止使用"}
          </button>
          {!source.configured && (
            <button
              type="button"
              data-testid="secrets-source-create"
              disabled={directApply.executing}
              onClick={createSource}
              title="路径处还没有文件就新建一个空的（仅本人可读）；已有文件不改动"
              className="rounded border border-primary/40 px-3 py-2 text-xs font-medium text-primary hover:bg-primary/10 disabled:opacity-50"
            >
              新建一个
            </button>
          )}
        </div>
        <p className="mt-2 text-[11px] leading-5 text-muted-foreground">
          一行一个 KEY=VALUE；macOS / Linux 文件权限须为 600。留空新建到 {DEFAULT_SECRETS_PATH}。应用不自动同步 Git，配置备份不包含密钥。
        </p>
        {source.error && !sourceChanged && (
          <p role="alert" data-testid="secrets-source-error" className="mt-2 rounded border border-destructive/30 bg-destructive/10 px-3 py-2 text-xs text-destructive">
            {source.error}
          </p>
        )}
      </section>

      <div className="flex items-start gap-2 rounded-lg border bg-muted px-4 py-2.5 text-xs text-muted-foreground">
        <EyeOff className="mt-0.5 h-4 w-4 shrink-0" aria-hidden />
        <p>页面只显示变量名和来源，不读取回显密钥。这里只允许修改 MCP 专用变量；账号凭据与其他变量只读。</p>
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
                <th className="py-2 pr-4 font-medium">当前来源</th>
                <th className="py-2 font-medium">操作</th>
              </tr>
            </thead>
            <tbody>
              {servers.map((server) => {
                const rows = variables.filter((item) => (item.server ?? "?") === server);
                return rows.map((item, index) => {
                  const variable = item.placeholder ?? item.env_key ?? "";
                  return (
                    <tr key={`${server}-${item.env_key}-${item.kind ?? "env"}`} className="border-b last:border-0">
                      {index === 0 && (
                        <td rowSpan={rows.length} className="py-2 pr-4 align-top font-mono text-xs text-muted-foreground">
                          {server}
                        </td>
                      )}
                      <td className="py-2 pr-4 font-mono text-xs text-foreground">
                        {variable}
                        {item.kind === "header" && (
                          <span className="ml-1.5 rounded border border-info/30 bg-info/10 px-1 py-0.5 text-[10px] text-info">header</span>
                        )}
                      </td>
                      <td className="py-2 pr-4">
                        {item.resolved ? (
                          <span className="inline-flex items-center gap-1 text-xs text-success">
                            <Check className="h-3.5 w-3.5" aria-hidden />
                            {item.stored ? "外部文件" : "进程环境"}
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
                          data-testid={`fill-secret-${variable}`}
                          disabled={directApply.executing || !sourceReady}
                          title={sourceReady ? `写入 ${currentSourcePath}` : "先在上方选择可用的外部密钥文件"}
                          onClick={() => setEditing({ variable, stored: item.stored === true })}
                          className="rounded border px-2.5 py-1 text-xs text-muted-foreground hover:bg-accent disabled:cursor-not-allowed disabled:opacity-50"
                        >
                          {item.stored ? "修改" : "写入外部文件"}
                        </button>
                      </td>
                    </tr>
                  );
                });
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
          onRequestClear={() => {
            setClearing(editing.variable);
            setEditing(null);
          }}
          onSubmit={(value) => {
            void directApply
              .apply({
                command: "secrets:set",
                label: `写入外部密钥 ${editing.variable}`,
                args: ["--var", editing.variable, "--value", value],
              })
              .then((envelope) => {
                if (!envelope?.ok) return;
                setEditing(null);
                void query.refetch();
              });
          }}
        />
      )}

      {clearing && (
        <ConfirmModal
          title={`删除 ${clearing} 的取值`}
          description={`只从外部密钥文件 ${currentSourcePath} 删除这一项；MCP 条目里的变量引用不变。删除后，相关受控操作会因缺少密钥而停止。`}
          risk="write-local"
          riskLabel="删除外部密钥"
          confirmLabel="确认删除"
          executing={directApply.executing}
          onClose={() => setClearing(null)}
          onConfirm={() => {
            const variable = clearing;
            setClearing(null);
            void directApply
              .apply({
                command: "secrets:set",
                label: `删除外部密钥 ${variable}`,
                args: ["--var", variable, "--value", ""],
              })
              .then((envelope) => envelope?.ok && void query.refetch());
          }}
        />
      )}

      {confirmDisconnect && (
        <ConfirmModal
          title="停止使用外部密钥文件"
          description="只会取消本应用对该文件的引用，不会删除或修改文件。依赖此文件的 MCP 凭据解析将停止，直到重新选择可用文件。"
          risk="write-local"
          confirmLabel="确认停止使用"
          executing={directApply.executing}
          onClose={() => setConfirmDisconnect(false)}
          onConfirm={disconnectSource}
        />
      )}
    </div>
  );
}
