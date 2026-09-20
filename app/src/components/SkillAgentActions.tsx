import type { EngineElement } from "../lib/contract";
import type { SkillAgentCell } from "../lib/observed";
import { cn } from "../lib/utils";
import { MatrixCell } from "./MatrixCell";

/**
 * 技能 × 助手单元格：状态、精确选择和可执行动作在同一行完成。
 */

const SKILL_ACTIONS: { action: string; label: string; className: string }[] = [
  { action: "add", label: "添加", className: "border-primary/30 bg-primary/10 text-primary hover:bg-primary/15" },
  { action: "update", label: "更新", className: "border-primary/30 bg-primary/10 text-primary hover:bg-primary/15" },
  { action: "enable", label: "启用", className: "border-success/30 bg-success/10 text-success hover:bg-success/15" },
  { action: "disable", label: "停用", className: "border-warning/30 bg-warning/10 text-warning hover:bg-warning/15" },
  { action: "remove", label: "删除分配", className: "border-destructive/30 bg-card text-destructive hover:bg-destructive/10" },
];

/** 计划外链接行的动作文案：add=纳入管理（接管既有链接），remove=取消链接（仅移除入口）。 */
const EXTRA_LINK_ACTION_VIEW: Record<string, { label: string; title: string }> = {
  add: { label: "纳入管理", title: "保存当前技能内容并接管既有链接" },
  remove: { label: "取消链接", title: "移除磁盘上的链接入口，不删除技能本身" },
};

export interface SkillAgentAssignment {
  key: string;
  agent: string;
  cell: SkillAgentCell | null;
}

export function SkillAgentCellActions({
  skill,
  skillName,
  assignment,
  statusTitle,
  selectedKeys,
  selectDisabled,
  onToggleSelect,
  onAction,
}: {
  skill: EngineElement;
  skillName: string;
  assignment: SkillAgentAssignment;
  statusTitle?: string;
  selectedKeys: ReadonlySet<string>;
  /** 批量执行在途时禁用勾选。 */
  selectDisabled: boolean;
  onToggleSelect: (key: string) => void;
  onAction: (skill: EngineElement, agent: string, action: string, cell: SkillAgentCell | null) => void;
}) {
  const { key, agent, cell } = assignment;
  if (!cell) return null;
  const actions = cell.actions ?? [];

  return (
    <div className="inline-flex items-center justify-center gap-1.5 whitespace-nowrap text-xs">
      <input
        type="checkbox"
        aria-label={`选择 ${skillName} @ ${agent}`}
        data-testid={`select-${skill.id}-${agent}`}
        disabled={selectDisabled || actions.length === 0}
        checked={selectedKeys.has(key)}
        onChange={() => onToggleSelect(key)}
        className="h-3.5 w-3.5 accent-primary disabled:opacity-40"
      />
      <MatrixCell state={cell.state} kind="skills" title={statusTitle} />
      {SKILL_ACTIONS.filter(({ action }) => actions.includes(action)).map(({ action, label, className }) => {
        const extraView = cell.state === "extra-link" ? EXTRA_LINK_ACTION_VIEW[action] : undefined;
        return (
          <button
            key={action}
            type="button"
            data-testid={`action-${action}-${skill.id}-${agent}`}
            title={extraView?.title}
            onClick={() => onAction(skill, agent, action, cell)}
            className={cn("rounded border px-1.5 py-0.5", className)}
          >
            {extraView?.label ?? label}
          </button>
        );
      })}
      {cell.blocked_reason && (
        <span className="max-w-40 truncate text-warning" title={cell.blocked_reason}>
          {cell.blocked_reason}
        </span>
      )}
    </div>
  );
}
