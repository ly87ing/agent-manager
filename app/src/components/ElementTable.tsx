import type { EngineElement } from "../lib/contract";
import { FreshnessBadge } from "./FreshnessBadge";
import { StateBadge } from "./StateBadge";

/** 元素只读表：id + 双轴徽章。写操作按钮 Phase 2 才出现（且只渲染 engine 提供的 actions）。 */
export function ElementTable({ elements }: { elements: EngineElement[] }) {
  if (elements.length === 0) {
    return <p className="py-4 text-center text-sm text-slate-400">无元素</p>;
  }
  return (
    <div className="overflow-x-auto">
      <table className="w-full text-left text-sm">
        <thead>
          <tr className="border-b border-slate-200 text-xs text-slate-500">
            <th className="py-2 pr-4 font-medium">元素</th>
            <th className="py-2 pr-4 font-medium">同步状态</th>
            <th className="py-2 font-medium">最新版本</th>
          </tr>
        </thead>
        <tbody>
          {elements.map((element) => (
            <tr key={element.id} className="border-b border-slate-100 last:border-0">
              <td className="py-2 pr-4 font-mono text-xs text-slate-700">{element.id}</td>
              <td className="py-2 pr-4">
                <StateBadge state={element.state} />
              </td>
              <td className="py-2">
                <FreshnessBadge freshness={element.freshness} />
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
