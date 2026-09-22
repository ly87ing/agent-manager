import { useState, type MouseEvent } from "react";
import { Activity, ArrowDown, ArrowUp, Check, Download, Loader2, MoreHorizontal, Pencil, Settings2, Terminal, Trash2, Upload } from "lucide-react";
import type { ConnectionKind } from "./ConnectionFormDialog";
import { AgentCurrentConfigView } from "./AgentCurrentConfigView";
import type { EngineElement } from "../lib/contract";
import type { AgentCurrentConfigStatus, AgentsObserved, AgentsSpec } from "../lib/observed";
import {
  parseInitialToFormValues,
  runtimeSettingsOf,
  type ConnectionEntry,
} from "../lib/agentDefinitionForm";
import { connectionKindOf } from "../lib/agentManagement";
import { runtimeAlignment, runtimeComparisons } from "../lib/agentRuntime";
import { parseAliasValue, renderAliasPreview } from "../lib/aliasCommand";
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

/** 默认实例当前选择的自动化档位；显示词仍由后端适配器 schema 提供。 */
function automationValueOf(element: EngineElement): string {
  const spec = element.spec as Record<string, unknown>;
  const settings = spec.settings && typeof spec.settings === "object" && !Array.isArray(spec.settings)
    ? spec.settings as Record<string, unknown>
    : {};
  return typeof settings.automation_mode === "string"
    ? settings.automation_mode
    : settings.yolo === true ? "full" : "manual";
}

function automationLabelOf(element: EngineElement, value = automationValueOf(element)): string {
  const management = (element.observed as AgentsObserved).management;
  const field = management?.runtime_fields.find((candidate) => candidate.key === "automation_mode");
  return field?.summary?.values?.[value] ?? field?.summary?.default ?? "未设置";
}

/** details 没有内建“选择后关闭”，执行菜单动作时显式收起，避免弹窗后面还挂着旧菜单。 */
function runMenuAction(event: MouseEvent<HTMLButtonElement>, action: (() => void) | undefined) {
  event.currentTarget.closest("details")?.removeAttribute("open");
  action?.();
}

type WorkbenchTab = "providers" | "automation" | "launch" | "actual";

function automationOverrideOf(value: string): string | null {
  const args = parseAliasValue(value).extraArgs;
  if (/(^|\s)--dangerously-skip-permissions(?=\s|$)/.test(args)) return "启动时跳过所有权限确认";
  if (/(^|\s)--enable-auto-mode(?=\s|$)/.test(args)
    || /(^|\s)--permission-mode(?:=|\s+)auto(?=\s|$)/.test(args)) return "启动时启用智能自动";
  if (/(^|\s)--dangerously-bypass-approvals-and-sandbox(?=\s|$)/.test(args)) return "启动时绕过审批与沙箱";
  if (/(^|\s)--approve-for-me(?=\s|$)/.test(args)) return "启动时启用自动审查并保留沙箱";
  if (/(^|\s)--full-auto(?=\s|$)/.test(args)) return "启动时启用 Full Auto";
  if (/(^|\s)--permission-mode(?:=|\s+)/.test(args)
    || /(^|\s)--ask-for-approval(?:=|\s+)/.test(args)
    || /(^|\s)--sandbox(?:=|\s+)/.test(args)) return "启动参数另行指定权限方式";
  return null;
}

export type ConnectionAlignment = "aligned" | "drifted" | "unknown";

function observedText(value: unknown): string {
  return value === undefined || value === null ? "" : String(value);
}

