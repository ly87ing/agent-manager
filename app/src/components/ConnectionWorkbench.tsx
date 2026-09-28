import { useState, type MouseEvent, type ReactNode } from "react";
import { Activity, ArrowDown, ArrowUp, Check, ChevronDown, ChevronRight, Download, Loader2, MoreHorizontal, Pencil, Plus, Save, Terminal, Trash2, Upload } from "lucide-react";
import { AgentRuntimePanel } from "./AgentRuntimePanel";
import { ConfigFileEditor } from "./ConfigFileEditor";
import { OfficialAuthPanel } from "./OfficialAuthPanel";
import type { EngineElement } from "../lib/contract";
import type { AgentCurrentConfig, AgentsObserved, AgentsSpec, OfficialAuthStatus } from "../lib/observed";
import {
  isPlainHttpUrl,
  connectionEntriesOf,
  type ConnectionEntry,
} from "../lib/agentDefinitionForm";
import { connectionKindOf } from "../lib/agentManagement";
import { configureBaseOf, currentConfigOf, currentConnectionOf, hasUnsavedLive } from "../lib/agentCurrent";
import { actualRuntimeSettings, automationRawText, runtimeValueLabel } from "../lib/agentRuntime";
import { parseAliasValue, renderAliasPreview } from "../lib/aliasCommand";
import { cn } from "../lib/utils";
import { overrideEffect } from "../lib/overrideEffects";
import { defaultEnvState, isOfficial } from "../lib/defaultEnvironment";
import { balanceSupported } from "../lib/balance";

export interface ConnectionCheckResult {
  ok: boolean;
  detail: string;
  latencyMs: number;
  pending?: boolean;
}

function displayNameOf(element: EngineElement): string {
  return (element.spec as AgentsSpec).display_name ?? element.id.replace(/^agents:/, "");
}

/** 客户端实际的自动化档位；显示词由后端适配器 schema 提供。 */
function automationLabelOf(element: EngineElement): string {
  const management = (element.observed as AgentsObserved).management;
  const field = management?.runtime_fields.find((candidate) => candidate.key === "automation_mode");
  const status = currentConfigOf(element);
  if (!field || !status || (status.state !== "ready" && status.state !== "missing")) return "暂不可读";
  const value = status.state === "missing" ? "manual" : status.config?.runtime?.automation_mode ?? "manual";
  return runtimeValueLabel(field, value, automationRawText(status));
}

/** details 没有内建“选择后关闭”，执行菜单动作时显式收起，避免弹窗后面还挂着旧菜单。 */
function runMenuAction(event: MouseEvent<HTMLButtonElement>, action: (() => void) | undefined) {
  event.currentTarget.closest("details")?.removeAttribute("open");
  action?.();
}

type WorkbenchTab = "providers" | "automation" | "launch";

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

/** 实际配置读不了（不是普通文件、无法解析等）时说明为什么不能写。 */
function unwritableReason(element: EngineElement): string | null {
  if (configureBaseOf(element)) return null;
  const state = currentConfigOf(element)?.state;
  return state === "invalid"
    ? "配置文件无法解析，修好之前不能写入"
    : "客户端配置文件暂不可读，不能写入";
}

