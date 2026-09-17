import type { ReactNode } from "react";
import type { EngineElement } from "../lib/contract";
import { FreshnessBadge } from "./FreshnessBadge";
import { StateBadge } from "./StateBadge";

/** 元素表：id + 双轴徽章 + 可选操作列（如启停开关）。 */
export function ElementTable({
  elements,
  renderActions,
}: {
  elements: EngineElement[];
  /** 追加的操作列（如启停开关）。 */
  renderActions?: (element: EngineElement) => ReactNode;
}) {
  if (elements.length === 0) {
    return <p className="py-4 text-center text-sm text-muted-foreground">无元素</p>;
  }
  return (
    <div className="overflow-x-auto">
      <table className="w-full text-left text-sm">
        <thead>
          <tr className="border-b text-xs text-muted-foreground">
            <th className="py-2 pr-4 font-medium">元素</th>
            <th className="py-2 pr-4 font-medium">同步状态</th>
            <th className="py-2 font-medium">最新版本</th>
            {renderActions && <th className="py-2 pl-4 font-medium text-right">操作</th>}
          </tr>
        </thead>
        <tbody>
          {elements.map((element) => (
            <tr key={element.id} className="border-b last:border-0">
              <td className="py-2 pr-4 font-mono text-xs text-foreground">{element.id}</td>
              <td className="py-2 pr-4">
                <StateBadge state={element.state} />
              </td>
              <td className="py-2">
                <FreshnessBadge freshness={element.freshness} />
              </td>
              {renderActions && <td className="py-2 pl-4 text-right">{renderActions(element)}</td>}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
