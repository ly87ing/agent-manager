import { useState, type MouseEvent } from "react";
import { Activity, ArrowDown, ArrowUp, Check, ChevronDown, ChevronRight, Download, Loader2, MoreHorizontal, Pencil, Save, Settings2, Terminal, Trash2, Upload } from "lucide-react";
import { AgentRuntimePanel } from "./AgentRuntimePanel";
import { ConfigFileEditor } from "./ConfigFileEditor";
import type { EngineElement } from "../lib/contract";
import type { AgentCurrentConfig, AgentsObserved, AgentsSpec } from "../lib/observed";
import {
  isPlainHttpUrl,
  parseInitialToFormValues,
  type ConnectionEntry,
} from "../lib/agentDefinitionForm";
import { connectionKindOf } from "../lib/agentManagement";
import { configureBaseOf, currentConfigOf, currentConnectionOf, hasUnsavedLive } from "../lib/agentCurrent";
import { actualRuntimeSettings, automationRawText, runtimeValueLabel } from "../lib/agentRuntime";
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

/** 各外部设置对主配置官方账号的影响（依据官方认证优先级文档、设置参考与 Codex 源码）。 */
const OVERRIDE_EFFECT: Record<string, string> = {
  ANTHROPIC_AUTH_TOKEN: "Claude 会优先用这个令牌，不用官方账号",
  ANTHROPIC_API_KEY: "Claude 可能改用这个 API key（-p 模式直接使用，交互模式确认后使用）",
  ANTHROPIC_BASE_URL: "Claude 会把请求连同官方账号的登录凭据发往这个地址",
  CLAUDE_CODE_OAUTH_TOKEN: "Claude 会用这个长期令牌，不用主配置里的登录",
  CLAUDE_CODE_USE_BEDROCK: "Claude 改走 Amazon Bedrock，不用官方账号",
  CLAUDE_CODE_USE_VERTEX: "Claude 改走 Google Vertex，不用官方账号",
  CLAUDE_CODE_USE_FOUNDRY: "Claude 改走 Microsoft Foundry，不用官方账号",
  CLAUDE_CODE_USE_ANTHROPIC_AWS: "Claude 改走 Claude Platform on AWS，不用官方账号",
  CLAUDE_CODE_USE_MANTLE: "Claude 改走 Amazon Bedrock Mantle，不用官方账号",
  ANTHROPIC_PROFILE: "Claude 会用这个 Anthropic 配置档案的凭据，优先于 /login 登录",
  ANTHROPIC_FEDERATION_RULE_ID: "Claude 会用工作负载身份联合换取的令牌，优先于 /login 登录",
  ANTHROPIC_CUSTOM_HEADERS: "每个请求都会附加这些请求头，其中可能有凭据头",
  oidc_federation: "活动的 Anthropic 配置档案是联合身份，Claude 会优先用它换取的令牌，不用 /login 登录",
  apiKeyHelper: "Claude 会优先用这条命令输出的凭据，不用官方账号",
  forceLoginMethod: "组织统一下发的设置限定了登录方式；用第三方凭据的独立环境会在启动时被拦下",
  forceLoginOrgUUID: "组织统一下发的设置限定了可登录的组织；用第三方凭据的独立环境会在启动时被拦下",
  CODEX_API_KEY: "codex exec 等非交互命令会优先用这个 API key，不用 auth.json 里的登录（交互界面不读它）",
  CODEX_ACCESS_TOKEN: "Codex 所有入口都会优先用这个访问令牌，不用 auth.json 里的登录",
  openai_base_url: "Codex 会把官方账号的凭据连同请求发往这个地址",
};

