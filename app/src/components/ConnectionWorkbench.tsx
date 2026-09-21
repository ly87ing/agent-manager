import { ArrowDown, ArrowUp, Check, Download, Loader2, Plus, Settings2, Upload } from "lucide-react";
import { AgentUsageBar } from "./AgentUsageBar";
import { AuthCommandActions } from "./AuthCommandActions";
import type { ConnectionKind } from "./ConnectionFormDialog";
import type { EngineElement } from "../lib/contract";
import type { AgentCurrentConfigStatus, AgentsObserved, AgentsSpec } from "../lib/observed";
import {
  parseInitialToFormValues,
  runtimeSettingsOf,
  type ConnectionEntry,
} from "../lib/agentDefinitionForm";
import { agentManagementOf, connectionKindOf } from "../lib/agentManagement";
import { cn } from "../lib/utils";

export interface ConnectionCheckResult {
  ok: boolean;
  detail: string;
  latencyMs: number;
  pending?: boolean;
}

function displayNameOf(element: EngineElement): string {
  return (element.spec as AgentsSpec).display_name ?? element.id.replace(/^agents:/, "");
}

function runtimeSummaryValue(value: unknown, field: NonNullable<ReturnType<typeof agentManagementOf>>["runtime_fields"][number]): string {
  const summary = field.summary;
  if (!summary) return "";
  if (field.type === "boolean") return value === true ? summary.enabled ?? "开启" : summary.default;
  if (value === undefined || value === null || value === "") return summary.default;
  return summary.values?.[String(value)] ?? String(value);
}

function CurrentConfigCard({ status }: { status: AgentCurrentConfigStatus }) {
  const config = status.config;
  if (status.state === "missing") {
    return (
      <div data-testid="agent-current-config" className="rounded-lg border border-dashed px-3 py-2 text-sm text-muted-foreground">
        本机尚无 <code>{status.file}</code>，当前使用客户端默认配置。
      </div>
    );
  }
  if (status.state !== "ready" || !config) {
    return (
      <div data-testid="agent-current-config" className="rounded-lg border border-warning/30 bg-warning/10 px-3 py-2 text-sm text-warning">
        无法安全读取本机 <code>{status.file}</code>（{status.state}），未展示可能不可靠的配置。
      </div>
    );
  }
  const credential = config.credential;
  return (
    <div data-testid="agent-current-config" className="rounded-lg border bg-muted/30 px-3 py-3">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <p className="text-sm font-medium text-foreground">本机当前配置</p>
        <span className="font-mono text-xs text-muted-foreground">{status.file} · 只读</span>
      </div>
      <dl className="mt-2 grid grid-cols-1 gap-x-4 gap-y-1 text-sm sm:grid-cols-2">
        <div><dt className="inline text-muted-foreground">Provider：</dt><dd className="inline font-mono">{config.provider || "客户端默认"}</dd></div>
        <div><dt className="inline text-muted-foreground">模型：</dt><dd className="inline font-mono">{config.model || "客户端默认"}</dd></div>
        <div className="sm:col-span-2"><dt className="inline text-muted-foreground">地址：</dt><dd className="inline break-all font-mono">{config.base_url || "客户端默认"}</dd></div>
        <div className="sm:col-span-2"><dt className="inline text-muted-foreground">凭据：</dt><dd className="inline">{credential?.configured ? `已配置${credential.variable ? `（变量 ${credential.variable}）` : "（值已隐藏）"}` : "未在该配置文件中设置"}</dd></div>
      </dl>
      <p className="mt-2 text-xs text-muted-foreground">这是客户端文件的实际观察值，不会自动写入数据库或接管。</p>
    </div>
  );
}

