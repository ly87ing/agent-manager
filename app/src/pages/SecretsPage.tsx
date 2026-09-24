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

export default function SecretsPage() {
  const query = useEngineQuery<StatusData>("secrets:status");
  const directApply = useDirectApply();
  const [editing, setEditing] = useState<{ variable: string; stored: boolean } | null>(null);
  const [clearing, setClearing] = useState<string | null>(null);
  const [confirmMigration, setConfirmMigration] = useState(false);
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
  const legacyCount = envObserved.legacy_database_values ?? 0;
  const providerKeys = envObserved.provider_keys ?? [];
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
      <PageHeader title="密钥" description="先选择一个外部密钥文件；本应用只保存路径，不把密钥写进数据库。" />
      {query.data.errors.length > 0 && <ErrorPanel title="密钥状态读取失败" errors={query.data.errors} />}

      <section data-testid="secrets-source-card" className="rounded-lg border bg-card p-5 shadow-sm">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div>
            <h3 className="text-sm font-semibold text-foreground">密钥保存位置</h3>
            <p className="mt-1 text-xs text-muted-foreground">
              选择私有仓库里的 `.env` 文件。以后在本应用填写或接管的密钥都只写这里。
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
            placeholder="例如：~/agent-manager/secrets/mcp.env"
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
        </div>
        <p className="mt-2 text-[11px] leading-5 text-muted-foreground">
          文件必须已存在、权限为 600，并使用 `KEY=VALUE` 格式。不会自动克隆仓库，也不会把文件内容放进配置备份。
        </p>
        {source.error && !sourceChanged && (
          <p role="alert" data-testid="secrets-source-error" className="mt-2 rounded border border-destructive/30 bg-destructive/10 px-3 py-2 text-xs text-destructive">
            {source.error}
          </p>
        )}
      </section>

      {legacyCount > 0 && (
        <section data-testid="legacy-secrets-warning" className="rounded-lg border border-warning/40 bg-warning/10 p-4">
          <h3 className="text-sm font-semibold text-warning">旧数据库里还有 {legacyCount} 个明文密钥</h3>
          <p className="mt-1 text-xs leading-5 text-muted-foreground">
            当前版本不会再读取这些旧值。先确认上面的外部文件可用，再把它们迁移过去并从数据库删除；同名但取值不同会拒绝覆盖。
          </p>
          <button
            type="button"
            data-testid="migrate-legacy-secrets"
            disabled={!sourceReady || directApply.executing}
            title={sourceReady ? "迁移后删除旧数据库表" : "先选择可用的外部密钥文件"}
            onClick={() => setConfirmMigration(true)}
            className="mt-3 rounded border border-warning/40 bg-card px-3 py-1.5 text-xs font-medium text-warning hover:bg-warning/15 disabled:opacity-50"
          >
            迁移并清除旧值
          </button>
        </section>
      )}

      <div className="flex items-start gap-2 rounded-lg border bg-muted px-4 py-2.5 text-xs text-muted-foreground">
        <EyeOff className="mt-0.5 h-4 w-4 shrink-0" aria-hidden />
        <p>页面只显示变量名和来源，不读取回显密钥。外部文件优先，进程环境仅作为只读回退。</p>
      </div>

      {mcpEnv && providerKeys.length > 0 && (
        <section data-testid="secrets-provider-keys" className="rounded-lg border bg-card p-5 shadow-sm">
          <h3 className="text-sm font-semibold text-foreground">接入与独立环境用到的密钥</h3>
          <p className="mt-1 text-xs text-muted-foreground">
            独立环境写配置时把取值写进了自己的配置文件：改了取值后，到下面列出的独立环境卡片上重新「写入配置」才生效。
          </p>
          <table className="mt-3 w-full text-left text-sm">
            <thead>
              <tr className="border-b text-xs text-muted-foreground">
                <th className="py-2 pr-4 font-medium">变量名</th>
                <th className="py-2 pr-4 font-medium">用在</th>
                <th className="py-2 pr-4 font-medium">当前来源</th>
                <th className="py-2 font-medium">操作</th>
              </tr>
            </thead>
            <tbody>
              {providerKeys.map((item) => (
                <tr key={item.variable} data-testid={`provider-key-${item.variable}`} className="border-b last:border-0">
                  <td className="py-2 pr-4 font-mono text-xs text-foreground">{item.variable}</td>
                  <td className="py-2 pr-4 text-xs text-muted-foreground">
                    {item.used_by
                      .map((use) => (use.kind === "connection" ? `接入 ${use.label}（${use.agent}）` : `独立环境 ${use.label}（${use.id}）`))
                      .join("、")}
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
                      data-testid={`fill-provider-key-${item.variable}`}
                      disabled={directApply.executing || !sourceReady}
                      title={sourceReady ? `写入 ${currentSourcePath}` : "先在上方选择可用的外部密钥文件"}
                      onClick={() => setEditing({ variable: item.variable, stored: item.stored })}
                      className="rounded border px-2.5 py-1 text-xs text-muted-foreground hover:bg-accent disabled:cursor-not-allowed disabled:opacity-50"
                    >
                      {item.stored ? "修改" : "写入外部文件"}
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </section>
      )}

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
          description={`只从外部密钥文件 ${currentSourcePath} 删除这一项；接入配置与 MCP 条目里的变量引用不变。删除后如果进程环境也没有该变量，相关配置将无法同步。`}
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

      {confirmMigration && (
        <ConfirmModal
          title="迁移旧数据库密钥"
          description={`先把 ${legacyCount} 个旧取值写入 ${currentSourcePath}，全部成功后才删除数据库里的旧密钥表。同名不同值会拒绝并保持数据库不变。`}
          risk="install-remove"
          riskLabel="迁移并删除旧值"
          requireText="迁移"
          confirmLabel="迁移并清除"
          executing={directApply.executing}
          onClose={() => setConfirmMigration(false)}
          onConfirm={() => {
            setConfirmMigration(false);
            void directApply
              .apply({ command: "secrets:migrate-legacy", label: "迁移旧数据库密钥" })
              .then((envelope) => envelope?.ok && void query.refetch());
          }}
        />
      )}

      {confirmDisconnect && (
        <ConfirmModal
          title="停止使用外部密钥文件"
          description="只会取消本应用对该文件的引用，不会删除或修改文件。之后只能读取进程环境，应用内填写和接管密钥将不可用。"
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
