import { Check, CircleAlert, RefreshCw, X } from "lucide-react";
import type { EngineElement } from "../lib/contract";
import { agentCellsOf, type McpObserved, type McpSpec } from "../lib/observed";
import { buildMcpSyncPlan } from "../lib/mcpSync";
import { FreshnessBadge } from "./FreshnessBadge";
import { StateBadge } from "./StateBadge";
import { DriftExplanationPanel } from "./DriftExplanationPanel";
import { McpCellStatusBadge, type McpAgentCellView } from "./McpAssignmentRow";
import { cn } from "../lib/utils";

/** 服务级 observed 扩展：库级操作动作与受阻原因（observed.ts 冻结，本地扩展）。 */
export interface McpServerObserved extends McpObserved {
  actions?: string[];
  blocked_reason?: string | null;
}

function displayPlaceholder(value: string | undefined): string {
  if (!value) return "<明文已隐藏>";
  return value.startsWith("${") ? value : `\${${value}}`;
}

function variableName(name: string, placeholder: string | undefined): string {
  const value = placeholder ?? name;
  const match = /^\$\{([^}]+)\}$/.exec(value);
  return match?.[1] ?? value;
}

function EnvResolutionList({ element }: { element: EngineElement }) {
  const observed = element.observed as McpObserved;
  const env = observed.env ?? {};
  const entries = Object.entries(env);
  if (entries.length === 0) {
    return <p className="text-xs text-muted-foreground">不需要环境变量。</p>;
  }
  return (
    <ul className="space-y-1">
      {entries.map(([name, resolution]) => (
        <li key={name} className="flex items-center gap-2 text-xs">
          {resolution.resolved ? (
            <Check className="h-3.5 w-3.5 text-success" aria-label="已配置" />
          ) : (
            <X className="h-3.5 w-3.5 text-destructive" aria-label="未配置" />
          )}
          <span className="font-mono text-foreground">{name}</span>
          <span className="text-muted-foreground">→</span>
          <code className="font-mono text-foreground">{displayPlaceholder(resolution.placeholder)}</code>
          <span className={resolution.resolved ? "text-success" : "text-destructive"}>
            {resolution.resolved ? "已配置" : "未配置（运行时会显示变量名）"}
          </span>
        </li>
      ))}
    </ul>
  );
}

/** 远程 headers 占位符解析列表（与 EnvResolutionList 平行：只显示是否已配置，永不显示值）。 */
function HeaderResolutionList({ element }: { element: EngineElement }) {
  const observed = element.observed as McpObserved;
  const headers = observed.headers ?? {};
  const entries = Object.entries(headers);
  if (entries.length === 0) {
    return <p className="text-xs text-muted-foreground">不需要请求头。</p>;
  }
  return (
    <ul className="space-y-1">
      {entries.map(([name, resolution]) => (
        <li key={name} className="flex items-center gap-2 text-xs" data-testid="mcp-header-resolution">
          {resolution.resolved ? (
            <Check className="h-3.5 w-3.5 text-success" aria-label="已配置" />
          ) : (
            <X className="h-3.5 w-3.5 text-destructive" aria-label="未配置" />
          )}
          <span className="font-mono text-foreground">{name}</span>
          <span className="text-muted-foreground">→</span>
          <code className="font-mono text-foreground">{displayPlaceholder(resolution.placeholder)}</code>
          <span className={resolution.resolved ? "text-success" : "text-destructive"}>
            {resolution.resolved ? "已配置" : "未配置（运行时会显示变量名）"}
          </span>
        </li>
      ))}
    </ul>
  );
}

