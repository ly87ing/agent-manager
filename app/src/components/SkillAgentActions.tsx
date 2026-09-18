import type { EngineElement } from "../lib/contract";
import type { SkillAgentCell } from "../lib/observed";
import { cn } from "../lib/utils";

/**
 * 一个技能展开后的逐助手操作条。
 *
 * 展开区原来是一张 5 行 × 6 列的表，技能名、所属集合、助手、状态四列跟上面矩阵那一行
 * 逐字重复，剩下两列（可用版本 / 当前版本）多数时候是「未获取」「已链接」。真正只在这里
 * 才有的东西就一样：能点的那几个动作。所以只留助手名加动作。
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

export function SkillAgentActions({
  skill,
  skillName,
  assignments,
  selectedKeys,
  selectDisabled,
  onToggleSelect,
  onAction,
}: {
  skill: EngineElement;
  /** 可以同时展开多个技能，无障碍标签要说清是哪个技能的哪个助手。 */
  skillName: string;
  assignments: SkillAgentAssignment[];
  selectedKeys: ReadonlySet<string>;
  /** 批量执行在途时禁用勾选。 */
  selectDisabled: boolean;
  onToggleSelect: (key: string) => void;
  onAction: (skill: EngineElement, agent: string, action: string, cell: SkillAgentCell | null) => void;
}) {
  // 引擎没报告的助手 = 跟这个技能无关，上面矩阵那格已经是个「·」，这里不必再占位置
  const related = assignments.filter(({ cell }) => cell !== null);

  return (
    <ul className="flex flex-wrap gap-x-6 gap-y-1.5">
      {related.map(({ key, agent, cell }) => {
        const actions = cell?.actions ?? [];
        return (
          <li key={key} className="flex items-center gap-2 text-xs">
            <input
              type="checkbox"
              aria-label={`选择 ${skillName} @ ${agent}`}
              data-testid={`select-${skill.id}-${agent}`}
              disabled={selectDisabled || actions.length === 0}
              checked={selectedKeys.has(key)}
              onChange={() => onToggleSelect(key)}
              className="h-3.5 w-3.5 accent-primary disabled:opacity-40"
            />
            <span className="font-mono text-foreground">{agent}</span>
            {actions.length === 0 ? (
              <span className="text-muted-foreground">
                {cell?.blocked_reason ?? (cell?.state === "unsupported" ? "没分配给它" : "没有可做的")}
              </span>
            ) : (
              SKILL_ACTIONS.filter(({ action }) => actions.includes(action)).map(({ action, label, className }) => {
                const extraView = cell?.state === "extra-link" ? EXTRA_LINK_ACTION_VIEW[action] : undefined;
                return (
                  <button
                    key={action}
                    type="button"
                    data-testid={`action-${action}-${skill.id}-${agent}`}
                    title={extraView?.title}
                    onClick={() => onAction(skill, agent, action, cell)}
                    className={cn("rounded border px-2 py-0.5", className)}
                  >
                    {extraView?.label ?? label}
                  </button>
                );
              })
            )}
            {cell?.blocked_reason && actions.length > 0 && (
              <span className="text-warning" title={cell.blocked_reason}>
                {cell.blocked_reason}
              </span>
            )}
          </li>
        );
      })}
    </ul>
  );
}
