import { useEngineQuery } from "../lib/queries";
import type { EngineElement, PlanAction, PlanData } from "../lib/contract";
import { agentCellsOf } from "../lib/observed";
import { engineErrorMessage } from "../lib/engine";
import { COPY, RISK_COPY } from "../lib/ui-copy";
import { MatrixCell } from "./MatrixCell";

/**
 * 结构化「不一致」解释视图（只读）。引擎不提供 before/after 文本 diff，
 * 本面板从现有 status/plan payload 组合：目标位置、各助手状态、
 * 将要执行的操作清单（op/类型/位置/改动前后内容指纹/说明）。
 */

export function shortenDigest(digest: string | null): string {
  if (!digest) return "—";
  if (digest.length <= 24) return digest;
  return `${digest.slice(0, 16)}…${digest.slice(-6)}`;
}

function PlanActionTable({ actions }: { actions: PlanAction[] }) {
  return (
    <div className="overflow-x-auto">
      <table className="w-full text-left text-xs">
        <thead>
          <tr className="border-b border-slate-200 text-slate-500">
            <th className="py-1.5 pr-3 font-medium">操作</th>
            <th className="py-1.5 pr-3 font-medium">类型</th>
            <th className="py-1.5 pr-3 font-medium">位置</th>
            <th className="py-1.5 pr-3 font-medium">内容指纹（前 → 后）</th>
            <th className="py-1.5 font-medium">说明</th>
          </tr>
        </thead>
        <tbody>
          {actions.map((action, index) => (
            <tr key={`${action.op}-${action.target ?? action.cmd ?? index}`} className="border-b border-slate-100 last:border-0">
              <td className="py-1.5 pr-3 font-mono">{action.op}</td>
              <td className="py-1.5 pr-3">{RISK_COPY[action.risk]?.label ?? action.risk}</td>
              <td className="max-w-56 truncate py-1.5 pr-3 font-mono" title={action.target ?? undefined}>
                {action.target ?? action.cmd ?? "—"}
              </td>
              <td className="py-1.5 pr-3 font-mono text-slate-500">
                {action.before_digest || action.after_digest
                  ? `${shortenDigest(action.before_digest)} → ${shortenDigest(action.after_digest)}`
                  : "—"}
              </td>
              <td className="py-1.5 text-slate-600">{action.note ?? "—"}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

export function DriftExplanationPanel({
  element,
  planCommand,
  cellKind,
}: {
  element: EngineElement;
  /** 对应域的 plan 命令（如 "mcp:plan"）；面板取其动作清单并过滤到本元素。 */
  planCommand: string;
  /** per-agent 单元格词汇表；不传则纯文本渲染 state。 */
  cellKind?: "mcp" | "skills";
}) {
  const planQuery = useEngineQuery<PlanData>(planCommand);
  const allActions = planQuery.data?.data.actions ?? [];
  const actions = allActions.filter((action) => action.element === element.id);

  const cells = agentCellsOf(element);
  const cellEntries = Object.entries(cells);
  const targets = Array.from(
    new Set([
      ...cellEntries.map(([, cell]) => cell.target ?? cell.link_path ?? "").filter(Boolean),
      ...actions.map((action) => action.target ?? "").filter(Boolean),
    ]),
  );

  return (
    <div data-testid="drift-panel" className="space-y-4 rounded-md border border-slate-200 bg-slate-50 p-4">
      <div>
        <h4 className="mb-1.5 text-xs font-semibold text-slate-500">目标位置</h4>
        {targets.length > 0 ? (
          <ul className="space-y-0.5">
            {targets.map((target) => (
              <li key={target} className="font-mono text-xs break-all text-slate-600">
                {target}
              </li>
            ))}
          </ul>
        ) : (
          <p className="text-xs text-slate-400">（本元素不涉及文件）</p>
        )}
      </div>

      {cellEntries.length > 0 && (
        <div>
          <h4 className="mb-1.5 text-xs font-semibold text-slate-500">各助手状态</h4>
          <table className="w-full text-left text-xs">
            <tbody>
              {cellEntries.map(([agent, cell]) => (
                <tr key={agent} className="border-b border-slate-100 last:border-0">
                  <td className="w-36 py-1.5 pr-3 font-mono text-slate-600">{agent}</td>
                  <td className="py-1.5 pr-3">
                    {cellKind ? (
                      <MatrixCell state={cell.state} kind={cellKind} />
                    ) : (
                      <span className="font-mono text-slate-600">{cell.state}</span>
                    )}
                  </td>
                  <td className="py-1.5 font-mono break-all text-slate-400">
                    {cell.target ?? cell.link_path ?? ""}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      <div>
        <h4 className="mb-1.5 text-xs font-semibold text-slate-500">
          {COPY.planPreviewTitle}（来自 {planCommand} 的预览，只读）
        </h4>
        {planQuery.isPending ? (
          <p className="text-xs text-slate-400">正在计算将要执行的操作…</p>
        ) : planQuery.isError ? (
          <p className="text-xs text-red-600">预览加载失败:{engineErrorMessage(planQuery.error)}</p>
        ) : actions.length > 0 ? (
          <PlanActionTable actions={actions} />
        ) : (
          <p className="text-xs text-slate-400">没有需要执行的操作。</p>
        )}
      </div>
    </div>
  );
}