/** 服务详情：远程为 服务地址 / 请求头占位符；stdio 为 启动命令 / 附加参数 / 密钥变量（只显示是否已配置，永不显示值）/ 写入位置 / 同步详情。 */
export function McpServerDetail({ element }: { element: EngineElement }) {
  const spec = element.spec as McpSpec;
  const cells = agentCellsOf(element);
  const extraArgs = spec.extra_args_by_agent ?? {};
  const isRemote = typeof spec.url === "string" && spec.url.length > 0;
  return (
    <section data-testid={`mcp-detail-${element.id}`} className="space-y-4 rounded-lg border bg-card p-5 shadow-sm">
      {isRemote ? (
        <div>
          <h4 className="mb-1 text-xs font-semibold text-muted-foreground">服务地址（远程 HTTP）</h4>
          <code className="block overflow-x-auto rounded bg-slate-900 px-3 py-2 font-mono text-xs break-all text-slate-100">
            {spec.url}
          </code>
          <p className="mt-1 text-xs text-muted-foreground">
            应用到: {spec.agents === null || spec.agents === undefined ? "全部助手" : spec.agents.join(", ")}
          </p>
        </div>
      ) : (
        <div className="space-y-3">
          <div>
            <h4 className="mb-1 text-xs font-semibold text-muted-foreground">启动命令 <code>command</code></h4>
            <code className="block overflow-x-auto rounded bg-slate-900 px-3 py-2 font-mono text-xs break-all text-slate-100">
              {spec.command}
            </code>
          </div>
          <div>
            <h4 className="mb-1 text-xs font-semibold text-muted-foreground">参数 <code>args</code></h4>
            <pre className="max-h-48 overflow-auto rounded bg-slate-900 px-3 py-2 font-mono text-xs text-slate-100">
              {JSON.stringify(spec.args ?? [], null, 2)}
            </pre>
          </div>
          <p className="mt-1 text-xs text-muted-foreground">
            启动超时 {spec.startup_timeout_sec ?? "—"} 秒 · 应用到:{" "}
            {spec.agents === null || spec.agents === undefined ? "全部助手" : spec.agents.join(", ")}
          </p>
        </div>
      )}

      {Object.keys(extraArgs).length > 0 && (
        <div>
          <h4 className="mb-1 text-xs font-semibold text-muted-foreground">各助手附加参数</h4>
          <ul className="space-y-1">
            {Object.entries(extraArgs).map(([agent, args]) => (
              <li key={agent} className="text-xs">
                <span className="font-mono text-muted-foreground">{agent}: </span>
                <code className="font-mono text-foreground">{args.join(" ")}</code>
              </li>
            ))}
          </ul>
        </div>
      )}

      {isRemote ? (
        <div>
          <h4 className="mb-1 text-xs font-semibold text-muted-foreground">请求头变量（只显示是否已配置，永不显示值）</h4>
          <HeaderResolutionList element={element} />
        </div>
      ) : (
        <div>
          <h4 className="mb-1 text-xs font-semibold text-muted-foreground">密钥变量（只显示是否已配置，永不显示值）</h4>
          <EnvResolutionList element={element} />
        </div>
      )}

      <div>
        <h4 className="mb-1 text-xs font-semibold text-muted-foreground">写入位置</h4>
        <ul className="space-y-1">
          {Object.entries(cells).map(([agent, cell]) => (
            <li key={agent} className="flex flex-wrap items-center gap-2 text-xs">
              <span className="font-mono text-muted-foreground">{agent}</span>
              <code className="font-mono break-all text-foreground">{cell.target ?? "—"}</code>
              <span className={cell.target_exists ? "text-success" : "text-muted-foreground"}>
                {cell.target_exists ? "已存在" : "未生成"}
              </span>
            </li>
          ))}
        </ul>
      </div>

      <DriftExplanationPanel element={element} cellKind="mcp" />
    </section>
  );
}

/**
 * 服务库卡片：定义/凭据状态展示 + 库级操作。
 * 启停/删除严格按 observed.actions 实际子集渲染，不猜 state；
 * 更新定义走「编辑定义」表单（save-only），不伪装升级服务程序。
 */