/** 只比较接入流程明确拥有、且 status 能脱敏观察到的字段。 */
export function connectionAlignment(
  kind: ConnectionKind,
  connection: ConnectionEntry | null,
  status: AgentCurrentConfigStatus,
): ConnectionAlignment {
  if (status.state === "missing") return connection ? "drifted" : "aligned";
  if (status.state !== "ready" || !status.config) return "unknown";
  const config = status.config;
  const credential = config.credential;
  if (!connection) {
    const hasManagedValue = [
      config.base_url,
      config.model,
      config.small_fast_model,
      config.model_aliases?.opus,
      config.model_aliases?.sonnet,
      config.model_aliases?.haiku,
      config.timeout_ms,
    ].some((value) => observedText(value) !== "");
    return !hasManagedValue && credential?.configured !== true ? "aligned" : "drifted";
  }

  const commonMatches = observedText(config.base_url) === connection.baseUrl
    && observedText(config.model) === connection.model
    && observedText(config.timeout_ms) === connection.timeoutMs;
  if (!commonMatches) return "drifted";
  if (kind === "codex") {
    return config.provider === connection.name
      && credential?.configured === true
      && credential.variable === connection.tokenVar
      ? "aligned"
      : "drifted";
  }
  return observedText(config.small_fast_model) === connection.smallFastModel
    && observedText(config.model_aliases?.opus) === connection.aliasOpus
    && observedText(config.model_aliases?.sonnet) === connection.aliasSonnet
    && observedText(config.model_aliases?.haiku) === connection.aliasHaiku
    && credential?.configured === true
    && credential.kind === connection.auth
    ? "aligned"
    : "drifted";
}