export function ConnectionWorkbench({
  agents,
  selectedAgentId,
  busyTarget,
  checkResults,
  onSelectAgent,
  onAdd,
  onImport,
  onSwitch,
  onCheck,
  onEdit,
  onDuplicate,
  onExport,
  onRemove,
  onMove,
  onRuntimeSettings,
}: {
  agents: EngineElement[];
  selectedAgentId: string | null;
  busyTarget: string | null;
  checkResults: Record<string, ConnectionCheckResult>;
  onSelectAgent: (agentId: string) => void;
  onAdd: (element: EngineElement) => void;
  onImport: (element: EngineElement) => void;
  onSwitch: (element: EngineElement, connection: string) => void;
  onCheck: (element: EngineElement, connection: string) => void;
  onEdit: (element: EngineElement, connection: ConnectionEntry) => void;
  onDuplicate: (element: EngineElement, connection: ConnectionEntry) => void;
  onExport: (element: EngineElement, connection: ConnectionEntry) => void;
  onRemove: (element: EngineElement, connection: ConnectionEntry) => void;
  onMove: (element: EngineElement, connection: ConnectionEntry, direction: "up" | "down") => void;
  onRuntimeSettings: (element: EngineElement) => void;
}) {
  const selected = agents.find((element) => element.id === selectedAgentId) ?? agents[0];
  if (!selected) return null;

  const agentId = selected.id.replace(/^agents:/, "");
  const name = displayNameOf(selected);
  const form = parseInitialToFormValues(agentId, selected.spec as Record<string, unknown>);
  const active = form.activeConnection;
  const activeEntry = form.connections.find((item) => item.name === active);
  const observed = selected.observed as AgentsObserved;
  const management = agentManagementOf(selected);
  const kind = connectionKindOf(selected);
  const runtime = runtimeSettingsOf(selected.spec as Record<string, unknown>);
  const auth = observed.auth;
  const currentConfig = observed.current_config;

  return (
    <section data-testid="connection-workbench" className="rounded-xl border bg-card p-4 shadow-sm sm:p-5">
      <div className="flex flex-col gap-4">
        <div className="flex flex-col justify-between gap-3 lg:flex-row lg:items-start">
          <div className="min-w-0">
            <p className="text-xs font-medium text-primary">日常使用</p>
            <h2 className="mt-1 text-lg font-semibold text-foreground">{name} 接入工作台</h2>
            <p className="mt-1 text-sm text-muted-foreground">
              Agent Manager 已纳管选择：
              <span data-testid="active-connection-summary" className="ml-1 font-medium text-foreground">
                {activeEntry?.label || activeEntry?.name || "助手默认账号"}
              </span>
              。切换会同时更新本机记录和助手配置文件，下次启动生效。
            </p>
          </div>
          <div className="flex shrink-0 flex-wrap items-center gap-2">
            <button
              type="button"
              data-testid="import-connection-button"
              disabled={!kind}
              onClick={() => onImport(selected)}
              className="inline-flex items-center justify-center gap-1.5 rounded-md border px-3 py-2 text-sm text-muted-foreground hover:bg-accent disabled:opacity-50"
            >
              <Upload className="h-4 w-4" aria-hidden /> 导入接入
            </button>
            <button
              type="button"
              data-testid="agent-runtime-button"
              disabled={!kind}
              onClick={() => onRuntimeSettings(selected)}
              className="inline-flex items-center justify-center gap-1.5 rounded-md border px-3 py-2 text-sm text-muted-foreground hover:bg-accent disabled:cursor-not-allowed disabled:opacity-50"
            >
              <Settings2 className="h-4 w-4" aria-hidden /> 运行设置
            </button>
            <button
              type="button"
              data-testid="add-connection-button"
              disabled={!kind}
              onClick={() => onAdd(selected)}
              className="inline-flex items-center justify-center gap-1.5 rounded-md bg-primary px-4 py-2 text-sm text-primary-foreground hover:bg-primary/90 disabled:cursor-not-allowed disabled:opacity-50"
            >
              <Plus className="h-4 w-4" aria-hidden /> 添加接入
            </button>
          </div>
        </div>

        {management && (
          <div className="flex flex-wrap gap-2 text-xs text-muted-foreground" aria-label="已纳管运行设置">
            {management.runtime_fields.filter((field) => field.summary).map((field) => (
              <span
                key={field.key}
                className={cn(
                  "rounded-full border px-2.5 py-1",
                  field.danger && runtime[field.key] === true
                    ? "border-destructive/30 bg-destructive/10 text-destructive"
                    : "bg-muted/40",
                )}
              >
                {field.summary!.label}：{runtimeSummaryValue(runtime[field.key], field)}
              </span>
            ))}
          </div>
        )}

        <div aria-label="选择助手" className="flex flex-wrap gap-2" role="group">
          {agents.map((element) => {
            const id = element.id.replace(/^agents:/, "");
            const chosen = element.id === selected.id;
            return (
              <button
                key={element.id}
                type="button"
                data-testid={`connection-agent-${id}`}
                aria-pressed={chosen}
                onClick={() => onSelectAgent(element.id)}
                className={cn(
                  "rounded-full border px-3 py-1.5 text-sm transition-colors",
                  chosen
                    ? "border-primary bg-primary/10 font-medium text-primary"
                    : "text-muted-foreground hover:bg-accent",
                )}
              >
                {displayNameOf(element)}
              </button>
            );
          })}
        </div>

        {observed.usage && (
          <div className="rounded-lg border bg-muted/40 px-3 py-2">
            <AgentUsageBar usage={observed.usage} testId={`workbench-usage-${selected.id}`} />
          </div>
        )}

        {auth && (
          <div
            data-testid={`agent-auth-status-${agentId}`}
            className="flex flex-wrap items-center justify-between gap-2 rounded-lg border bg-muted/30 px-3 py-2 text-sm"
          >
            <span className="text-muted-foreground">
              官方登录：
              <strong className="ml-1 font-medium text-foreground">
                {auth.state === "ready"
                  ? `已识别${auth.mode ? `（${auth.mode}）` : ""}`
                  : auth.state === "missing"
                    ? kind === "claude"
                      ? "未发现凭据文件（可能保存在系统钥匙串）"
                      : "未登录"
                    : "认证文件需要处理"}
              </strong>
            </span>
            <div className="flex flex-wrap items-center gap-2">
              <span className="font-mono text-xs text-muted-foreground">{auth.file}</span>
              {kind && (
                <AuthCommandActions
                  kind={kind}
                  bin={(selected.spec as AgentsSpec).bin ?? (kind === "codex" ? "codex" : "claude")}
                  testIdPrefix={`agent-${agentId}`}
                />
              )}
            </div>
          </div>
        )}

        {!kind && (
          <p className="rounded-lg border border-warning/30 bg-warning/10 px-3 py-2 text-sm text-warning">
            这个助手暂不支持在应用内切换接入或写入运行设置；已有配置仅只读展示。
          </p>
        )}

        {kind === "codex" && (
          <p
            data-testid="codex-auth-boundary"
            className="rounded-lg border border-primary/20 bg-primary/5 px-3 py-2 text-sm text-muted-foreground"
          >
            切换接入只会更新 <code>config.toml</code>。第三方凭据仅用于模型请求；
            <code>auth.json</code> 中的 Codex 官方登录会保留，只有单独确认的认证操作才能修改。
          </p>
        )}

        {currentConfig && <CurrentConfigCard status={currentConfig} />}

        <div className="grid grid-cols-1 gap-3 lg:grid-cols-2">
          <ConnectionCard
            agentId={agentId}
            kind={kind}
            connection={null}
            active={!active}
            busy={busyTarget === `${agentId}/default`}
            disabled={busyTarget !== null}
            onActivate={kind ? () => onSwitch(selected, "") : undefined}
          />
          {form.connections.map((connection, index) => {
            const key = `${agentId}/${connection.name}`;
            return (
              <ConnectionCard
                key={connection.name}
                agentId={agentId}
                kind={kind}
                connection={connection}
                active={active === connection.name}
                busy={busyTarget === key}
                disabled={busyTarget !== null}
                check={checkResults[key]}
                onActivate={kind ? () => onSwitch(selected, connection.name) : undefined}
                onCheck={kind ? () => onCheck(selected, connection.name) : undefined}
                onEdit={kind ? () => onEdit(selected, connection) : undefined}
                onDuplicate={kind ? () => onDuplicate(selected, connection) : undefined}
                onExport={kind ? () => onExport(selected, connection) : undefined}
                onRemove={kind ? () => onRemove(selected, connection) : undefined}
                onMoveUp={kind && index > 0 ? () => onMove(selected, connection, "up") : undefined}
                onMoveDown={kind && index < form.connections.length - 1 ? () => onMove(selected, connection, "down") : undefined}
              />
            );
          })}
        </div>

        {form.connections.length === 0 && kind && (
          <p className="text-sm text-muted-foreground">
            目前只有助手默认账号。点“添加接入”即可从官方厂商预设开始，不需要先理解安装命令。
          </p>
        )}
      </div>
    </section>
  );
}

