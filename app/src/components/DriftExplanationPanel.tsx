import { useEngineQuery } from "../lib/queries";
import type { EngineElement, PlanAction, PlanData } from "../lib/contract";
import { agentCellsOf } from "../lib/observed";
import { engineErrorMessage } from "../lib/engine";
import { MatrixCell } from "./MatrixCell";

/**
 * 结构化漂移解释视图（只读）。引擎 Phase 1 不提供 before/after 文本 diff，
 * 本面板从现有 status/plan payload 组合：目标路径、per-agent 单元格状态、
 * plan 动作清单（op/risk/target/before/after digest/note）。
 */

const RISK_LABELS: Record<string, string> = {
  read: "读取",
  "write-local": "写本地",
  "exec-local": "本地执行",
  network: "网络",
  "install-remove": "安装/卸载",
};

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
            <th className="py-1.5 pr-3 font-medium">风险级</th>
            <th className="py-1.5 pr-3 font-medium">目标</th>
            <th className="py-1.5 pr-3 font-medium">before → after</th>
            <th className="py-1.5 font-medium">说明</th>
          </tr>
        </thead>
        <tbody>
          {actions.map((action, index) => (
            <tr key={`${action.op}-${action.target ?? action.cmd ?? index}`} className="border-b border-slate-100 last:border-0">
              <td className="py-1.5 pr-3 font-mono">{action.op}</td>
              <td className="py-1.5 pr-3">{RISK_LABELS[action.risk] ?? action.risk}</td>
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
        <h4 className="mb-1.5 text-xs font-semibold text-slate-500">目标路径</h4>
        {targets.length > 0 ? (
          <ul className="space-y-0.5">
            {targets.map((target) => (
              <li key={target} className="font-mono text-xs break-all text-slate-600">
                {target}
              </li>
            ))}
          </ul>
        ) : (
          <p className="text-xs text-slate-400">（本元素不涉及文件目标）</p>
        )}
      </div>

      {cellEntries.length > 0 && (
        <div>
          <h4 className="mb-1.5 text-xs font-semibold text-slate-500">per-agent 单元格状态</h4>
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
          plan 动作清单（来自 {planCommand}，只读预览——引擎不锁版本，apply 将在 Phase 2 提供）
        </h4>
        {planQuery.isPending ? (
          <p className="text-xs text-slate-400">plan 加载中…</p>
        ) : planQuery.isError ? (
          <p className="text-xs text-red-600">plan 加载失败:{engineErrorMessage(planQuery.error)}</p>
        ) : actions.length > 0 ? (
          <PlanActionTable actions={actions} />
        ) : (
          <p className="text-xs text-slate-400">plan 为空：该元素当前无需收敛动作。</p>
        )}
      </div>
    </div>
  );
}