export function ConnectionWorkbench({
  agents,
  selectedAgentId,
  busyTarget,
  checkingAllAgentId,
  checkResults,
  instancesOf,
  onShowInstances,
  onBalance,
  onSelectAgent,
  onAdd,
  onImport,
  onResetDefault,
  aliases,
  onAddLaunch,
  onEditLaunch,
  onRemoveLaunch,
  onCheck,
  onCheckAll,
  onEdit,
  onDuplicate,
  onMakeProfile,
  onExport,
  onRemove,
  onMove,
  runtimeSaving,
  onSaveRuntime,
  onSaveCurrent,
  onFixStatusline,
  onClearEnvOverrides,
  onCheckAuth,
  renderInstances,
  onNewEnvironment,
}: {
  agents: EngineElement[];
  selectedAgentId: string | null;
  busyTarget: string | null;
  checkingAllAgentId: string | null;
  checkResults: Record<string, ConnectionCheckResult>;
  /** 由这套接入建立的独立环境；旧定义按地址与凭据变量推断。 */
  instancesOf?: (agentId: string, connection: ConnectionEntry) => { id: string; label: string }[];
  /** 定位这套接入建立的独立环境卡片。 */
  onShowInstances?: (id: string) => void;
  /** 查这套接入凭据的余额（只对有官方余额接口的厂商）；返回一行结果或失败原因。 */
  onBalance?: (element: EngineElement, connection: ConnectionEntry) => Promise<string>;
  onSelectAgent: (agentId: string) => void;
  onAdd: (element: EngineElement) => void;
  onImport: (element: EngineElement) => void;
  onResetDefault: (element: EngineElement) => void;
  aliases: Record<string, string>;
  onAddLaunch: (element: EngineElement) => void;
  onEditLaunch: (element: EngineElement, name: string) => void;
  onRemoveLaunch: (name: string) => void;
  onCheck: (element: EngineElement, connection: string) => void;
  onCheckAll: (element: EngineElement, connections: ConnectionEntry[]) => void;
  onEdit: (element: EngineElement, connection: ConnectionEntry) => void;
  onDuplicate: (element: EngineElement, connection: ConnectionEntry) => void;
  /** 把这套接入建成独立环境：第三方凭据放进新环境自己的目录，主配置留给官方账号。 */
  onMakeProfile?: (element: EngineElement, connection: ConnectionEntry) => void;
  /** 用官方命令查一次主配置的官方账号登录状态；失败时返回原因。 */
  onCheckAuth?: (element: EngineElement) => Promise<OfficialAuthStatus | string>;
  onExport: (element: EngineElement, connection: ConnectionEntry) => void;
  onRemove: (element: EngineElement, connection: ConnectionEntry) => void;
  onMove: (element: EngineElement, connection: ConnectionEntry, direction: "up" | "down") => void;
  runtimeSaving: boolean;
  /** 运行设置只发改动过的字段；撤掉的项为 null。 */
  onSaveRuntime: (element: EngineElement, patch: Record<string, unknown>, fileBase: string) => void;
  /** 把「当前实际配置（未保存）」存成接入配置，不写助手文件。 */
  onSaveCurrent: (element: EngineElement) => void;
  /** 旧格式状态栏：按引用形式重写一次。 */
  onFixStatusline: (element: EngineElement) => void;
  /** 清理旧版本写进 env 的单次会话覆盖变量（用户点按钮才执行）。 */
  onClearEnvOverrides: (element: EngineElement) => void;
  /** 这个助手的独立环境：与默认环境放在同一个列表里，不再分页签。 */
  renderInstances?: (element: EngineElement) => ReactNode;
  /** 新建环境：选第三方预设，或打开空白环境表单手动填写。 */
  onNewEnvironment?: (element: EngineElement, kind: "third-party" | "manual") => void;
}) {
  const [tab, setTab] = useState<WorkbenchTab>("providers");
  const [fileOpen, setFileOpen] = useState(false);
  const selected = agents.find((element) => element.id === selectedAgentId) ?? agents[0];
  if (!selected) return null;

  const agentId = selected.id.replace(/^agents:/, "");
  const name = displayNameOf(selected);
  const automationLabel = automationLabelOf(selected);
  const management = (selected.observed as AgentsObserved).management;
  const runtimeFields = management?.runtime_fields ?? [];
  const connections = connectionEntriesOf(selected.spec as Record<string, unknown>);
  // 当前项以实际配置为准：引擎按内容识别，库里的 active 只是上次写入记录
  const current = currentConnectionOf(selected);
  const active = current?.profile ?? "";
  const activeEntry = connections.find((item) => item.name === active);
  const unsaved = hasUnsavedLive(current);
  const writeBlocked = unwritableReason(selected);
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
  const currentConfig = currentConfigOf(selected);
  const overrides = (selected.observed as { credential_overrides?: { source: string; name: string }[] })
    .credential_overrides ?? [];
  const legacyStatusline = currentConfig?.config?.runtime?.statusline === "builtin-legacy";
  const modelEfforts = Object.entries(currentConfig?.config?.claude_model_efforts ?? {});
  const envOverrides = Object.entries(currentConfig?.config?.claude_env_overrides ?? {});
  const fatalIssues = currentConfig?.config?.fatal_issues ?? [];
  const defaultPermissions = currentConfig?.config?.default_permissions ?? null;
  const driftedEntry = current?.drifted_from
    ? connections.find((item) => item.name === current.drifted_from)
    : undefined;
  const official = isOfficial(current);
  const envState = kind ? defaultEnvState(kind, current, currentConfig?.config) : null;
  const thirdParty = envState?.account === "third-party";
  const configFile = kind === "codex" ? "~/.codex/config.toml" : "~/.claude/settings.json";
  const summary = activeEntry
    ? activeEntry.label || activeEntry.name
    : official
      ? "官方账号"
      : unsaved
        ? "当前实际配置（未保存）"
        : "无法读取实际配置";
  const connectionCheckStates = connections
    .map((connection) => ({ connection, result: checkResults[`${agentId}/${connection.name}`] }));
  const connectionChecks = connectionCheckStates.filter(
    (item): item is { connection: ConnectionEntry; result: ConnectionCheckResult } =>
      Boolean(item.result && !item.result.pending),
  );
  const allChecked = connections.length > 0 && connectionChecks.length === connections.length;
  const fastest = connectionChecks
    .filter((item) => item.result.ok && item.result.latencyMs > 0)
    .sort((a, b) => a.result.latencyMs - b.result.latencyMs)[0];
  const checkingAll = checkingAllAgentId === agentId;
  // 主配置正在用的那套已保存第三方接入放进「账号」一节；其余接入只在独立环境里用
  const mainThirdParty = thirdParty && activeEntry ? activeEntry : null;
  const savedEntries = connections.filter((connection) => connection !== mainThirdParty);

  const renderCard = (connection: ConnectionEntry, inMain: boolean) => {
    const key = `${agentId}/${connection.name}`;
    const index = connections.findIndex((item) => item.name === connection.name);
    return (
      <ConnectionCard
        key={connection.name}
        agentId={agentId}
        connection={connection}
        active={inMain}
        busy={busyTarget === key}
        disabled={busyTarget !== null || Boolean(writeBlocked)}
        check={checkResults[key]}
        onCheck={kind ? () => onCheck(selected, connection.name) : undefined}
        onEdit={kind ? () => onEdit(selected, connection) : undefined}
        onDuplicate={kind ? () => onDuplicate(selected, connection) : undefined}
        onMakeProfile={kind && onMakeProfile ? () => onMakeProfile(selected, connection) : undefined}
        instances={instancesOf ? instancesOf(agentId, connection) : []}
        onShowInstances={onShowInstances}
        onBalance={kind && onBalance && balanceSupported(connection.baseUrl) ? () => onBalance(selected, connection) : undefined}
        onExport={kind ? () => onExport(selected, connection) : undefined}
        onRemove={kind ? () => onRemove(selected, connection) : undefined}
        onMoveUp={kind && index > 0 ? () => onMove(selected, connection, "up") : undefined}
        onMoveDown={kind && index < connections.length - 1 ? () => onMove(selected, connection, "down") : undefined}
      />
    );
  };

  return (
    <section data-testid="connection-workbench" className="flex flex-col gap-4">
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
            {summary}
          </span>
          {kind && onNewEnvironment && (
            <details className="group relative">
              <summary
                data-testid="new-environment-button"
                className="inline-flex h-10 cursor-pointer list-none items-center gap-1.5 rounded-xl bg-primary px-4 text-sm font-medium text-primary-foreground shadow-sm hover:bg-primary/90"
              >
                <Plus className="h-4 w-4" aria-hidden /> 新建环境
              </summary>
              <div className="absolute right-0 z-10 mt-2 flex w-72 flex-col rounded-lg border bg-popover p-1 shadow-lg">
                <button
                  type="button"
                  data-testid="new-environment-third-party"
                  onClick={(event) => runMenuAction(event, () => onNewEnvironment(selected, "third-party"))}
                  className="rounded px-3 py-2 text-left hover:bg-accent"
                >
                  <span className="block text-sm text-foreground">第三方厂商或中转</span>
                  <span className="block text-xs text-muted-foreground">选厂商预设、填密钥，建成一个独立环境</span>
                </button>
                <button
                  type="button"
                  data-testid="new-environment-manual"
                  onClick={(event) => runMenuAction(event, () => onNewEnvironment(selected, "manual"))}
                  className="rounded px-3 py-2 text-left hover:bg-accent"
                >
                  <span className="block text-sm text-foreground">手动创建环境</span>
                  <span className="block text-xs text-muted-foreground">另一个官方账号，或手动填写第三方接口</span>
                </button>
              </div>
            </details>
          )}
        </div>
        <p className="text-xs text-muted-foreground">
          {name} 的每个环境各有自己的配置、登录与会话，可以同时开着；在终端里输入环境的命令启动它。
        </p>

        <article data-testid="default-environment-card" className="flex flex-col gap-4 rounded-xl border bg-card p-4 shadow-sm sm:p-5">
        <div className="flex flex-wrap items-center justify-between gap-3 rounded-xl border border-primary/20 bg-primary/5 px-4 py-3">
          <div>
            <div className="flex flex-wrap items-center gap-2">
              <span className="rounded-full bg-primary/10 px-2 py-0.5 text-[11px] font-medium text-primary">默认环境</span>
              <strong className="text-sm font-semibold text-foreground">{name}</strong>
              <span data-testid="default-env-launch" className="text-xs text-muted-foreground">
                终端里直接输入 <code className="font-mono text-foreground">{bin}</code>
              </span>
            </div>
            {envState ? (
              <dl data-testid="default-env-status" className="mt-2 grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-xs">
                <dt className="text-muted-foreground">账号</dt>
                <dd data-testid="default-env-account" className={cn("font-medium", thirdParty ? "text-warning" : "text-foreground")}>
                  {envState.accountText}
                  {thirdParty && "（和官方账号混在主配置里，见下方说明）"}
                </dd>
                <dt className="text-muted-foreground">登录</dt>
                <dd data-testid="default-env-login" className="text-muted-foreground">
                  {thirdParty ? "现在用的是配置里的第三方令牌，不是官方登录。" : ""}
                  {envState.loginNote}
                </dd>
                <dt className="text-muted-foreground">模型</dt>
                <dd data-testid="default-env-model" className="text-foreground">
                  {envState.modelText}
                  {envState.modelSource && <span className="text-muted-foreground"> · {envState.modelSource}</span>}
                </dd>
                <dt className="text-muted-foreground">自动化</dt>
                <dd data-testid="default-env-automation" className="text-foreground">
                  {automationLabel}
                  <span className="text-muted-foreground"> · 在「运行设置」里改</span>
                </dd>
              </dl>
            ) : (
              <p className="mt-1 text-xs text-muted-foreground">当前接入：{summary} · 自动化：{automationLabel}。</p>
            )}
            <p className="mt-2 text-xs text-muted-foreground">
              以 {configFile} 的实际内容为准；下面三个页签都属于默认环境。
            </p>
            {writeBlocked && (
              <p data-testid="configure-blocked" className="mt-1 text-xs text-warning">{writeBlocked}</p>
            )}
          </div>
        </div>

        {kind && (
          <div className="rounded-lg border">
            <button
              type="button"
              data-testid="agent-edit-config-file"
              aria-expanded={fileOpen}
              onClick={() => setFileOpen((open) => !open)}
              className="flex w-full flex-wrap items-center gap-2 px-3 py-2.5 text-left text-sm text-foreground hover:bg-accent/50"
            >
              {fileOpen ? <ChevronDown className="h-4 w-4" aria-hidden /> : <ChevronRight className="h-4 w-4" aria-hidden />}
              <span className="font-medium">查看实际配置文件</span>
              <span className="text-xs text-muted-foreground">原文可能含凭据</span>
              <code className="min-w-0 break-all text-xs text-muted-foreground">{currentConfig?.target ?? currentConfig?.file ?? ""}</code>
            </button>
            {fileOpen && <ConfigFileEditor key={agentId} agentId={agentId} />}
          </div>
        )}

        <div aria-label="环境配置" role="tablist" className="flex gap-1 overflow-x-auto border-b">
          {([
            ["providers", "账号与登录"],
            ["automation", "运行设置"],
            ["launch", "启动命令"],
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

        {fatalIssues.length > 0 && (
          <div
            role="alert"
            data-testid="codex-fatal-issues"
            className="rounded-lg border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive"
          >
            <p className="font-medium">Codex 会拒绝加载这份配置，所有 codex 命令（包括登录）都会直接报错（按 Codex 0.156 起的规则）：</p>
            <ul className="mt-1 list-disc space-y-0.5 pl-5 text-xs">
              {fatalIssues.map((issue) => <li key={issue}>{issue}</li>)}
            </ul>
            <p className="mt-1 text-xs">在上方展开「查看实际配置文件」改掉这些行。</p>
          </div>
        )}
        {tab === "automation" && (
          <div data-testid="automation-panel" className="space-y-4 rounded-xl border px-4 py-4">
            {defaultPermissions && (
              <p data-testid="codex-default-permissions" className="rounded-lg border border-warning/30 bg-warning/10 px-3 py-2 text-xs text-warning">
                config.toml 设了 default_permissions = {defaultPermissions}（新的权限写法），Codex 的沙箱以它为准；
                下面自动化档位里的沙箱部分不再生效，只有审批方式照常生效。
              </p>
            )}
            <p className="text-xs text-muted-foreground">
              以客户端配置文件里的实际值为准，只写你改动的那几项；账号与登录在「账号与登录」里看，第三方接入在下方建成独立环境使用。启动命令若带权限参数，会在启动时覆盖这里的自动化设置。
            </p>
            {legacyStatusline && (
              <div className="flex flex-wrap items-center justify-between gap-2 rounded-lg border border-warning/30 bg-warning/10 px-3 py-2">
                <p data-testid="statusline-legacy-notice" className="text-xs text-warning">
                  本工具的状态栏还是旧格式，数据目录含空格时无法运行；修复一次即可。
                </p>
                <button type="button" data-testid="agent-statusline-fix" disabled={!kind || busyTarget !== null || Boolean(writeBlocked)} onClick={() => onFixStatusline(selected)} className="rounded-lg bg-primary px-3 py-1.5 text-xs font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50">
                  修复状态栏
                </button>
              </div>
            )}
            {envOverrides.length > 0 && (
              <div className="flex flex-wrap items-center justify-between gap-2 rounded-lg border border-warning/30 bg-warning/10 px-3 py-2">
                <p data-testid="env-overrides-notice" className="text-xs text-warning">
                  settings.json 的 env 里有单次会话覆盖变量（{envOverrides.map(([name, value]) => `${name}=${value}`).join("，")}），
                  它们优先于这里和 /effort 保存的所有设置，是旧版本写进去的。清理后以正常设置为准。
                </p>
                <button type="button" data-testid="agent-env-overrides-clear" disabled={!kind || busyTarget !== null || Boolean(writeBlocked)} onClick={() => onClearEnvOverrides(selected)} className="rounded-lg border border-warning/40 px-3 py-1.5 text-xs font-medium text-warning hover:bg-warning/10 disabled:opacity-50">
                  清理覆盖变量
                </button>
              </div>
            )}
            {modelEfforts.length > 0 && (
              <p data-testid="model-efforts" className="text-xs text-muted-foreground">
                按模型保存的推理强度（Claude Code 里用 /effort 保存，优先于默认推理强度）：
                {modelEfforts.map(([model, level]) => `${model} = ${level}`).join("，")}
              </p>
            )}
            {kind && management && !writeBlocked && (
              <AgentRuntimePanel
                key={`${agentId}:${configureBaseOf(selected)}`}
                agentId={agentId}
                adapter={management.adapter}
                fields={runtimeFields}
                initial={actualRuntimeSettings(runtimeFields, currentConfig)}
                currentConfig={currentConfig}
                busy={runtimeSaving}
                onSubmit={(patch, fileBase) => onSaveRuntime(selected, patch, fileBase)}
              />
            )}
            {kind && writeBlocked && (
              <p className="text-sm text-warning">{writeBlocked}；可以展开上方的「查看实际配置文件」修好它。</p>
            )}
          </div>
        )}

        {tab === "launch" && <div data-testid={`agent-launch-commands-${agentId}`} className="rounded-xl border px-4 py-3">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <div>
              <h3 className="flex items-center gap-1.5 text-sm font-medium text-foreground">
                <Terminal className="h-4 w-4" aria-hidden /> 运行别名
              </h3>
              <p className="mt-0.5 text-xs text-muted-foreground">以 ~/.zshrc 与 ~/.zprofile 里本工具那一段为准；保存即写入，新开终端窗口生效。</p>
            </div>
            <div className="flex items-center gap-2">
              <button type="button" data-testid={`agent-add-launch-${agentId}`} onClick={() => onAddLaunch(selected)} className="rounded border px-2.5 py-1.5 text-xs hover:bg-accent">
                新增别名
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
                  <pre className="mt-1 overflow-x-auto whitespace-pre-wrap break-all font-mono text-[11px] text-muted-foreground">{renderAliasPreview(name, value)}</pre>
                  {override && <p className="mt-1 text-xs text-warning">{override}</p>}
                </li>
                );
              })}
            </ul>
          )}
        </div>}

        {!kind && (
          <p className="rounded-lg border border-warning/30 bg-warning/10 px-3 py-2 text-sm text-warning">
            这个助手暂不支持在应用内切换接入或写入运行设置；已有配置仅只读展示。
          </p>
        )}

        {tab === "providers" && overrides.length > 0 && (
          <div
            role="status"
            data-testid="main-credential-overrides"
            className="rounded-lg border border-warning/30 bg-warning/10 px-4 py-3 text-sm text-warning"
          >
            <p className="font-medium">这些外部设置会压过或限定主配置里的官方账号：</p>
            <ul className="mt-1 list-disc space-y-0.5 pl-5 text-xs">
              {overrides.map((item) => (
                <li key={`${item.source}/${item.name}`}>
                  <span className="font-mono">{item.source}</span> 里的 <span className="font-mono">{item.name}</span>：
                  {overrideEffect(item.name, "main")}
                </li>
              ))}
            </ul>
            <p className="mt-1 text-xs">只提示，不会改这些文件；不需要时请自己删除或注释掉，再重新打开终端。</p>
          </div>
        )}
        {tab === "providers" && thirdParty && (
          <p
            role="status"
            data-testid="main-mixed-warning"
            className="rounded-lg border border-warning/30 bg-warning/10 px-4 py-3 text-sm text-warning"
          >
            主配置里有第三方接入，和官方账号用的是同一个配置目录：{name} 现在把请求发往第三方，用的是配置里的令牌。
            点「官方账号」上的「切回官方账号」（会先让你确认改动范围），这份第三方配置会先保存为接入；要继续用它，
            在接入卡片上点「建成独立环境」。
          </p>
        )}
        {tab === "providers" && <div className="flex flex-col gap-3">
            {unsaved && currentConfig?.config && (
              <LiveConfigCard
                agentId={agentId}
                config={currentConfig.config}
                driftedLabel={driftedEntry ? driftedEntry.label || driftedEntry.name : null}
                unverified={(current?.unverified ?? []).map((item) => connections.find((entry) => entry.name === item)?.label || item)}
                busy={busyTarget === `${agentId}/save-current`}
                disabled={busyTarget !== null || Boolean(writeBlocked)}
                onSave={kind ? () => onSaveCurrent(selected) : undefined}
              />
            )}
            {mainThirdParty && renderCard(mainThirdParty, true)}
            <ConnectionCard
              agentId={agentId}
              connection={null}
              active={official}
              officialDetail={official
                ? `正在使用 · 模型 ${envState?.modelText ?? "助手默认"}；登录由助手自己管理，本工具不读取`
                : "官方登录没有被动过；切回后助手改用它"}
              busy={busyTarget === `${agentId}/default`}
              disabled={busyTarget !== null || Boolean(writeBlocked)}
              onReset={kind && thirdParty ? () => onResetDefault(selected) : undefined}
            />
            {kind && onCheckAuth && (
              <OfficialAuthPanel
                key={`auth-${agentId}`}
                kind={kind}
                bin={bin}
                testIds={{
                  root: `main-auth-status-${agentId}`,
                  text: `main-auth-text-${agentId}`,
                  check: `main-auth-check-${agentId}`,
                  mixed: `main-auth-mixed-${agentId}`,
                  commands: `main-${agentId}`,
                }}
                onCheck={() => onCheckAuth(selected)}
              />
            )}
        </div>}
        </article>

        {renderInstances?.(selected)}

        <section data-testid={`third-party-section-${agentId}`} className="flex flex-col gap-3 rounded-xl border bg-card p-4 shadow-sm sm:p-5">
          <div className="flex flex-wrap items-start justify-between gap-3">
            <div>
              <h3 className="text-base font-semibold text-foreground">第三方接入 · 用来建独立环境</h3>
              <p className="mt-0.5 text-xs text-muted-foreground">
                这里只保存接入参数（地址、模型、密钥变量名），不会写进默认环境；建成独立环境后才能用。编辑时可逐项选择更新已建独立环境，默认不改。
              </p>
            </div>
            <div className="flex items-center gap-2">
              {connections.length > 1 && (
                <button
                  type="button"
                  data-testid="check-all-connections-button"
                  disabled={!kind || checkingAll || connectionCheckStates.some((item) => item.result?.pending)}
                  onClick={() => onCheckAll(selected, connections)}
                  title="检查全部接入"
                  aria-label="检查全部接入"
                  className="inline-flex h-9 items-center justify-center gap-1.5 rounded-lg border px-3 text-sm text-muted-foreground hover:bg-accent disabled:opacity-50"
                >
                  {checkingAll ? <Loader2 className="h-4 w-4 animate-spin" aria-hidden /> : <Activity className="h-4 w-4" aria-hidden />}
                  {checkingAll ? "检查中…" : "检查全部"}
                </button>
              )}
              <button
                type="button"
                data-testid="import-connection-button"
                disabled={!kind}
                onClick={() => onImport(selected)}
                className="inline-flex h-9 items-center gap-1.5 rounded-lg border px-3 text-sm text-muted-foreground hover:bg-accent disabled:opacity-50"
              >
                <Upload className="h-4 w-4" aria-hidden /> 导入接入
              </button>
              <button
                type="button"
                data-testid="add-connection-button"
                disabled={!kind}
                onClick={() => onAdd(selected)}
                className="inline-flex h-9 items-center rounded-lg border border-primary/40 px-3 text-sm font-medium text-primary hover:bg-primary/10 disabled:cursor-not-allowed disabled:opacity-50"
              >
                添加接入
              </button>
            </div>
          </div>
        {connections.length > 1 && connectionChecks.length > 0 && (
          <div
            data-testid="connection-check-summary"
            className="flex flex-wrap items-center justify-between gap-2 rounded-lg border bg-muted/30 px-3 py-2 text-sm"
          >
            <p className="text-muted-foreground">
              接入检查：已完成 {connectionChecks.length}/{connections.length}，
              <strong className="font-medium text-foreground">
                {connectionChecks.filter((item) => item.result.ok).length} 个可用
              </strong>
            </p>
            {allChecked && fastest && (
              <div className="flex flex-wrap items-center gap-2">
                <p className="text-success">
                  最快：{fastest.connection.label || fastest.connection.name} {fastest.result.latencyMs}ms
                </p>
              </div>
            )}
          </div>
        )}

          {savedEntries.length === 0 ? (
            <p data-testid={`third-party-empty-${agentId}`} className="rounded-lg border border-dashed px-4 py-3 text-xs text-muted-foreground">
              还没有保存第三方接入。用右上角「新建环境 → 第三方厂商或中转」一步建好，或在这里「添加接入」先存起来。
            </p>
          ) : (
            savedEntries.map((connection) => renderCard(connection, false))
          )}
        </section>
    </section>
  );
}

