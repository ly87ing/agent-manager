import { ChevronDown, ChevronRight } from "lucide-react";
import { Fragment, type ReactNode } from "react";
import { MatrixCell } from "./MatrixCell";
import { cn } from "../lib/utils";

/**
 * 资源 × 助手矩阵：一行一个资源，每个助手一格状态。
 *
 * 替代「资源 × 助手」笛卡尔积长表——那种表里资源名要重复 N 遍（5 个技能 × 5 个助手 = 25 行），
 * 看一眼说不出「哪个技能在哪缺」。矩阵一行看全一个资源在所有助手上的处境。
 *
 * 详细操作由调用方决定放在单元格内还是展开行：技能页适合单行直接操作，
 * MCP 配置等详情较多的资源仍可展开查看。
 */

export interface MatrixRowCell {
  state: string;
  /** 悬停看的详情：版本、阻塞原因等，格子本身不占位置显示。 */
  title?: string;
  /** 该格有没有可执行的动作，用来提示这一行值得点开。 */
  hasActions?: boolean;
}

export interface MatrixRow {
  key: string;
  /** 资源名（技能名 / 服务名）。 */
  label: string;
  /** 次要信息，如所属集合。 */
  sublabel?: string;
  /** 助手 → 该格状态；缺失表示该助手与这个资源无关。 */
  cells: Record<string, MatrixRowCell | null>;
  /** 可批量选中（有可执行动作时）。 */
  selectable?: boolean;
}

export function ResourceAgentMatrix({
  rows,
  agents,
  kind,
  expandedKeys,
  onToggleExpand,
  renderExpanded,
  renderCell,
  isRowSelected,
  allSelected = false,
  someSelected = false,
  selectDisabled,
  onToggleSelect,
  onToggleSelectAll,
  resourceHeader = "资源",
  emptyHint,
}: {
  rows: MatrixRow[];
  agents: string[];
  kind: "mcp" | "skills";
  /** 允许同时展开多行；不传时不显示展开列。 */
  expandedKeys?: ReadonlySet<string>;
  onToggleExpand?: (key: string) => void;
  /** 展开后显示的逐助手操作区。 */
  renderExpanded?: (row: MatrixRow) => ReactNode;
  /** 自定义单元格内容，用于把状态与简短操作放在同一行。 */
  renderCell?: (row: MatrixRow, agent: string, cell: MatrixRowCell) => ReactNode;
  /** 行是否选中由调用方判定：选中项可能以「资源×助手」为单位，矩阵不猜这个模型。 */
  isRowSelected?: (rowKey: string) => boolean;
  allSelected?: boolean;
  someSelected?: boolean;
  /** 批量执行期间禁用勾选，避免执行中改选。 */
  selectDisabled?: boolean;
  onToggleSelect?: (key: string) => void;
  onToggleSelectAll?: () => void;
  resourceHeader?: string;
  emptyHint?: ReactNode;
}) {
  const selectable = rows.filter((row) => row.selectable);
  const expandable = Boolean(onToggleExpand && renderExpanded);

  if (rows.length === 0) return <>{emptyHint}</>;

  return (
    <div className="overflow-x-auto rounded-lg border bg-card p-5 shadow-sm">
      <table className="w-full text-left text-sm">
        <thead>
          <tr className="border-b text-xs text-muted-foreground">
            {onToggleSelect && (
              <th className="w-8 py-2.5 pr-2 font-medium">
                <input
                  type="checkbox"
                  aria-label="全选当前可见行"
                  data-testid="select-all-visible"
                  disabled={selectDisabled || selectable.length === 0}
                  checked={allSelected}
                  ref={(element) => {
                    if (element) element.indeterminate = someSelected && !allSelected;
                  }}
                  onChange={() => onToggleSelectAll?.()}
                  className="h-3.5 w-3.5 accent-primary disabled:opacity-40"
                />
              </th>
            )}
            {expandable && <th className="w-8 py-2.5 pr-2" />}
            <th className="py-2.5 pr-4 font-medium">{resourceHeader}</th>
            {agents.map((agent) => (
              <th key={agent} className="whitespace-nowrap py-2.5 pr-4 text-center font-mono font-medium">
                {agent}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {rows.map((row) => {
            const expanded = expandable && (expandedKeys?.has(row.key) ?? false);
            return (
              <Fragment key={row.key}>
                <tr
                  data-testid={`matrix-row-${row.key}`}
                  className={cn("border-b last:border-0", expanded && "bg-accent/30")}
                >
                  {onToggleSelect && (
                    <td className="py-2 pr-2">
                      <input
                        type="checkbox"
                        aria-label={`选择 ${row.label}`}
                        data-testid={`select-${row.key}`}
                        disabled={selectDisabled || !row.selectable}
                        checked={isRowSelected?.(row.key) ?? false}
                        onChange={() => onToggleSelect(row.key)}
                        className="h-3.5 w-3.5 accent-primary disabled:opacity-40"
                      />
                    </td>
                  )}
                  {expandable && (
                    <td className="py-2 pr-2">
                      <button
                        type="button"
                        data-testid={`expand-${row.key}`}
                        aria-expanded={expanded}
                        aria-label={`${expanded ? "收起" : "展开"} ${row.label} 的逐助手操作`}
                        onClick={() => onToggleExpand?.(row.key)}
                        className="rounded p-0.5 text-muted-foreground hover:bg-accent"
                      >
                        {expanded ? <ChevronDown className="h-3.5 w-3.5" aria-hidden /> : <ChevronRight className="h-3.5 w-3.5" aria-hidden />}
                      </button>
                    </td>
                  )}
                  <td className="py-2 pr-4">
                    <span className="font-mono text-xs font-medium text-foreground">{row.label}</span>
                    {row.sublabel && <span className="ml-1.5 text-[11px] text-muted-foreground">{row.sublabel}</span>}
                  </td>
                  {agents.map((agent) => {
                    const cell = row.cells[agent];
                    return (
                      <td key={agent} className="py-2 pr-4 text-center">
                        {cell && renderCell ? (
                          renderCell(row, agent, cell)
                        ) : cell ? (
                          <span className="inline-flex flex-col items-center gap-0.5">
                            <MatrixCell state={cell.state} kind={kind} title={cell.title} />
                          </span>
                        ) : (
                          // 该助手与这个资源无关：用一个点代替「未配置」四个字，一眼能看出哪几格
                          // 才是重点；悬停仍说清楚它是什么意思。
                          <span title={`${agent}: 未配置`} className="text-xs text-muted-foreground/40">
                            ·
                          </span>
                        )}
                      </td>
                    );
                  })}
                </tr>
                {expanded && renderExpanded && (
                  <tr className="border-b bg-accent/20 last:border-0">
                    <td
                      colSpan={agents.length + 1 + (onToggleSelect ? 1 : 0) + (expandable ? 1 : 0)}
                      className="px-3 py-2"
                    >
                      {renderExpanded(row)}
                    </td>
                  </tr>
                )}
              </Fragment>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}
