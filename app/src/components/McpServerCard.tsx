import { Check, X } from "lucide-react";
import type { EngineElement } from "../lib/contract";
import { agentCellsOf, type McpObserved, type McpSpec } from "../lib/observed";
import type { McpAgentCellView } from "./McpAssignmentRow";
import { FreshnessBadge } from "./FreshnessBadge";
import { StateBadge } from "./StateBadge";
import { DriftExplanationPanel } from "./DriftExplanationPanel";
import { cn } from "../lib/utils";

/** 服务级 observed 扩展：库级操作动作与受阻原因（observed.ts 冻结，本地扩展）。 */
export interface McpServerObserved extends McpObserved {
  actions?: string[];
  blocked_reason?: string | null;
}

function detailPlanRequest(element: EngineElement): { command: string; args: string[] } | null {
  const cells = ((element.observed as McpObserved).agents ?? {}) as Record<string, McpAgentCellView>;
  for (const action of ["update", "enable", "add"]) {
    const agent = Object.keys(cells)
      .sort()
      .find((key) => (cells[key].actions ?? []).includes(action));
    if (agent) {
      return {
        command: "mcp:plan",
        args: ["--only", element.id, "--agent", agent, "--action", action],
      };
    }
  }
  return null;
}

function EnvResolutionList({ element }: { element: EngineElement }) {
  const observed = element.observed as McpObserved;
  const env = observed.env ?? {};
  const entries = Object.entries(env);
  if (entries.length === 0) {
    return <p className="text-xs text-slate-400">不需要环境变量。</p>;
  }
  return (
    <ul className="space-y-1">
      {entries.map(([name, resolution]) => (
        <li key={name} className="flex items-center gap-2 text-xs">
          {resolution.resolved ? (
            <Check className="h-3.5 w-3.5 text-emerald-600" aria-label="已配置" />
          ) : (
            <X className="h-3.5 w-3.5 text-red-500" aria-label="未配置" />
          )}
          <span className="font-mono text-slate-700">{name}</span>
          <span className={resolution.resolved ? "text-emerald-600" : "text-red-600"}>
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
    return <p className="text-xs text-slate-400">不需要请求头。</p>;
  }
  return (
    <ul className="space-y-1">
      {entries.map(([name, resolution]) => (
        <li key={name} className="flex items-center gap-2 text-xs" data-testid="mcp-header-resolution">
          {resolution.resolved ? (
            <Check className="h-3.5 w-3.5 text-emerald-600" aria-label="已配置" />
          ) : (
            <X className="h-3.5 w-3.5 text-red-500" aria-label="未配置" />
          )}
          <span className="font-mono text-slate-700">{name}</span>
          <span className={resolution.resolved ? "text-emerald-600" : "text-red-600"}>
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
    <section data-testid={`mcp-detail-${element.id}`} className="space-y-4 rounded-lg border border-slate-200 bg-white p-5 shadow-sm">
      {isRemote ? (
        <div>
          <h4 className="mb-1 text-xs font-semibold text-slate-500">服务地址（远程 HTTP）</h4>
          <code className="block overflow-x-auto rounded bg-slate-900 px-3 py-2 font-mono text-xs break-all text-slate-100">
            {spec.url}
          </code>
          <p className="mt-1 text-xs text-slate-400">
            应用到: {spec.agents === null || spec.agents === undefined ? "全部助手" : spec.agents.join(", ")}
          </p>
        </div>
      ) : (
        <div>
          <h4 className="mb-1 text-xs font-semibold text-slate-500">启动命令</h4>
          <code className="block overflow-x-auto rounded bg-slate-900 px-3 py-2 font-mono text-xs break-all text-slate-100">
            {spec.command} {(spec.args ?? []).join(" ")}
          </code>
          <p className="mt-1 text-xs text-slate-400">
            启动超时 {spec.startup_timeout_sec ?? "—"} 秒 · 应用到:{" "}
            {spec.agents === null || spec.agents === undefined ? "全部助手" : spec.agents.join(", ")}
          </p>
        </div>
      )}

      {Object.keys(extraArgs).length > 0 && (
        <div>
          <h4 className="mb-1 text-xs font-semibold text-slate-500">各助手附加参数</h4>
          <ul className="space-y-1">
            {Object.entries(extraArgs).map(([agent, args]) => (
              <li key={agent} className="text-xs">
                <span className="font-mono text-slate-500">{agent}: </span>
                <code className="font-mono text-slate-700">{args.join(" ")}</code>
              </li>
            ))}
          </ul>
        </div>
      )}

      {isRemote ? (
        <div>
          <h4 className="mb-1 text-xs font-semibold text-slate-500">请求头变量（只显示是否已配置，永不显示值）</h4>
          <HeaderResolutionList element={element} />
        </div>
      ) : (
        <div>
          <h4 className="mb-1 text-xs font-semibold text-slate-500">密钥变量（只显示是否已配置，永不显示值）</h4>
          <EnvResolutionList element={element} />
        </div>
      )}

      <div>
        <h4 className="mb-1 text-xs font-semibold text-slate-500">写入位置</h4>
        <ul className="space-y-1">
          {Object.entries(cells).map(([agent, cell]) => (
            <li key={agent} className="flex flex-wrap items-center gap-2 text-xs">
              <span className="font-mono text-slate-500">{agent}</span>
              <code className="font-mono break-all text-slate-700">{cell.target ?? "—"}</code>
              <span className={cell.target_exists ? "text-emerald-600" : "text-slate-400"}>
                {cell.target_exists ? "已存在" : "未生成"}
              </span>
            </li>
          ))}
        </ul>
      </div>

      <DriftExplanationPanel element={element} planRequest={detailPlanRequest(element)} cellKind="mcp" />
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
}: {
  element: EngineElement;
  selected: boolean;
  onSelect: (id: string | null) => void;
  onEdit: (element: EngineElement) => void;
  onAction: (element: EngineElement, action: "enable" | "disable" | "remove") => void;
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

  return (
    <section
      data-testid={`mcp-card-${element.id}`}
      className={cn("rounded-lg border bg-white p-5 shadow-sm", selected ? "border-indigo-300" : "border-slate-200")}
    >
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <button
            type="button"
            onClick={() => onSelect(selected ? null : element.id)}
            aria-expanded={selected}
            className="text-left font-mono text-xs font-semibold break-all text-indigo-600 hover:text-indigo-800"
            title="点击展开/收起服务详情"
          >
            {element.id}
          </button>
          <p className="mt-1 font-mono text-xs break-all text-slate-400">
            {isRemote ? spec.url : `${spec.command ?? ""} ${(spec.args ?? []).join(" ")}`}
          </p>
        </div>
        <div className="flex flex-shrink-0 items-center gap-2">
          <StateBadge state={element.state} />
          <FreshnessBadge freshness={element.freshness} className="items-end" />
        </div>
      </div>

      <div className="mt-3 flex flex-wrap gap-1.5 text-xs">
        <span className="rounded border border-slate-200 bg-slate-50 px-1.5 py-0.5 text-slate-600">
          {spec.agents === null || spec.agents === undefined ? "全部助手" : `限定 ${spec.agents.length} 个助手`}
        </span>
        {isRemote && (
          <span className="rounded border border-sky-200 bg-sky-50 px-1.5 py-0.5 text-sky-700">远程 HTTP</span>
        )}
        <span
          className={cn(
            "rounded border px-1.5 py-0.5",
            secretEntries.length === 0 || envResolved === secretEntries.length
              ? "border-slate-200 bg-slate-50 text-slate-600"
              : "border-amber-200 bg-amber-50 text-amber-700",
          )}
        >
          {isRemote ? `请求头 ${envResolved}/${secretEntries.length} 已配置` : `密钥变量 ${envResolved}/${secretEntries.length} 已配置`}
        </span>
      </div>

      {observed.blocked_reason && (
        <div className="mt-3 rounded-md border border-amber-200 bg-amber-50 px-3 py-2 text-xs text-amber-800">
          受阻原因：{observed.blocked_reason}
        </div>
      )}

      <div className="mt-4 flex flex-wrap items-center gap-2 border-t border-slate-100 pt-3">
        <button
          type="button"
          data-testid={`edit-server-${element.id}`}
          onClick={() => onEdit(element)}
          className="inline-flex items-center gap-1 rounded border border-slate-200 bg-white px-2.5 py-1 text-xs text-slate-600 hover:bg-slate-50"
        >
          编辑定义
        </button>
        {actions.includes("enable") && !isEnabled && (
          <button
            type="button"
            data-testid={`enable-server-${element.id}`}
            onClick={() => onAction(element, "enable")}
            className="rounded border border-emerald-200 bg-emerald-50 px-2.5 py-1 text-xs text-emerald-700 hover:bg-emerald-100"
          >
            启用
          </button>
        )}
        {actions.includes("disable") && isEnabled && (
          <button
            type="button"
            data-testid={`disable-server-${element.id}`}
            onClick={() => onAction(element, "disable")}
            className="rounded border border-amber-200 bg-amber-50 px-2.5 py-1 text-xs text-amber-700 hover:bg-amber-100"
          >
            停用
          </button>
        )}
        {actions.includes("remove") && (
          <button
            type="button"
            data-testid={`remove-server-${element.id}`}
            onClick={() => onAction(element, "remove")}
            className="rounded border border-red-200 bg-white px-2.5 py-1 text-xs text-red-600 hover:bg-red-50"
          >
            删除服务
          </button>
        )}
        {actions.length === 0 && <span className="text-xs text-slate-400">当前引擎未提供库级操作</span>}
      </div>
    </section>
  );
}