export function ConnectionWorkbench({
  agents,
  selectedAgentId,
  busyTarget,
  checkingAllAgentId,
  checkResults,
  onSelectAgent,
  onAdd,
  onImport,
  onSwitch,
  onResetDefault,
  aliases,
  aliasGlobals,
  aliasBusy,
  onAddLaunch,
  onEditLaunch,
  onRemoveLaunch,
  onSyncLaunch,
  onCheck,
  onCheckAll,
  onEdit,
  onDuplicate,
  onExport,
  onRemove,
  onMove,
  onRuntimeSettings,
  onApplyRuntime,
}: {
  agents: EngineElement[];
  selectedAgentId: string | null;
  busyTarget: string | null;
  checkingAllAgentId: string | null;
  checkResults: Record<string, ConnectionCheckResult>;
  onSelectAgent: (agentId: string) => void;
  onAdd: (element: EngineElement) => void;
  onImport: (element: EngineElement) => void;
  onSwitch: (element: EngineElement, connection: string) => void;
  onResetDefault: (element: EngineElement) => void;
  aliases: Record<string, string>;
  aliasGlobals: { timezone: string; lang: string };
  aliasBusy: boolean;
  onAddLaunch: (element: EngineElement) => void;
  onEditLaunch: (element: EngineElement, name: string) => void;
  onRemoveLaunch: (name: string) => void;
  onSyncLaunch: () => void;
  onCheck: (element: EngineElement, connection: string) => void;
  onCheckAll: (element: EngineElement, connections: ConnectionEntry[]) => void;
  onEdit: (element: EngineElement, connection: ConnectionEntry) => void;
  onDuplicate: (element: EngineElement, connection: ConnectionEntry) => void;
  onExport: (element: EngineElement, connection: ConnectionEntry) => void;
  onRemove: (element: EngineElement, connection: ConnectionEntry) => void;
  onMove: (element: EngineElement, connection: ConnectionEntry, direction: "up" | "down") => void;
  onRuntimeSettings: (element: EngineElement) => void;
  onApplyRuntime: (element: EngineElement) => void;
}) {
  const [tab, setTab] = useState<WorkbenchTab>("providers");
  const selected = agents.find((element) => element.id === selectedAgentId) ?? agents[0];
  if (!selected) return null;

  const agentId = selected.id.replace(/^agents:/, "");
  const name = displayNameOf(selected);
  const savedAutomationValue = automationValueOf(selected);
  const savedAutomationLabel = automationLabelOf(selected, savedAutomationValue);
  const management = (selected.observed as AgentsObserved).management;
  const runtimeFields = management?.runtime_fields ?? [];
  const savedRuntime = runtimeSettingsOf(selected.spec as Record<string, unknown>);
  const form = parseInitialToFormValues(agentId, selected.spec as Record<string, unknown>);
  const active = form.activeConnection;
  const activeEntry = form.connections.find((item) => item.name === active);
  const kind = connectionKindOf(selected);
  const bin = (selected.spec as AgentsSpec).bin ?? (kind === "codex" ? "codex" : "claude");
  const launchAliases = Object.entries(aliases)
    .filter(([, value]) => {
      const parsed = parseAliasValue(value);
      return parsed.command === bin
        && parsed.otherEnv.CLAUDE_CONFIG_DIR === undefined
        && parsed.otherEnv.CODEX_HOME === undefined;
    })
    .sort(([left], [right]) => left.localeCompare(right));
  const currentConfig = (selected.observed as AgentsObserved).current_config;
  const runtimeState = runtimeAlignment(runtimeFields, savedRuntime, currentConfig);
  const runtimeComparisonItems = runtimeComparisons(runtimeFields, savedRuntime, currentConfig);
  const runtimeDifferences = runtimeComparisonItems.filter((item) => item.aligned === false);
  const automationComparison = runtimeComparisonItems.find((item) => item.key === "automation_mode");
  const activeAlignment = kind && currentConfig
    ? connectionAlignment(kind, activeEntry ?? null, currentConfig)
    : "unknown";
  const connectionCheckStates = form.connections
    .map((connection) => ({ connection, result: checkResults[`${agentId}/${connection.name}`] }));
  const connectionChecks = connectionCheckStates.filter(
    (item): item is { connection: ConnectionEntry; result: ConnectionCheckResult } =>
      Boolean(item.result && !item.result.pending),
  );
  const allChecked = form.connections.length > 0 && connectionChecks.length === form.connections.length;
  const fastest = connectionChecks
    .filter((item) => item.result.ok && item.result.latencyMs > 0)
    .sort((a, b) => a.result.latencyMs - b.result.latencyMs)[0];
  const checkingAll = checkingAllAgentId === agentId;
  const orderedEntries: Array<ConnectionEntry | null> = activeEntry
    ? [activeEntry, null, ...form.connections.filter((connection) => connection.name !== active)]
    : [null, ...form.connections];

  return (
    <section data-testid="connection-workbench" className="rounded-xl border bg-card p-4 shadow-sm sm:p-5">
      <div className="flex flex-col gap-5">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div aria-label="选择助手" className="inline-flex rounded-xl bg-muted p-1" role="group">
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
                  "rounded-lg px-5 py-2 text-sm font-medium transition-colors",
                  chosen
                    ? "bg-background text-foreground shadow-sm"
                    : "text-muted-foreground hover:text-foreground",
                )}
              >
                {displayNameOf(element)}
              </button>
            );
          })}
          </div>
          <span className="sr-only" data-testid="active-connection-summary">
            {activeEntry?.label || activeEntry?.name || "助手默认账号"}
          </span>
          {tab === "providers" && <div className="flex items-center gap-2">
            {form.connections.length > 0 && (
              <button
                type="button"
                data-testid="check-all-connections-button"
                disabled={!kind || checkingAll || connectionCheckStates.some((item) => item.result?.pending)}
                onClick={() => onCheckAll(selected, form.connections)}
                title="检查全部接入"
                aria-label="检查全部接入"
                className="inline-flex h-10 items-center justify-center gap-1.5 rounded-xl border px-3 text-sm text-muted-foreground hover:bg-accent disabled:opacity-50"
              >
                {checkingAll ? <Loader2 className="h-4 w-4 animate-spin" aria-hidden /> : <Activity className="h-4 w-4" aria-hidden />}
                {checkingAll ? "检查中…" : "检查全部"}
              </button>
            )}
            <details className="group relative">
              <summary aria-label="更多设置" className="inline-flex h-10 w-10 cursor-pointer list-none items-center justify-center rounded-xl border text-muted-foreground hover:bg-accent">
                <Settings2 className="h-4 w-4" aria-hidden />
              </summary>
              <div className="absolute right-0 z-10 mt-2 flex w-40 flex-col rounded-lg border bg-popover p-1 shadow-lg">
                <button
                  type="button"
                  data-testid="import-connection-button"
                  disabled={!kind}
                  onClick={(event) => runMenuAction(event, () => onImport(selected))}
                  className="inline-flex items-center gap-2 rounded px-3 py-2 text-left text-sm hover:bg-accent disabled:opacity-50"
                >
                  <Upload className="h-4 w-4" aria-hidden /> 导入接入
                </button>
              </div>
            </details>
            <button
              type="button"
              data-testid="add-connection-button"
              disabled={!kind}
              onClick={() => onAdd(selected)}
              className="inline-flex h-10 items-center rounded-xl bg-primary px-4 text-sm font-medium text-primary-foreground shadow-sm hover:bg-primary/90 disabled:cursor-not-allowed disabled:opacity-50"
            >
              添加 Provider
            </button>
          </div>}
        </div>

        <div className="flex flex-wrap items-center justify-between gap-3 rounded-xl border border-primary/20 bg-primary/5 px-4 py-3">
          <div>
            <div className="flex flex-wrap items-center gap-2">
              <span className="rounded-full bg-primary/10 px-2 py-0.5 text-[11px] font-medium text-primary">默认环境</span>
              <strong className="text-sm font-semibold text-foreground">{name}</strong>
            </div>
            <p className="mt-1 text-xs text-muted-foreground">
              当前选择：{activeEntry?.label || activeEntry?.name || "助手默认账号"} · {runtimeState === "aligned"
                ? `运行设置已生效（自动化：${savedAutomationLabel}）`
                : runtimeState === "drifted"
                  ? `${runtimeDifferences.length} 项运行设置尚未生效`
                  : "运行设置暂时无法核对"}。
              {activeAlignment === "drifted" ? " 当前 Provider 尚未应用，请在下方点击应用。" : " 下面所有页签都属于这一个环境。"}
            </p>
          </div>
        </div>

        <div aria-label="环境配置" role="tablist" className="flex gap-1 overflow-x-auto border-b">
          {([
            ["providers", "接入 Provider"],
            ["automation", "自动化与权限"],
            ["launch", "启动命令"],
            ["actual", "实际配置"],
          ] as const).map(([id, label]) => (
            <button
              key={id}
              type="button"
              role="tab"
              aria-selected={tab === id}
              data-testid={`workbench-tab-${id}`}
              onClick={() => setTab(id)}
              className={cn(
                "whitespace-nowrap border-b-2 px-3 py-2 text-sm",
                tab === id ? "border-primary font-medium text-foreground" : "border-transparent text-muted-foreground hover:text-foreground",
              )}
            >
              {label}
            </button>
          ))}
        </div>

        {tab === "automation" && (
          <div data-testid="automation-panel" className="flex flex-wrap items-center justify-between gap-3 rounded-xl border px-4 py-4">
            <div>
              <h3 className="text-sm font-medium text-foreground">
                {runtimeState === "aligned"
                  ? `运行设置已生效 · ${savedAutomationLabel}`
                  : runtimeState === "drifted"
                    ? `${runtimeDifferences.length} 项设置尚未生效`
                    : "客户端实际值暂不可读"}
              </h3>
              <p className="mt-1 text-xs text-muted-foreground">
                {runtimeState === "aligned"
                  ? "软件保存值与客户端实际配置逐项一致；启动命令若带权限参数，会在启动时覆盖它。"
                  : runtimeState === "drifted"
                    ? "下面直接列出不一致项；应用只写本工具负责的配置键。"
                    : "可以先到“实际配置”查看读取失败原因，再决定是否重新应用。"}
              </p>
              {runtimeDifferences.length > 0 && (
                <ul data-testid="runtime-differences" className="mt-2 space-y-1 text-xs">
                  {runtimeDifferences.map((item) => (
                    <li key={item.key} className="text-warning">
                      {item.label}：已保存 {item.saved}，客户端实际 {item.actual}
                    </li>
                  ))}
                </ul>
              )}
              {automationComparison?.aligned === true && runtimeState === "drifted" && (
                <p className="mt-2 text-xs text-muted-foreground">自动化档位已是 {automationComparison.actual}；差异来自其他运行设置。</p>
              )}
            </div>
            <div className="flex flex-wrap items-center gap-2">
              {runtimeState !== "aligned" && (
                <button type="button" data-testid="agent-runtime-apply" disabled={!kind || busyTarget !== null} onClick={() => onApplyRuntime(selected)} className="rounded-lg bg-primary px-3 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50">
                  应用已保存设置
                </button>
              )}
              <button type="button" data-testid="agent-runtime-button" disabled={!kind} onClick={() => onRuntimeSettings(selected)} className="rounded-lg border px-3 py-2 text-sm font-medium text-foreground hover:bg-accent disabled:opacity-50">
                修改自动化与权限
              </button>
            </div>
          </div>
        )}

        {tab === "launch" && <div data-testid={`agent-launch-commands-${agentId}`} className="rounded-xl border px-4 py-3">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <div>
              <h3 className="flex items-center gap-1.5 text-sm font-medium text-foreground">
                <Terminal className="h-4 w-4" aria-hidden /> 运行别名
              </h3>
              <p className="mt-0.5 text-xs text-muted-foreground">保存只改本机定义；点“同步到终端”才写入 shell 配置。</p>
            </div>
            <div className="flex items-center gap-2">
              <button type="button" data-testid={`agent-add-launch-${agentId}`} onClick={() => onAddLaunch(selected)} className="rounded border px-2.5 py-1.5 text-xs hover:bg-accent">
                新增别名
              </button>
              <button type="button" data-testid={`agent-sync-launch-${agentId}`} disabled={aliasBusy} onClick={onSyncLaunch} className="rounded border px-2.5 py-1.5 text-xs text-muted-foreground hover:bg-accent disabled:opacity-50">
                {aliasBusy ? "同步中…" : "同步到终端"}
              </button>
            </div>
          </div>
          {launchAliases.length === 0 ? (
            <p className="mt-3 text-xs text-muted-foreground">还没有属于这个 Agent 的运行别名。</p>
          ) : (
            <ul className="mt-3 space-y-2">
              {launchAliases.map(([name, value]) => {
                const override = automationOverrideOf(value);
                return (
                <li key={name} data-testid={`agent-launch-${agentId}-${name}`} className="rounded-lg bg-muted/35 px-3 py-2">
                  <div className="flex items-center justify-between gap-2">
                    <div className="flex flex-wrap items-center gap-2">
                      <code className="text-xs font-semibold text-foreground">{name}</code>
                      {override && <span data-testid={`agent-launch-override-${agentId}-${name}`} className="rounded-full bg-warning/15 px-2 py-0.5 text-[11px] font-medium text-warning">覆盖环境自动化</span>}
                    </div>
                    <div className="flex items-center gap-1">
                      <button type="button" data-testid={`agent-launch-edit-${agentId}-${name}`} onClick={() => onEditLaunch(selected, name)} className="rounded border px-2 py-1 text-[11px] text-muted-foreground hover:bg-accent">编辑</button>
                      <button type="button" data-testid={`agent-launch-remove-${agentId}-${name}`} onClick={() => onRemoveLaunch(name)} className="inline-flex items-center gap-1 rounded border border-destructive/30 px-2 py-1 text-[11px] text-destructive hover:bg-destructive/10">
                        <Trash2 className="h-3 w-3" aria-hidden /> 删除
                      </button>
                    </div>
                  </div>
                  <pre className="mt-1 overflow-x-auto whitespace-pre-wrap break-all font-mono text-[11px] text-muted-foreground">{renderAliasPreview(name, value, aliasGlobals)}</pre>
                  {override && <p className="mt-1 text-xs text-warning">{override}</p>}
                </li>
                );
              })}
            </ul>
          )}
        </div>}

        {tab === "actual" && (
          <div data-testid="actual-config-panel" className="space-y-3 rounded-xl border px-4 py-4">
            <div>
              <h3 className="text-sm font-medium text-foreground">当前实际生效值</h3>
              <p className="mt-1 text-xs text-muted-foreground">按用途整理客户端最终读取值；敏感值只显示是否已配置。</p>
            </div>
            <AgentCurrentConfigView
              status={currentConfig}
              fields={runtimeFields}
              savedRuntime={savedRuntime}
            />
          </div>
        )}

        {!kind && (
          <p className="rounded-lg border border-warning/30 bg-warning/10 px-3 py-2 text-sm text-warning">
            这个助手暂不支持在应用内切换接入或写入运行设置；已有配置仅只读展示。
          </p>
        )}

        {tab === "providers" && connectionChecks.length > 0 && (
          <div
            data-testid="connection-check-summary"
            className="flex flex-wrap items-center justify-between gap-2 rounded-lg border bg-muted/30 px-3 py-2 text-sm"
          >
            <p className="text-muted-foreground">
              接入检查：已完成 {connectionChecks.length}/{form.connections.length}，
              <strong className="font-medium text-foreground">
                {connectionChecks.filter((item) => item.result.ok).length} 个可用
              </strong>
            </p>
            {allChecked && fastest && (
              <div className="flex flex-wrap items-center gap-2">
                <p className="text-success">
                  最快：{fastest.connection.label || fastest.connection.name} {fastest.result.latencyMs}ms
                </p>
                {fastest.connection.name !== active && (
                  <button
                    type="button"
                    data-testid="activate-fastest-connection"
                    disabled={busyTarget !== null}
                    onClick={() => onSwitch(selected, fastest.connection.name)}
                    className="rounded border border-success/30 bg-success/10 px-2.5 py-1 text-xs font-medium text-success hover:bg-success/15 disabled:opacity-50"
                  >
                    启用最快接入
                  </button>
                )}
              </div>
            )}
          </div>
        )}

        {tab === "providers" && <div className="flex flex-col gap-3">
            {orderedEntries.map((connection) => {
              const connectionName = connection?.name ?? "";
              const key = `${agentId}/${connectionName || "default"}`;
              const index = connection
                ? form.connections.findIndex((item) => item.name === connection.name)
                : -1;
              const alignment = kind && currentConfig
                ? connectionAlignment(kind, connection, currentConfig)
                : "unknown";
              return (
              <ConnectionCard
                key={connection?.name ?? "default"}
                agentId={agentId}
                connection={connection}
                active={active === connectionName}
                alignment={alignment}
                busy={busyTarget === key}
                disabled={busyTarget !== null}
                check={checkResults[key]}
                onActivate={kind && connection && (active !== connectionName || alignment === "drifted")
                  ? () => onSwitch(selected, connectionName)
                  : undefined}
                onReset={kind && !connection ? () => onResetDefault(selected) : undefined}
                onCheck={kind && connection ? () => onCheck(selected, connection.name) : undefined}
                onEdit={kind && connection ? () => onEdit(selected, connection) : undefined}
                onDuplicate={kind && connection ? () => onDuplicate(selected, connection) : undefined}
                onExport={kind && connection ? () => onExport(selected, connection) : undefined}
                onRemove={kind && connection ? () => onRemove(selected, connection) : undefined}
                onMoveUp={kind && connection && index > 0 ? () => onMove(selected, connection, "up") : undefined}
                onMoveDown={kind && connection && index < form.connections.length - 1 ? () => onMove(selected, connection, "down") : undefined}
              />
              );
            })}
        </div>}
      </div>
    </section>
  );
}