function ConnectionCard({
  agentId,
  kind,
  connection,
  active,
  busy,
  disabled,
  check,
  onActivate,
  onCheck,
  onEdit,
  onDuplicate,
  onExport,
  onRemove,
  onMoveUp,
  onMoveDown,
}: {
  agentId: string;
  kind: ConnectionKind | null;
  connection: ConnectionEntry | null;
  active: boolean;
  busy: boolean;
  disabled: boolean;
  check?: ConnectionCheckResult;
  onActivate?: () => void;
  onCheck?: () => void;
  onEdit?: () => void;
  onDuplicate?: () => void;
  onExport?: () => void;
  onRemove?: () => void;
  onMoveUp?: () => void;
  onMoveDown?: () => void;
}) {
  const id = connection?.name ?? "default";
  return (
    <article
      data-testid={`connection-card-${agentId}-${id}`}
      className={cn(
        "rounded-lg border p-4",
        active ? "border-primary/50 bg-primary/5 ring-1 ring-primary/20" : "bg-background",
      )}
    >
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-2">
            <h3 className="font-medium text-foreground">
              {connection ? connection.label || connection.name : "助手默认账号"}
            </h3>
            {active && (
              <span className="inline-flex items-center gap-1 rounded-full bg-success/15 px-2 py-0.5 text-xs font-medium text-success">
                <Check className="h-3 w-3" aria-hidden /> 当前接入
              </span>
            )}
          </div>
          <p className="mt-1 break-all font-mono text-xs text-muted-foreground">
            {connection?.baseUrl || "使用助手自己的地址与登录状态"}
          </p>
        </div>
        {!active && onActivate && (
          <button
            type="button"
            data-testid={`activate-connection-${agentId}-${id}`}
            disabled={disabled}
            onClick={onActivate}
            className="inline-flex shrink-0 items-center gap-1 rounded-md border border-primary/30 bg-primary/10 px-3 py-1.5 text-xs font-medium text-primary hover:bg-primary/15 disabled:opacity-50"
          >
            {busy && <Loader2 className="h-3.5 w-3.5 animate-spin" aria-hidden />}
            {busy ? "正在启用…" : "启用"}
          </button>
        )}
      </div>

      {connection && (
        <dl className="mt-3 grid grid-cols-1 gap-x-4 gap-y-3 rounded-md border bg-muted/20 p-3 text-xs sm:grid-cols-2">
          <div>
            <dt className="text-muted-foreground">模型 <code>model</code></dt>
            <dd className="mt-0.5 break-all font-mono text-foreground">{connection.model || "未指定"}</dd>
          </div>
          <div>
            <dt className="text-muted-foreground">凭据变量 <code>token_var</code></dt>
            <dd className="mt-0.5 break-all font-mono text-foreground">{connection.tokenVar || "未指定"}</dd>
          </div>
          {kind === "claude" && (
            <div>
              <dt className="text-muted-foreground">认证方式 <code>auth</code></dt>
              <dd className="mt-0.5 break-all font-mono text-foreground">
                {connection.auth === "api_key" ? "api_key (x-api-key)" : "token (Authorization: Bearer)"}
              </dd>
            </div>
          )}
          <div>
            <dt className="text-muted-foreground">请求超时 <code>timeout_ms</code></dt>
            <dd className="mt-0.5 break-all font-mono text-foreground">
              {connection.timeoutMs ? `${connection.timeoutMs} ms` : "未指定（使用客户端默认）"}
            </dd>
          </div>
          {kind === "claude" && (
            <>
              <div>
                <dt className="text-muted-foreground">小任务模型 <code>small_fast_model</code></dt>
                <dd className="mt-0.5 break-all font-mono text-foreground">
                  {connection.smallFastModel || "未指定（使用客户端默认）"}
                </dd>
              </div>
              <div>
                <dt className="text-muted-foreground">Opus 别名 <code>model_aliases.opus</code></dt>
                <dd className="mt-0.5 break-all font-mono text-foreground">{connection.aliasOpus || "未指定"}</dd>
              </div>
              <div>
                <dt className="text-muted-foreground">Sonnet 别名 <code>model_aliases.sonnet</code></dt>
                <dd className="mt-0.5 break-all font-mono text-foreground">{connection.aliasSonnet || "未指定"}</dd>
              </div>
              <div>
                <dt className="text-muted-foreground">Haiku 别名 <code>model_aliases.haiku</code></dt>
                <dd className="mt-0.5 break-all font-mono text-foreground">{connection.aliasHaiku || "未指定"}</dd>
              </div>
            </>
          )}
        </dl>
      )}

      {connection && (onCheck || onEdit || onDuplicate || onExport || onRemove || onMoveUp || onMoveDown) && (
        <div className="mt-3 flex flex-wrap items-center gap-2 border-t pt-3">
          {onCheck && (
            <button
              type="button"
              data-testid={`check-connection-${agentId}-${id}`}
              disabled={check?.pending}
              onClick={onCheck}
              className="rounded border px-2.5 py-1 text-xs text-muted-foreground hover:bg-accent disabled:opacity-50"
            >
              {check?.pending ? "正在测试…" : "测试连接"}
            </button>
          )}
          {check && !check.pending && (
            <p
              data-testid={`check-result-${agentId}-${id}`}
              className={cn("text-xs", check.ok ? "text-success" : "text-destructive")}
            >
              {check.detail}
              {check.ok && check.latencyMs > 0 && (
                <span className="ml-1 text-muted-foreground">{check.latencyMs}ms</span>
              )}
            </p>
          )}
          <div className="ml-auto flex items-center gap-2">
            <div className="flex items-center" aria-label="调整接入顺序">
              <button
                type="button"
                aria-label={`上移 ${connection.label || connection.name}`}
                disabled={disabled || !onMoveUp}
                onClick={onMoveUp}
                className="rounded-l border px-1.5 py-1 text-muted-foreground hover:bg-accent disabled:opacity-30"
              >
                <ArrowUp className="h-3.5 w-3.5" aria-hidden />
              </button>
              <button
                type="button"
                aria-label={`下移 ${connection.label || connection.name}`}
                disabled={disabled || !onMoveDown}
                onClick={onMoveDown}
                className="rounded-r border border-l-0 px-1.5 py-1 text-muted-foreground hover:bg-accent disabled:opacity-30"
              >
                <ArrowDown className="h-3.5 w-3.5" aria-hidden />
              </button>
            </div>
            {onEdit && (
              <button
                type="button"
                data-testid={`edit-connection-${agentId}-${id}`}
                disabled={disabled}
                onClick={onEdit}
                className="rounded border px-2.5 py-1 text-xs text-muted-foreground hover:bg-accent disabled:opacity-50"
              >
                编辑
              </button>
            )}
            {onDuplicate && (
              <button
                type="button"
                data-testid={`duplicate-connection-${agentId}-${id}`}
                disabled={disabled}
                onClick={onDuplicate}
                className="rounded border px-2.5 py-1 text-xs text-muted-foreground hover:bg-accent disabled:opacity-50"
              >
                复制
              </button>
            )}
            {onExport && (
              <button
                type="button"
                data-testid={`export-connection-${agentId}-${id}`}
                disabled={disabled}
                onClick={onExport}
                className="inline-flex items-center gap-1 rounded border px-2.5 py-1 text-xs text-muted-foreground hover:bg-accent disabled:opacity-50"
              >
                <Download className="h-3.5 w-3.5" aria-hidden /> 导出
              </button>
            )}
            {onRemove && (
              <button
                type="button"
                data-testid={`remove-connection-${agentId}-${id}`}
                disabled={disabled || active}
                title={active ? "当前接入不能删除，请先切换到其他接入或默认账号" : "删除这套接入"}
                onClick={onRemove}
                className="rounded border border-destructive/30 px-2.5 py-1 text-xs text-destructive hover:bg-destructive/10 disabled:cursor-not-allowed disabled:opacity-50"
              >
                删除
              </button>
            )}
          </div>
        </div>
      )}
    </article>
  );
}