export function McpServerCard({
  element,
  selected,
  onSelect,
  onEdit,
  onAction,
  onSync,
  syncing,
  onConfigureSecrets,
}: {
  element: EngineElement;
  selected: boolean;
  onSelect: (id: string | null) => void;
  onEdit: (element: EngineElement) => void;
  onAction: (element: EngineElement, action: "enable" | "disable" | "remove" | "upgrade") => void;
  onSync: (element: EngineElement) => void;
  syncing: boolean;
  onConfigureSecrets?: (variable: string) => void;
}) {
  const spec = element.spec as McpSpec;
  const observed = element.observed as McpServerObserved;
  const actions = observed.actions ?? [];
  const isEnabled = (element.policy as { enabled?: boolean }).enabled !== false;
  const isRemote = typeof spec.url === "string" && spec.url.length > 0;
  const envEntries = Object.entries(observed.env ?? {});
  const headerEntries = Object.entries(observed.headers ?? {});
  const secretEntries = [...envEntries, ...headerEntries];
  const envResolved = secretEntries.filter(([, resolution]) => resolution.resolved).length;
  const missingSecretVariables = Array.from(
    new Set(
      secretEntries
        .filter(([, resolution]) => resolution.resolved === false)
        .map(([name, resolution]) => variableName(name, resolution.placeholder)),
    ),
  );
  const syncPlan = buildMcpSyncPlan(element);
  const syncRows = new Map(syncPlan.rows.map((row) => [row.agent, row]));
  const blockedRows = new Map(syncPlan.blocked.map((row) => [row.agent, row]));
  const issueAgents = new Set([...syncRows.keys(), ...blockedRows.keys()]);
  const resolutionEntries = Object.entries(agentCellsOf(element))
    .filter(([agent]) => issueAgents.has(agent))
    .map(([agent, cell]) => ({ agent, cell: cell as McpAgentCellView }));
  const firstMissingSecret = missingSecretVariables[0];

  return (
    <section
      data-testid={`mcp-card-${element.id}`}
      className={cn("rounded-lg border bg-card p-5 shadow-sm transition-shadow hover:shadow-md", selected ? "border-primary/40" : "border")}
    >
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <button
            type="button"
            onClick={() => onSelect(selected ? null : element.id)}
            aria-expanded={selected}
            className="text-left font-mono text-xs font-semibold break-all text-primary hover:text-primary"
            title="点击展开/收起服务详情"
          >
            {element.id}
          </button>
          <p
            className="mt-1 truncate font-mono text-xs text-muted-foreground"
            title={isRemote ? spec.url : `${spec.command ?? ""} ${(spec.args ?? []).join(" ")}`}
          >
            {isRemote ? spec.url : `${spec.command ?? ""} ${(spec.args ?? []).join(" ")}`}
          </p>
        </div>
        <div className="flex flex-shrink-0 items-center gap-2">
          <StateBadge state={element.state} />
          <FreshnessBadge freshness={element.freshness} className="items-end" />
        </div>
      </div>

      <div className="mt-3 flex flex-wrap gap-1.5 text-xs">
        <span className="rounded border bg-muted px-1.5 py-0.5 text-muted-foreground">
          {spec.agents === null || spec.agents === undefined ? "全部助手" : `限定 ${spec.agents.length} 个助手`}
        </span>
        {isRemote && (
          <span className="rounded border border-info/30 bg-info/10 px-1.5 py-0.5 text-info">远程 HTTP</span>
        )}
        <span
          className={cn(
            "rounded border px-1.5 py-0.5",
            secretEntries.length === 0 || envResolved === secretEntries.length
              ? "border bg-muted text-muted-foreground"
              : "border-warning/30 bg-warning/10 text-warning",
          )}
        >
          {isRemote ? `请求头 ${envResolved}/${secretEntries.length} 已配置` : `密钥变量 ${envResolved}/${secretEntries.length} 已配置`}
        </span>
        {firstMissingSecret && onConfigureSecrets && (
          <button
            type="button"
            data-testid={`configure-secrets-${element.id}`}
            onClick={() => onConfigureSecrets(firstMissingSecret)}
            className="rounded border border-primary/30 bg-primary/10 px-1.5 py-0.5 font-medium text-primary hover:bg-primary/15"
          >
            填写缺失变量{missingSecretVariables.length > 1 ? `（${missingSecretVariables.length}）` : ""}
          </button>
        )}
      </div>

      {selected && <div className="mt-4"><McpServerDetail element={element} /></div>}

      {observed.command_available === false && (
        <div
          data-testid={`command-missing-${element.id}`}
          className="mt-3 rounded-md border border-warning/30 bg-warning/10 px-3 py-2 text-xs text-warning"
        >
          本机找不到它要执行的命令，助手启动这个服务时会失败。先装好该命令，或改成已装好的命令。
        </div>
      )}

      {observed.blocked_reason && (
        <div className="mt-3 rounded-md border border-warning/30 bg-warning/10 px-3 py-2 text-xs text-warning">
          受阻原因：{observed.blocked_reason}
        </div>
      )}

      {resolutionEntries.length > 0 && (
        <div
          data-testid={`mcp-resolution-${element.id}`}
          className="mt-3 rounded-md border border-warning/30 bg-warning/5 px-3 py-2.5"
        >
          <div className="flex flex-wrap items-center justify-between gap-2">
            <div className="flex items-center gap-1.5 text-xs font-semibold text-foreground">
              <CircleAlert className="h-3.5 w-3.5 text-warning" aria-hidden />
              {syncPlan.rows.length > 0 ? `${syncPlan.rows.length} 个助手待同步` : "暂时无法自动同步"}
              {syncPlan.blocked.length > 0 && <span className="font-normal text-warning">· {syncPlan.blocked.length} 个需先处理</span>}
            </div>
            <div className="flex items-center gap-2">
              {syncPlan.rows.length > 0 && (
                <button
                  type="button"
                  data-testid={`sync-server-${element.id}`}
                  onClick={() => onSync(element)}
                  disabled={syncing}
                  className="inline-flex items-center gap-1 rounded border border-primary/30 bg-primary px-2 py-0.5 text-xs font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
                >
                  <RefreshCw className={cn("h-3 w-3", syncing && "animate-spin")} aria-hidden />
                  一键同步
                </button>
              )}
            </div>
          </div>
          <ul className="mt-2 divide-y divide-border/70">
            {resolutionEntries.map(({ agent, cell }) => {
              const syncRow = syncRows.get(agent);
              const blockedRow = blockedRows.get(agent);
              return (
                <li
                  key={agent}
                  data-testid={`mcp-resolution-${element.id}-${agent}`}
                  className="flex flex-wrap items-center gap-x-2 gap-y-1.5 py-2 first:pt-0 last:pb-0"
                >
                  <span className="min-w-24 font-mono text-xs font-medium text-foreground">{agent}</span>
                  <McpCellStatusBadge agent={agent} cell={cell} />
                  <span className="min-w-48 flex-1 text-xs text-muted-foreground">
                    {syncRow?.reason ?? blockedRow?.reason}
                  </span>
                </li>
              );
            })}
          </ul>
        </div>
      )}

      <div className="mt-4 flex flex-wrap items-center gap-2 border-t pt-3">
        <button
          type="button"
          data-testid={`edit-server-${element.id}`}
          onClick={() => onEdit(element)}
          disabled={syncing}
          className="inline-flex items-center gap-1 rounded border bg-card px-2.5 py-1 text-xs text-muted-foreground hover:bg-accent disabled:opacity-50"
        >
          编辑定义
        </button>
        {actions.includes("enable") && !isEnabled && (
          <button
            type="button"
            data-testid={`enable-server-${element.id}`}
            onClick={() => onAction(element, "enable")}
            disabled={syncing}
            className="rounded border border-success/30 bg-success/10 px-2.5 py-1 text-xs text-success hover:bg-success/15 disabled:opacity-50"
          >
            启用
          </button>
        )}
        {actions.includes("disable") && isEnabled && (
          <button
            type="button"
            data-testid={`disable-server-${element.id}`}
            onClick={() => onAction(element, "disable")}
            disabled={syncing}
            className="rounded border border-warning/30 bg-warning/10 px-2.5 py-1 text-xs text-warning hover:bg-warning/15 disabled:opacity-50"
          >
            停用
          </button>
        )}
        {actions.includes("upgrade") && (
          <button
            type="button"
            data-testid={`upgrade-server-${element.id}`}
            onClick={() => onAction(element, "upgrade")}
            disabled={syncing}
            className="rounded border border-primary/30 bg-primary/10 px-2.5 py-1 text-xs text-primary hover:bg-primary/15 disabled:opacity-50"
          >
            换到 {element.freshness.upstream}
          </button>
        )}
        {actions.includes("remove") && (
          <button
            type="button"
            data-testid={`remove-server-${element.id}`}
            onClick={() => onAction(element, "remove")}
            disabled={syncing}
            className="rounded border border-destructive/30 bg-card px-2.5 py-1 text-xs text-destructive hover:bg-destructive/10 disabled:opacity-50"
          >
            删除服务
          </button>
        )}
      </div>
    </section>
  );
}
