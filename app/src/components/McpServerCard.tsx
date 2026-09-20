import { Check, CircleAlert, X } from "lucide-react";
import type { EngineElement } from "../lib/contract";
import { agentCellsOf, type McpObserved, type McpSpec } from "../lib/observed";
import { FreshnessBadge } from "./FreshnessBadge";
import { StateBadge } from "./StateBadge";
import { DriftExplanationPanel } from "./DriftExplanationPanel";
import { McpCellActions, McpCellStatusBadge, type McpAgentCellView } from "./McpAssignmentRow";
import { cn } from "../lib/utils";

/** 服务级 observed 扩展：库级操作动作与受阻原因（observed.ts 冻结，本地扩展）。 */
export interface McpServerObserved extends McpObserved {
  actions?: string[];
  blocked_reason?: string | null;
}

const RESOLUTION_STATES = new Set(["drifted", "leftover", "probe-error", "missing", "not-materialized"]);

function resolutionActions(cell: McpAgentCellView): string[] {
  const actions = cell.actions ?? [];
  if (cell.state === "leftover") return actions.includes("remove") ? ["remove"] : [];
  if (cell.state === "drifted") {
    if (actions.includes("update")) return ["update"];
    if (actions.includes("disable")) return ["disable"];
    return [];
  }
  if (cell.state === "missing" || cell.state === "not-materialized") {
    if (actions.includes("update")) return ["update"];
    if (actions.includes("add")) return ["add"];
  }
  return [];
}

function resolutionReason(cell: McpAgentCellView, actions: string[]): string {
  if (cell.blocked_reason) return cell.blocked_reason;
  if (cell.state === "leftover") return "分配已取消，但旧配置还留在这个助手中。";
  if (cell.state === "probe-error") return "配置读取失败，修复文件后再重新检查。";
  if (cell.state === "not-materialized") return "这个助手的配置文件尚未生成。";
  if (cell.state === "missing") return "服务库已有定义，但这个助手中还没有该服务。";
  if (actions.includes("disable")) return "分配已停用，但助手中仍保留启用配置。";
  return "这个助手中的配置与服务库定义不同。";
}

