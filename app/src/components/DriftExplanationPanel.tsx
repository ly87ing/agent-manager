import type { EngineElement } from "../lib/contract";
import { agentCellsOf } from "../lib/observed";
import { MatrixCell } from "./MatrixCell";

/**
 * 结构化「不一致」解释视图（只读，纯 status/observed 展示）：
 * 目标位置与各助手状态；引擎不提供 before/after 文本 diff。
 */

export function shortenDigest(digest: string | null): string {
  if (!digest) return "—";
  if (digest.length <= 24) return digest;
  return `${digest.slice(0, 16)}…${digest.slice(-6)}`;
}

function collectTargets(element: EngineElement): string[] {
  const cells = agentCellsOf(element);
  const raw: string[] = [];
  for (const [, cell] of Object.entries(cells)) {
    if (cell.target) raw.push(cell.target);
    if (cell.link_path) raw.push(cell.link_path);
  }
  const observed = element.observed as { target?: unknown };
  if (typeof observed.target === "string") raw.push(observed.target);
  const spec = element.spec as { target?: unknown };
  if (typeof spec.target === "string") raw.push(spec.target);
  return Array.from(new Set(raw.filter(Boolean)));
}

export function DriftExplanationPanel({
  element,
  cellKind,
}: {
  element: EngineElement;
  /** per-agent 单元格词汇表；不传则纯文本渲染 state。 */
  cellKind?: "mcp" | "skills";
}) {
  const cells = agentCellsOf(element);
  const cellEntries = Object.entries(cells);
  const targets = collectTargets(element);

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
    </div>
  );
}