/** 实际配置对不上任何已保存接入时的虚拟卡片：展示实际值，可存成接入配置。 */
function LiveConfigCard({
  agentId,
  config,
  driftedLabel,
  unverified,
  busy,
  disabled,
  onSave,
}: {
  agentId: string;
  config: AgentCurrentConfig;
  driftedLabel: string | null;
  unverified: string[];
  busy: boolean;
  disabled: boolean;
  onSave?: () => void;
}) {
  const tokenUnsaved = Boolean(
    config.credential?.configured
    && (config.credential.kind === "token" || config.credential.kind === "api_key")
    && !config.credential.saved_var,
  );
  return (
    <article
      data-testid={`connection-card-${agentId}-live`}
      className="flex min-h-24 items-center gap-4 rounded-2xl border border-success/40 bg-success/5 px-5 py-4 shadow-sm"
    >
      <div className="flex h-12 w-12 shrink-0 items-center justify-center rounded-xl border bg-muted text-lg font-semibold text-foreground">实</div>
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-2">
          <h3 className="text-lg font-semibold text-foreground">当前实际配置（未保存）</h3>
          <span className="inline-flex items-center gap-1 rounded-full bg-success/15 px-2.5 py-1 text-xs font-medium text-success">
            <Check className="h-3 w-3" aria-hidden />当前使用
          </span>
        </div>
        <p className="mt-1 truncate text-sm text-primary">
          {config.base_url || (config.provider && config.provider !== "anthropic" && config.provider !== "openai" ? `接入方式 ${config.provider}` : "助手自己的默认地址")}
          {config.model ? ` · ${config.model}` : ""}
        </p>
        <p data-testid={`connection-live-hint-${agentId}`} className="mt-1 text-xs text-muted-foreground">
          {driftedLabel
            ? `与「${driftedLabel}」地址相同，外部改了模型等参数；切换或保存时会更新「${driftedLabel}」。`
            : "还没有保存为接入配置；切回官方账号前会先自动保存，不会丢。"}
          {tokenUnsaved ? " 令牌还没有保存为变量，切换前需要选择密钥文件或确认放弃。" : ""}
          {unverified.length > 0 ? ` 与「${unverified.join("」「")}」地址和模型一致，但其令牌变量在本机取不到值，无法核对。` : ""}
        </p>
        {isPlainHttpUrl(config.base_url ?? "") && (
          <p className="mt-1 text-xs text-warning">非 https 地址：令牌将明文传输。</p>
        )}
      </div>
      {onSave && (
        <button
          type="button"
          data-testid={`save-live-connection-${agentId}`}
          disabled={disabled}
          onClick={onSave}
          className="inline-flex shrink-0 items-center gap-1 rounded-lg border px-3 py-2 text-xs font-medium text-foreground hover:bg-accent disabled:opacity-50"
        >
          {busy ? <Loader2 className="h-3.5 w-3.5 animate-spin" aria-hidden /> : <Save className="h-3.5 w-3.5" aria-hidden />}
          {busy ? "正在保存…" : driftedLabel ? `保存到「${driftedLabel}」` : "保存为接入配置"}
        </button>
      )}
    </article>
  );
}