function displayPlaceholder(value: string | undefined): string {
  if (!value) return "<明文已隐藏>";
  return value.startsWith("${") ? value : `\${${value}}`;
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
  onCellAction,
  onTakeover,
  onConfigureSecrets,
}: {
  element: EngineElement;
  selected: boolean;
  onSelect: (id: string | null) => void;
  onEdit: (element: EngineElement) => void;
  onAction: (element: EngineElement, action: "enable" | "disable" | "remove" | "upgrade") => void;
  onCellAction: (element: EngineElement, agent: string, action: string) => void;
  onTakeover: (element: EngineElement, agent: string, action: string) => void;
  onConfigureSecrets?: () => void;
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
  const configuredReferences = Object.entries(isRemote ? spec.headers ?? {} : spec.env ?? {});
  const resolutionEntries = Object.entries(agentCellsOf(element))
    .map(([agent, cell]) => ({ agent, cell: cell as McpAgentCellView }))
    .filter(
      ({ agent, cell }) =>
        RESOLUTION_STATES.has(cell.state) &&
        (spec.agents == null || spec.agents.includes(agent) || cell.assigned || cell.state === "leftover"),
    );
  const hasMissingSecrets = resolutionEntries.some(({ cell }) =>
    (cell.blocked_reason ?? "").includes("凭证变量未配置"),
  );

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
          <p className="mt-1 font-mono text-xs break-all text-muted-foreground">
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
      </div>

      <dl className="mt-3 grid grid-cols-1 gap-x-4 gap-y-2 rounded-md border bg-muted/20 p-3 text-xs sm:grid-cols-2">
        {isRemote ? (
          <div className="sm:col-span-2">
            <dt className="text-muted-foreground">服务地址 <code>url</code></dt>
            <dd className="mt-0.5 break-all font-mono text-foreground">{spec.url}</dd>
          </div>
        ) : (
          <>
            <div>
              <dt className="text-muted-foreground">启动命令 <code>command</code></dt>
              <dd className="mt-0.5 break-all font-mono text-foreground">{spec.command}</dd>
            </div>
            <div>
              <dt className="text-muted-foreground">启动超时 <code>startup_timeout_sec</code></dt>
              <dd className="mt-0.5 font-mono text-foreground">{spec.startup_timeout_sec ?? "未指定"}</dd>
            </div>
            <div className="sm:col-span-2">
              <dt className="text-muted-foreground">参数 <code>args</code></dt>
              <dd className="mt-0.5 max-h-16 overflow-auto whitespace-pre-wrap break-all font-mono text-foreground">
                {JSON.stringify(spec.args ?? [])}
              </dd>
            </div>
          </>
        )}
        <div className="sm:col-span-2">
          <dt className="text-muted-foreground">变量引用</dt>
          <dd className="mt-1 max-h-24 overflow-auto rounded bg-background px-2 py-1.5 font-mono text-foreground">
            {configuredReferences.length === 0 ? "无" : configuredReferences.map(([key, value]) => (
              <span key={key} className="block break-all">{key} = {value}</span>
            ))}
          </dd>
        </div>
        {!isRemote && Object.keys(spec.extra_args_by_agent ?? {}).length > 0 && (
          <div className="sm:col-span-2">
            <dt className="text-muted-foreground">各助手附加参数 <code>extra_args_by_agent</code></dt>
            <dd className="mt-0.5 max-h-16 overflow-auto break-all font-mono text-foreground">
              {JSON.stringify(spec.extra_args_by_agent)}
            </dd>
          </div>
        )}
      </dl>

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
              {resolutionEntries.length} 个助手待处理
            </div>
            {hasMissingSecrets && onConfigureSecrets && (
              <button
                type="button"
                data-testid={`configure-secrets-${element.id}`}
                onClick={onConfigureSecrets}
                className="rounded border border-primary/30 bg-primary/10 px-2 py-0.5 text-xs font-medium text-primary hover:bg-primary/15"
              >
                配置缺失变量
              </button>
            )}
          </div>
          <ul className="mt-2 divide-y divide-border/70">
            {resolutionEntries.map(({ agent, cell }) => {
              const actions = resolutionActions(cell);
              return (
                <li
                  key={agent}
                  data-testid={`mcp-resolution-${element.id}-${agent}`}
                  className="flex flex-wrap items-center gap-x-2 gap-y-1.5 py-2 first:pt-0 last:pb-0"
                >
                  <span className="min-w-24 font-mono text-xs font-medium text-foreground">{agent}</span>
                  <McpCellStatusBadge agent={agent} cell={cell} />
                  <span className="min-w-48 flex-1 text-xs text-muted-foreground">
                    {resolutionReason(cell, actions)}
                  </span>
                  <McpCellActions
                    element={element}
                    agent={agent}
                    cell={cell}
                    visibleActions={actions}
                    showEmptyReason={false}
                    onAction={onCellAction}
                    onTakeover={onTakeover}
                  />
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
          className="inline-flex items-center gap-1 rounded border bg-card px-2.5 py-1 text-xs text-muted-foreground hover:bg-accent"
        >
          编辑定义
        </button>
        {actions.includes("enable") && !isEnabled && (
          <button
            type="button"
            data-testid={`enable-server-${element.id}`}
            onClick={() => onAction(element, "enable")}
            className="rounded border border-success/30 bg-success/10 px-2.5 py-1 text-xs text-success hover:bg-success/15"
          >
            启用
          </button>
        )}
        {actions.includes("disable") && isEnabled && (
          <button
            type="button"
            data-testid={`disable-server-${element.id}`}
            onClick={() => onAction(element, "disable")}
            className="rounded border border-warning/30 bg-warning/10 px-2.5 py-1 text-xs text-warning hover:bg-warning/15"
          >
            停用
          </button>
        )}
        {actions.includes("upgrade") && (
          <button
            type="button"
            data-testid={`upgrade-server-${element.id}`}
            onClick={() => onAction(element, "upgrade")}
            className="rounded border border-primary/30 bg-primary/10 px-2.5 py-1 text-xs text-primary hover:bg-primary/15"
          >
            换到 {element.freshness.upstream}
          </button>
        )}
        {actions.includes("remove") && (
          <button
            type="button"
            data-testid={`remove-server-${element.id}`}
            onClick={() => onAction(element, "remove")}
            className="rounded border border-destructive/30 bg-card px-2.5 py-1 text-xs text-destructive hover:bg-destructive/10"
          >
            删除服务
          </button>
        )}
        {actions.length === 0 && <span className="text-xs text-muted-foreground">当前引擎未提供库级操作</span>}
      </div>
    </section>
  );
}
