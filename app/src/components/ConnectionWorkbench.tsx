import { Check, Loader2, Plus } from "lucide-react";
import { AgentUsageBar } from "./AgentUsageBar";
import type { ConnectionKind } from "./ConnectionFormDialog";
import type { EngineElement } from "../lib/contract";
import type { AgentsObserved, AgentsSpec } from "../lib/observed";
import { parseInitialToFormValues, type ConnectionEntry } from "../lib/agentDefinitionForm";
import { cn } from "../lib/utils";

export interface ConnectionCheckResult {
  ok: boolean;
  detail: string;
  latencyMs: number;
  pending?: boolean;
}

export function connectionKindOf(element: EngineElement): ConnectionKind | null {
  const bin = ((element.spec as AgentsSpec).bin ?? "")
    .trim()
    .replace(/\.exe$/i, "")
    .split("/")
    .pop();
  return bin === "claude" ? "claude" : bin === "codex" ? "codex" : null;
}

function displayNameOf(element: EngineElement): string {
  return (element.spec as AgentsSpec).display_name ?? element.id.replace(/^agents:/, "");
}

export function ConnectionWorkbench({
  agents,
  selectedAgentId,
  busyTarget,
  checkResults,
  onSelectAgent,
  onAdd,
  onSwitch,
  onCheck,
  onEdit,
  onRemove,
}: {
  agents: EngineElement[];
  selectedAgentId: string | null;
  busyTarget: string | null;
  checkResults: Record<string, ConnectionCheckResult>;
  onSelectAgent: (agentId: string) => void;
  onAdd: (element: EngineElement) => void;
  onSwitch: (element: EngineElement, connection: string) => void;
  onCheck: (element: EngineElement, connection: string) => void;
  onEdit: (element: EngineElement, connection: ConnectionEntry) => void;
  onRemove: (element: EngineElement, connection: ConnectionEntry) => void;
}) {
  const selected = agents.find((element) => element.id === selectedAgentId) ?? agents[0];
  if (!selected) return null;

  const agentId = selected.id.replace(/^agents:/, "");
  const name = displayNameOf(selected);
  const form = parseInitialToFormValues(agentId, selected.spec as Record<string, unknown>);
  const active = form.activeConnection;
  const activeEntry = form.connections.find((item) => item.name === active);
  const observed = selected.observed as AgentsObserved;
  const kind = connectionKindOf(selected);

  return (
    <section data-testid="connection-workbench" className="rounded-xl border bg-card p-4 shadow-sm sm:p-5">
      <div className="flex flex-col gap-4">
        <div className="flex flex-col justify-between gap-3 lg:flex-row lg:items-start">
          <div className="min-w-0">
            <p className="text-xs font-medium text-primary">日常使用</p>
            <h2 className="mt-1 text-lg font-semibold text-foreground">{name} 接入工作台</h2>
            <p className="mt-1 text-sm text-muted-foreground">
              当前使用：
              <span data-testid="active-connection-summary" className="ml-1 font-medium text-foreground">
                {activeEntry?.label || activeEntry?.name || "助手默认账号"}
              </span>
              。切换会同时更新本机记录和助手配置文件，下次启动生效。
            </p>
          </div>
          <button
            type="button"
            data-testid="add-connection-button"
            disabled={!kind}
            onClick={() => onAdd(selected)}
            className="inline-flex shrink-0 items-center justify-center gap-1.5 rounded-md bg-primary px-4 py-2 text-sm text-primary-foreground hover:bg-primary/90 disabled:cursor-not-allowed disabled:opacity-50"
          >
            <Plus className="h-4 w-4" aria-hidden /> 添加接入
          </button>
        </div>

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

        {!kind && (
          <p className="rounded-lg border border-warning/30 bg-warning/10 px-3 py-2 text-sm text-warning">
            这个助手暂不支持在应用内切换接入；安装、启动命令和技能管理仍可在下方完成。
          </p>
        )}

        <div className="grid grid-cols-1 gap-3 lg:grid-cols-2">
          <ConnectionCard
            agentId={agentId}
            connection={null}
            active={!active}
            busy={busyTarget === `${agentId}/default`}
            disabled={busyTarget !== null}
            onActivate={kind ? () => onSwitch(selected, "") : undefined}
          />
          {form.connections.map((connection) => {
            const key = `${agentId}/${connection.name}`;
            return (
              <ConnectionCard
                key={connection.name}
                agentId={agentId}
                connection={connection}
                active={active === connection.name}
                busy={busyTarget === key}
                disabled={busyTarget !== null}
                check={checkResults[key]}
                onActivate={kind ? () => onSwitch(selected, connection.name) : undefined}
                onCheck={kind ? () => onCheck(selected, connection.name) : undefined}
                onEdit={kind ? () => onEdit(selected, connection) : undefined}
                onRemove={kind ? () => onRemove(selected, connection) : undefined}
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
  connection,
  active,
  busy,
  disabled,
  check,
  onActivate,
  onCheck,
  onEdit,
  onRemove,
}: {
  agentId: string;
  connection: ConnectionEntry | null;
  active: boolean;
  busy: boolean;
  disabled: boolean;
  check?: ConnectionCheckResult;
  onActivate?: () => void;
  onCheck?: () => void;
  onEdit?: () => void;
  onRemove?: () => void;
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
        <dl className="mt-3 grid grid-cols-1 gap-2 text-xs sm:grid-cols-2">
          <div>
            <dt className="text-muted-foreground">模型</dt>
            <dd className="mt-0.5 break-all font-mono text-foreground">{connection.model || "未指定"}</dd>
          </div>
          <div>
            <dt className="text-muted-foreground">凭据变量</dt>
            <dd className="mt-0.5 break-all font-mono text-foreground">{connection.tokenVar || "未指定"}</dd>
          </div>
        </dl>
      )}

      {connection && (onCheck || onEdit || onRemove) && (
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