export function ConnectionWorkbench({
  agents,
  selectedAgentId,
  busyTarget,
  checkingAllAgentId,
  checkResults,
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
}: {
  agents: EngineElement[];
  selectedAgentId: string | null;
  busyTarget: string | null;
  checkingAllAgentId: string | null;
  checkResults: Record<string, ConnectionCheckResult>;
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
  onExport: (element: EngineElement, connection: ConnectionEntry) => void;
  onRemove: (element: EngineElement, connection: ConnectionEntry) => void;
  onMove: (element: EngineElement, connection: ConnectionEntry, direction: "up" | "down") => void;
  runtimeSaving: boolean;
  /** 运行设置只发改动过的字段；撤掉的项为 null。 */
  onSaveRuntime: (element: EngineElement, patch: Record<string, unknown>) => void;
  /** 把「当前实际配置（未保存）」存成接入配置，不写助手文件。 */
  onSaveCurrent: (element: EngineElement) => void;
  /** 旧格式状态栏：按引用形式重写一次。 */
  onFixStatusline: (element: EngineElement) => void;
  /** 清理旧版本写进 env 的单次会话覆盖变量（用户点按钮才执行）。 */
  onClearEnvOverrides: (element: EngineElement) => void;
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
  const form = parseInitialToFormValues(agentId, selected.spec as Record<string, unknown>);
  // 当前项以实际配置为准：引擎按内容识别，库里的 active 只是上次写入记录
  const current = currentConnectionOf(selected);
  const active = current?.profile ?? "";
  const activeEntry = form.connections.find((item) => item.name === active);
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
  const driftedEntry = current?.drifted_from
    ? form.connections.find((item) => item.name === current.drifted_from)
    : undefined;
  const summary = activeEntry
    ? activeEntry.label || activeEntry.name
    : current?.default
      ? "官方账号"
      : unsaved
        ? "当前实际配置（未保存）"
        : "无法读取实际配置";
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
  // 当前那张排在最前：已保存的当前项、默认账号，或虚拟的「当前实际配置（未保存）」
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
            {summary}
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
              当前接入：{summary} · 自动化：{automationLabel}。以客户端实际配置为准，下面所有页签都属于这一个环境。
            </p>
            {writeBlocked && (
              <p data-testid="configure-blocked" className="mt-1 text-xs text-warning">{writeBlocked}</p>
            )}
          </div>
        </div>

        <div aria-label="环境配置" role="tablist" className="flex gap-1 overflow-x-auto border-b">
          {([
            ["providers", "接入 Provider"],
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

        {tab === "automation" && (
          <div data-testid="automation-panel" className="space-y-4 rounded-xl border px-4 py-4">
            <p className="text-xs text-muted-foreground">
              以客户端配置文件里的实际值为准，只写你改动的那几项；账号、接口地址、模型与凭据在「接入 Provider」维护。启动命令若带权限参数，会在启动时覆盖这里的自动化设置。
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
                adapter={management.adapter}
                fields={runtimeFields}
                initial={actualRuntimeSettings(runtimeFields, currentConfig)}
                currentConfig={currentConfig}
                busy={runtimeSaving}
                onSubmit={(patch) => onSaveRuntime(selected, patch)}
              />
            )}
            {kind && writeBlocked && (
              <p className="text-sm text-warning">{writeBlocked}；可以在下方直接编辑配置文件修好它。</p>
            )}
            {kind && (
              <div className="rounded-lg border">
                <button
                  type="button"
                  data-testid="agent-edit-config-file"
                  aria-expanded={fileOpen}
                  onClick={() => setFileOpen((open) => !open)}
                  className="flex w-full items-center gap-2 px-3 py-2.5 text-left text-sm text-foreground hover:bg-accent/50"
                >
                  {fileOpen ? <ChevronDown className="h-4 w-4" aria-hidden /> : <ChevronRight className="h-4 w-4" aria-hidden />}
                  <span className="font-medium">直接编辑配置文件</span>
                  <code className="min-w-0 truncate text-xs text-muted-foreground">{currentConfig?.target ?? currentConfig?.file ?? ""}</code>
                </button>
                {fileOpen && <ConfigFileEditor key={agentId} agentId={agentId} />}
              </div>
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
              </div>
            )}
          </div>
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
                  {OVERRIDE_EFFECT[item.name] ?? "会改变助手使用的凭据"}
                </li>
              ))}
            </ul>
            <p className="mt-1 text-xs">只提示，不会改这些文件；不需要时请自己删除或注释掉，再重新打开终端。</p>
          </div>
        )}
        {tab === "providers" && current && !current.default && (
          <p
            role="status"
            data-testid="main-mixed-warning"
            className="rounded-lg border border-warning/30 bg-warning/10 px-4 py-3 text-sm text-warning"
          >
            主配置里有第三方接入，和官方账号用的是同一个配置目录。点「官方账号」卡片上的「切回官方账号」，
            这份第三方配置会先保存为接入；要继续用它，在它的菜单里选「建成独立环境」。
          </p>
        )}
        {tab === "providers" && <div className="flex flex-col gap-3">
            {unsaved && currentConfig?.config && (
              <LiveConfigCard
                agentId={agentId}
                config={currentConfig.config}
                driftedLabel={driftedEntry ? driftedEntry.label || driftedEntry.name : null}
                unverified={(current?.unverified ?? []).map((item) => form.connections.find((entry) => entry.name === item)?.label || item)}
                busy={busyTarget === `${agentId}/save-current`}
                disabled={busyTarget !== null || Boolean(writeBlocked)}
                onSave={kind ? () => onSaveCurrent(selected) : undefined}
              />
            )}
            {orderedEntries.map((connection) => {
              const connectionName = connection?.name ?? "";
              const key = `${agentId}/${connectionName || "default"}`;
              const index = connection
                ? form.connections.findIndex((item) => item.name === connection.name)
                : -1;
              const isCurrent = connection ? active === connectionName : current?.default === true;
              return (
              <ConnectionCard
                key={connection?.name ?? "default"}
                agentId={agentId}
                connection={connection}
                active={isCurrent}
                busy={busyTarget === key}
                disabled={busyTarget !== null || Boolean(writeBlocked)}
                check={checkResults[key]}
                onReset={kind && !connection && !isCurrent ? () => onResetDefault(selected) : undefined}
                onCheck={kind && connection ? () => onCheck(selected, connection.name) : undefined}
                onEdit={kind && connection ? () => onEdit(selected, connection) : undefined}
                onDuplicate={kind && connection ? () => onDuplicate(selected, connection) : undefined}
                onMakeProfile={kind && connection && onMakeProfile ? () => onMakeProfile(selected, connection) : undefined}
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
            : "还没有保存为接入配置；切换到其他接入前会自动保存，不会丢。"}
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
}: {
  agentId: string;
  connection: ConnectionEntry | null;
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
        active ? "border-success/40 bg-success/5" : "hover:border-primary/30",
      )}
    >
      <div className="flex h-12 w-12 shrink-0 items-center justify-center rounded-xl border bg-muted text-lg font-semibold text-foreground">
        {(connection?.label || connection?.name || "默").slice(0, 1).toUpperCase()}
      </div>
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-2">
          <h3 className="text-lg font-semibold text-foreground">
            {connection ? connection.label || connection.name : "官方账号"}
          </h3>
          {active && (
            <span className="inline-flex items-center gap-1 rounded-full bg-success/15 px-2.5 py-1 text-xs font-medium text-success">
              <Check className="h-3 w-3" aria-hidden />
              当前使用
            </span>
          )}
        </div>
        <p className="mt-1 truncate text-sm text-primary">
          {connection?.providerRef
            ? `引用 config.toml 中的接入方式 ${connection.providerRef}`
            : connection?.baseUrl || "使用助手自己的登录与默认地址"}
        </p>
        {connection && isPlainHttpUrl(connection.baseUrl) && (
          <p className="mt-1 text-xs text-warning">非 https 地址：令牌将明文传输。</p>
        )}
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
        {connection && (onCheck || onEdit || onDuplicate || onMakeProfile || onExport || onRemove || onMoveUp || onMoveDown) && (
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
            {onMakeProfile && connection && (
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
