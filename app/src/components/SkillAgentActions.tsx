import type { EngineElement } from "../lib/contract";
import type { SkillAgentCell } from "../lib/observed";
import { cn } from "../lib/utils";
import { MatrixCell } from "./MatrixCell";

/**
 * 技能 × 助手单元格：一行直接显示状态和当前有意义的操作。
 */

const SKILL_ACTION_VIEW: Record<string, { label: string; className: string }> = {
  add: { label: "添加", className: "border-primary/30 bg-primary/10 text-primary hover:bg-primary/15" },
  update: { label: "更新", className: "border-primary/30 bg-primary/10 text-primary hover:bg-primary/15" },
  enable: { label: "启用", className: "border-success/30 bg-success/10 text-success hover:bg-success/15" },
  disable: { label: "停用", className: "border-warning/30 bg-warning/10 text-warning hover:bg-warning/15" },
  remove: { label: "取消链接", className: "border-destructive/30 bg-card text-destructive hover:bg-destructive/10" },
};

/** 计划外链接行的动作文案：add=纳入管理（接管既有链接），remove=取消链接（仅移除入口）。 */
const EXTRA_LINK_ACTION_VIEW: Record<string, { label: string; title: string }> = {
  add: { label: "纳入管理", title: "保存当前技能内容并接管既有链接" },
  remove: { label: "取消链接", title: "移除磁盘上的链接入口，不删除技能本身" },
};

export interface SkillAgentAssignment {
  agent: string;
  cell: SkillAgentCell | null;
}

/** update 是引擎能力，不等于当前需要更新；只对版本变化或链接损坏提示。 */
export function skillNeedsUpdate(cell: SkillAgentCell | null): boolean {
  if (!cell?.actions?.includes("update")) return false;
  if (cell.state === "missing-link" || cell.state === "dead-link") return true;
  if (cell.state === "drifted") return false;
  return Boolean(cell.available_version && cell.version !== cell.available_version);
}

export function SkillAgentCellActions({
  skill,
  assignment,
  statusTitle,
  onAction,
}: {
  skill: EngineElement;
  assignment: SkillAgentAssignment;
  statusTitle?: string;
  onAction: (skill: EngineElement, agent: string, action: string, cell: SkillAgentCell | null) => void;
}) {
  const { agent, cell } = assignment;
  if (!cell) return null;
  const actions = cell.actions ?? [];
  const needsUpdate = skillNeedsUpdate(cell);
  const needsRepair = (cell.state === "missing-link" || cell.state === "dead-link") && actions.includes("update");
  // 当前技能分发只有软链接：已分配技能的 disable/remove 都删除同一入口，
  // UI 只保留可逆的停用；remove 用于未分配的计划外链接（取消链接），以及来源里已经
  // 没有的技能（启用、停用、更新都做不成，只剩移除）。
  const visibleActions = cell.source_missing
    ? actions.filter((action) => action === "remove")
    : cell.state === "extra-link"
    ? (["add", "remove"] as const).filter((action) => actions.includes(action))
    : ([
        ...(needsUpdate ? ["update"] : []),
        ...(["add", "enable", "disable"] as const).filter((action) => actions.includes(action)),
      ] as string[]);
  const showStatus = !["linked", "disabled", "not-assigned"].includes(cell.state) || visibleActions.length === 0;

  const actionLabel = (action: string) => {
    if (cell.source_missing && action === "remove") return "移除";
    if (needsRepair && action === "update") return "修复";
    if (cell.state === "not-assigned" && action === "add") return "启用";
    return cell.state === "extra-link" ? (EXTRA_LINK_ACTION_VIEW[action]?.label ?? SKILL_ACTION_VIEW[action]?.label) : SKILL_ACTION_VIEW[action]?.label;
  };

  return (
    <div className="inline-flex items-center justify-center gap-1.5 whitespace-nowrap text-xs">
      {showStatus && <MatrixCell state={cell.state} kind="skills" title={statusTitle} />}
      {visibleActions.map((action) => (
        <button
          key={action}
          type="button"
          data-testid={`action-${action}-${skill.id}-${agent}`}
          title={
            cell.source_missing
              ? "来源里已没有这个技能：删掉这条分配记录和残留入口"
              : cell.state === "extra-link"
              ? EXTRA_LINK_ACTION_VIEW[action]?.title
              : cell.state === "not-assigned" && action === "add"
                ? "首次启用并建立分配"
                : undefined
          }
          onClick={() => onAction(skill, agent, action, cell)}
          className={cn("rounded border px-1.5 py-0.5", SKILL_ACTION_VIEW[action].className)}
        >
          {actionLabel(action)}
        </button>
      ))}
      {cell.blocked_reason && (
        <span className="max-w-40 truncate text-warning" title={cell.blocked_reason}>
          {cell.blocked_reason}
        </span>
      )}
    </div>
  );
}