function ConnectionCard({
  agentId,
  connection,
  active,
  alignment,
  busy,
  disabled,
  check,
  onActivate,
  onReset,
  onCheck,
  onEdit,
  onDuplicate,
  onExport,
  onRemove,
  onMoveUp,
  onMoveDown,
}: {
  agentId: string;
  connection: ConnectionEntry | null;
  active: boolean;
  alignment: ConnectionAlignment;
  busy: boolean;
  disabled: boolean;
  check?: ConnectionCheckResult;
  onActivate?: () => void;
  onReset?: () => void;
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
      onClick={(event) => {
        if (onEdit && !(event.target as HTMLElement).closest("button, details, summary")) onEdit();
      }}
      className={cn(
        "flex min-h-24 items-center gap-4 rounded-2xl border bg-background px-5 py-4 shadow-sm transition-colors",
        onEdit && "cursor-pointer",
        active && alignment === "aligned" && "border-success/40 bg-success/5",
        active && alignment === "drifted" && "border-warning/40 bg-warning/5",
        (!active || alignment === "unknown") && "hover:border-primary/30",
      )}
    >
      <div className="flex h-12 w-12 shrink-0 items-center justify-center rounded-xl border bg-muted text-lg font-semibold text-foreground">
        {(connection?.label || connection?.name || "默").slice(0, 1).toUpperCase()}
      </div>
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-2">
          <h3 className="text-lg font-semibold text-foreground">
            {connection ? connection.label || connection.name : "助手默认账号"}
          </h3>
          {active && alignment !== "unknown" && (
            <span className={cn(
              "inline-flex items-center gap-1 rounded-full px-2.5 py-1 text-xs font-medium",
              alignment === "drifted" ? "bg-warning/15 text-warning" : "bg-success/15 text-success",
            )}>
              {alignment === "aligned" && <Check className="h-3 w-3" aria-hidden />}
              {alignment === "drifted" ? "尚未生效" : "当前使用"}
            </span>
          )}
          {active && alignment === "unknown" && (
            <span className="rounded-full bg-muted px-2.5 py-1 text-xs font-medium text-muted-foreground">无法确认是否生效</span>
          )}
        </div>
        <p className="mt-1 truncate text-sm text-primary">
          {connection?.baseUrl || "使用助手自己的登录与默认地址"}
        </p>
        {connection?.note && (
          <p data-testid={`connection-note-${agentId}-${id}`} className="mt-1 truncate text-xs text-muted-foreground">
            {connection.note}
          </p>
        )}
      </div>
      <div className="flex shrink-0 items-center gap-2">
        {check && !check.pending && (
          <span
            data-testid={`check-result-${agentId}-${id}`}
            className={cn("text-xs", check.ok ? "text-success" : "text-destructive")}
          >
            {check.ok && check.latencyMs > 0 ? `${check.latencyMs}ms` : check.detail}
          </span>
        )}
        {onActivate && (
          <button
            type="button"
            data-testid={`activate-connection-${agentId}-${id}`}
            disabled={disabled}
            onClick={onActivate}
            className="inline-flex shrink-0 items-center gap-1 rounded-lg bg-primary px-3 py-2 text-xs font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
          >
            {busy && <Loader2 className="h-3.5 w-3.5 animate-spin" aria-hidden />}
            {busy ? "正在应用…" : active ? "应用修改" : "应用"}
          </button>
        )}
        {onReset && (
          <button
            type="button"
            data-testid={`reset-connection-${agentId}-${id}`}
            disabled={disabled}
            onClick={onReset}
            className={cn(
              "inline-flex shrink-0 items-center gap-1 rounded-lg px-3 py-2 text-xs font-medium disabled:opacity-50",
              alignment === "drifted"
                ? "bg-primary text-primary-foreground hover:bg-primary/90"
                : "border text-foreground hover:bg-accent",
            )}
          >
            {busy && <Loader2 className="h-3.5 w-3.5 animate-spin" aria-hidden />}
            {busy ? "正在应用…" : alignment === "drifted" ? "应用默认配置" : "重新应用"}
          </button>
        )}
        {onEdit && (
          <button
            type="button"
            data-testid={`edit-connection-${agentId}-${id}`}
            aria-label={connection ? `编辑 ${connection.label || connection.name}` : "添加可编辑 Provider"}
            title={connection ? "编辑 Provider" : "默认账号由助手管理；添加一个可编辑 Provider"}
            disabled={disabled}
            onClick={onEdit}
            className="inline-flex h-9 items-center justify-center gap-1.5 rounded-lg border px-2.5 text-xs text-muted-foreground hover:border-primary/40 hover:bg-accent hover:text-foreground disabled:opacity-50"
          >
            <Pencil className="h-4 w-4" aria-hidden />
            {connection ? "编辑" : "添加 Provider"}
          </button>
        )}
        {connection && (onCheck || onEdit || onDuplicate || onExport || onRemove || onMoveUp || onMoveDown) && (
          <details className="group relative">
            <summary aria-label={`管理 ${connection.label || connection.name}`} className="inline-flex h-9 w-9 cursor-pointer list-none items-center justify-center rounded-lg border text-muted-foreground hover:bg-accent">
              <MoreHorizontal className="h-4 w-4" aria-hidden />
            </summary>
            <div className="absolute right-0 top-full z-10 mt-1 flex w-44 flex-col rounded-lg border bg-popover p-1 shadow-lg">
          {onCheck && (
            <button
              type="button"
              data-testid={`check-connection-${agentId}-${id}`}
              disabled={check?.pending}
              onClick={(event) => runMenuAction(event, onCheck)}
              className="rounded px-3 py-2 text-left text-xs text-muted-foreground hover:bg-accent disabled:opacity-50"
            >
              {check?.pending ? "正在测试…" : "测试连接"}
            </button>
          )}
            <div className="flex items-center" aria-label="调整接入顺序">
              <button
                type="button"
                aria-label={`上移 ${connection.label || connection.name}`}
                disabled={disabled || !onMoveUp}
                onClick={(event) => runMenuAction(event, onMoveUp)}
                className="flex-1 rounded-l border px-2 py-1.5 text-muted-foreground hover:bg-accent disabled:opacity-30"
              >
                <ArrowUp className="h-3.5 w-3.5" aria-hidden />
              </button>
              <button
                type="button"
                aria-label={`下移 ${connection.label || connection.name}`}
                disabled={disabled || !onMoveDown}
                onClick={(event) => runMenuAction(event, onMoveDown)}
                className="flex-1 rounded-r border border-l-0 px-2 py-1.5 text-muted-foreground hover:bg-accent disabled:opacity-30"
              >
                <ArrowDown className="h-3.5 w-3.5" aria-hidden />
              </button>
            </div>
            {onDuplicate && (
              <button
                type="button"
                data-testid={`duplicate-connection-${agentId}-${id}`}
                disabled={disabled}
                onClick={(event) => runMenuAction(event, onDuplicate)}
                className="rounded px-3 py-2 text-left text-xs text-muted-foreground hover:bg-accent disabled:opacity-50"
              >
                创建副本
              </button>
            )}
            {onExport && (
              <button
                type="button"
                data-testid={`export-connection-${agentId}-${id}`}
                disabled={disabled}
                onClick={(event) => runMenuAction(event, onExport)}
                className="inline-flex items-center gap-1 rounded px-3 py-2 text-left text-xs text-muted-foreground hover:bg-accent disabled:opacity-50"
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
                onClick={(event) => runMenuAction(event, onRemove)}
                className="rounded px-3 py-2 text-left text-xs text-destructive hover:bg-destructive/10 disabled:cursor-not-allowed disabled:opacity-50"
              >
                删除
              </button>
            )}
            </div>
          </details>
        )}
      </div>
    </article>
  );
}