function ConnectionCard({
  agentId,
  connection,
  active,
  busy,
  disabled,
  check,
  onReset,
  onCheck,
  onEdit,
  onDuplicate,
  onMakeProfile,
  onExport,
  onRemove,
  onMoveUp,
  onMoveDown,
  instances = [],
  onShowInstances,
  onBalance,
  officialDetail,
}: {
  agentId: string;
  connection: ConnectionEntry | null;
  /** 官方账号卡片的说明：正在用时写模型，没在用时说明登录没被动过。 */
  officialDetail?: string;
  active: boolean;
  busy: boolean;
  disabled: boolean;
  check?: ConnectionCheckResult;
  onReset?: () => void;
  onCheck?: () => void;
  onEdit?: () => void;
  onDuplicate?: () => void;
  onMakeProfile?: () => void;
  onExport?: () => void;
  onRemove?: () => void;
  onMoveUp?: () => void;
  onMoveDown?: () => void;
  instances?: { id: string; label: string }[];
  onShowInstances?: (id: string) => void;
  onBalance?: () => Promise<string>;
}) {
  const id = connection?.name ?? "default";
  const [balance, setBalance] = useState<string | null>(null);
  const [balanceBusy, setBalanceBusy] = useState(false);
  return (
    <article
      data-testid={`connection-card-${agentId}-${id}`}
      onClick={(event) => {
        if (onEdit && !(event.target as HTMLElement).closest("button, details, summary")) onEdit();
      }}
      className={cn(
        "flex min-h-24 flex-wrap items-center gap-4 rounded-2xl border bg-background px-5 py-4 shadow-sm transition-colors",
        onEdit && "cursor-pointer",
        active ? "border-success/40 bg-success/5" : "hover:border-primary/30",
      )}
    >
      <div className="flex h-12 w-12 shrink-0 items-center justify-center rounded-xl border bg-muted text-lg font-semibold text-foreground">
        {(connection?.label || connection?.name || "官").slice(0, 1).toUpperCase()}
      </div>
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-2">
          <h3 className="text-lg font-semibold text-foreground">
            {connection ? connection.label || connection.name : "官方账号"}
          </h3>
          {active && (
            <span
              className={cn(
                "inline-flex items-center gap-1 rounded-full px-2.5 py-1 text-xs font-medium",
                connection ? "bg-warning/15 text-warning" : "bg-success/15 text-success",
              )}
            >
              <Check className="h-3 w-3" aria-hidden />
              当前使用
            </span>
          )}
        </div>
        <p className="mt-1 truncate text-sm text-primary">
          {connection?.providerRef
            ? `引用 config.toml 中的接入方式 ${connection.providerRef}`
            : connection?.baseUrl || officialDetail || "使用助手自己的登录与默认地址"}
        </p>
        {connection && isPlainHttpUrl(connection.baseUrl) && (
          <p className="mt-1 text-xs text-warning">非 https 地址：令牌将明文传输。</p>
        )}
        {connection?.note && (
          <p data-testid={`connection-note-${agentId}-${id}`} className="mt-1 truncate text-xs text-muted-foreground">
            {connection.note}
          </p>
        )}
        {connection && instances.length > 0 && (
          <p data-testid={`connection-instances-${agentId}-${id}`} className="mt-1 text-xs text-muted-foreground">
            已建成独立环境：
            {instances.map((instance, index) => (
              <span key={instance.id}>
                {index > 0 && "、"}
                <button
                  type="button"
                  onClick={() => onShowInstances?.(instance.id)}
                  className="font-mono text-primary hover:underline"
                  title={`查看独立环境 ${instance.label}`}
                >
                  {instance.id}
                </button>
              </span>
            ))}
          </p>
        )}
        {balance && (
          <p data-testid={`connection-balance-${agentId}-${id}`} className="mt-1 text-xs text-muted-foreground">
            {balance}
          </p>
        )}
      </div>
      <div className="flex min-w-0 flex-[1_1_240px] flex-wrap items-center justify-end gap-2">
        {check && !check.pending && (
          <span
            data-testid={`check-result-${agentId}-${id}`}
            className={cn("text-xs", check.ok ? "text-success" : "text-destructive")}
          >
            {check.ok && check.latencyMs > 0 ? `${check.latencyMs}ms` : check.detail}
          </span>
        )}
        {onCheck && (
          <button
            type="button"
            data-testid={`check-connection-${agentId}-${id}`}
            disabled={check?.pending}
            title="发送一次最小真实请求检查接入；只在点击时联网，不改配置"
            onClick={onCheck}
            className="inline-flex shrink-0 items-center gap-1 rounded-lg border px-3 py-2 text-xs text-muted-foreground hover:bg-accent disabled:opacity-50"
          >
            {check?.pending ? "正在测试…" : "测试连接"}
          </button>
        )}
        {onBalance && (
          <button
            type="button"
            data-testid={`connection-balance-button-${agentId}-${id}`}
            disabled={balanceBusy}
            title="用这套接入的凭据查账户余额；只在点它时联网，不改配置"
            onClick={async () => {
              setBalanceBusy(true);
              try {
                setBalance(await onBalance());
              } finally {
                setBalanceBusy(false);
              }
            }}
            className="inline-flex shrink-0 items-center gap-1 rounded-lg border px-3 py-2 text-xs text-muted-foreground hover:bg-accent disabled:opacity-50"
          >
            {balanceBusy ? "正在查…" : "查余额"}
          </button>
        )}
        {connection && instances.length === 0 && onMakeProfile && !connection.providerRef && (
          <button
            type="button"
            data-testid={`make-profile-quick-${agentId}-${id}`}
            disabled={disabled}
            onClick={onMakeProfile}
            className="inline-flex shrink-0 items-center gap-1 rounded-lg border border-primary/40 px-3 py-2 text-xs font-medium text-primary hover:bg-primary/10 disabled:opacity-50"
          >
            建成独立环境
          </button>
        )}
        {onReset && (
          <button
            type="button"
            data-testid={`reset-connection-${agentId}-${id}`}
            disabled={disabled}
            onClick={onReset}
            className="inline-flex shrink-0 items-center gap-1 rounded-lg bg-primary px-3 py-2 text-xs font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
          >
            {busy && <Loader2 className="h-3.5 w-3.5 animate-spin" aria-hidden />}
            {busy ? "正在切回…" : "切回官方账号"}
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
        {connection && (onDuplicate || onMakeProfile || onExport || onRemove || onMoveUp || onMoveDown) && (
          <details className="group relative">
            <summary aria-label={`管理 ${connection.label || connection.name}`} className="inline-flex h-9 w-9 cursor-pointer list-none items-center justify-center rounded-lg border text-muted-foreground hover:bg-accent">
              <MoreHorizontal className="h-4 w-4" aria-hidden />
            </summary>
            <div className="absolute right-0 top-full z-10 mt-1 flex w-44 flex-col rounded-lg border bg-popover p-1 shadow-lg">
            {(onMoveUp || onMoveDown) && <div className="flex items-center" aria-label="调整接入顺序">
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
            </div>}
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
            {onMakeProfile && connection && (instances.length > 0 || Boolean(connection.providerRef)) && (
              <button
                type="button"
                data-testid={`make-profile-${agentId}-${id}`}
                disabled={disabled || Boolean(connection.providerRef)}
                title={connection.providerRef
                  ? "这套接入引用主配置 config.toml 里的接入方式，没法搬进独立目录"
                  : "新建一个独立环境使用这套接入，官方账号留在主配置"}
                onClick={(event) => runMenuAction(event, onMakeProfile)}
                className="rounded px-3 py-2 text-left text-xs text-muted-foreground hover:bg-accent disabled:cursor-not-allowed disabled:opacity-50"
              >
                建成独立环境
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
