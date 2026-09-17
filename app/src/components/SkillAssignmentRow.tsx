import type { EngineElement } from "../lib/contract";
import type { SkillAgentCell } from "../lib/observed";
import { cn } from "../lib/utils";

function shortCommit(value: string | null | undefined): string {
  if (!value) return "?";
  return value.length > 12 ? value.slice(0, 7) : value;
}

/**
 * 技能 × 助手 分配状态徽标。异常态（冲突/共用/不一致/不支持）优先于
 * assigned/enabled 标志显示，enabled=true 不能掩盖异常；cell 缺失（引擎未报告
 * 此助手路径）显示「未配置」，不伪造可执行状态。
 */
export function SkillStatusBadge({ agent, cell }: { agent: string; cell: SkillAgentCell | null }) {
  let label = "未配置";
  let state = "未配置";
  let className = "border bg-muted text-muted-foreground";

  if (cell) {
    state = cell.state;
    label = "未分配";
    className = "border bg-muted text-muted-foreground";

    if (state === "conflict") {
      label = "冲突";
      className = "border-rose/30 bg-rose/10 text-rose";
    } else if (state === "shared") {
      label = "共用（异常）";
      className = "border-warning/30 bg-warning/10 text-warning";
    } else if (state === "drifted") {
      label = "不一致";
      className = "border-warning/30 bg-warning/10 text-warning";
    } else if (state === "unsupported") {
      label = "不支持";
      className = "border-input bg-muted text-muted-foreground";
    } else if (state === "dead-link") {
      label = "失效链接";
      className = "border-rose/30 bg-rose/10 text-rose";
    } else if (state === "missing-link") {
      label = "未生效";
      className = "border-warning/30 bg-warning/10 text-warning";
    } else if (state === "not-materialized") {
      label = "尚未生成";
      className = "border bg-muted text-muted-foreground";
    } else if (state === "extra-link") {
      label = "计划外链接";
      className = "border-violet/30 bg-violet/10 text-violet";
    } else if (state === "disabled") {
      label = "已停用";
      className = "border bg-muted text-muted-foreground";
    } else if (state === "linked") {
      label = "已启用";
      className = "border-success/30 bg-success/10 text-success";
    } else if (cell.assigned && cell.enabled === false) {
      label = "已停用";
      className = "border bg-muted text-muted-foreground";
    } else if (cell.assigned && cell.enabled) {
      label = "已分配";
      className = "border-success/30 bg-success/10 text-success";
    }
  }

  return (
    <span
      title={`${agent}: ${state}`}
      className={cn("inline-flex items-center rounded border px-2 py-0.5 text-xs font-medium", className)}
    >
      {label}
    </span>
  );
}

const SKILL_ACTIONS: { action: string; label: string; className: string }[] = [
  { action: "add", label: "添加", className: "border-primary/30 bg-primary/10 text-primary hover:bg-primary/15" },
  { action: "update", label: "更新", className: "border-primary/30 bg-primary/10 text-primary hover:bg-primary/15" },
  { action: "enable", label: "启用", className: "border-success/30 bg-success/10 text-success hover:bg-success/15" },
  { action: "disable", label: "禁用", className: "border-warning/30 bg-warning/10 text-warning hover:bg-warning/15" },
  { action: "remove", label: "删除分配", className: "border-destructive/30 bg-card text-destructive hover:bg-destructive/10" },
];

/** 计划外链接行的动作文案：add=纳入管理（接管既有链接），remove=取消链接（仅移除入口）。 */
const EXTRA_LINK_ACTION_VIEW: Record<string, { label: string; title: string }> = {
  add: { label: "纳入管理", title: "保存当前技能内容并接管既有链接" },
  remove: { label: "取消链接", title: "移除磁盘上的链接入口，不删除技能本身" },
};

/** 「当前分配版本」列无版本时的链接态集合：仅这些状态确实存在链接入口。
 * 引擎恒输出非空 link_path（目标路径而非存在性），不能据此判「已链接」。 */
const LINKED_CELL_STATES = new Set(["linked", "dead-link", "drifted"]);

/**
 * 全部技能列表中的一行（技能 × 助手）。操作按钮严格按引擎报告的
 * cell.actions 子集渲染（add/update/enable/disable/remove），更新与启用不混；
 * 计划外链接（extra-link）行的 add/remove 改用 纳入管理/取消链接 文案；
 * cell 缺失时不伪造操作，明确显示未配置。
 */
export function SkillAssignmentRow({
  skill,
  skillName,
  collection,
  agent,
  cell,
  onFilterCollection,
  onFilterAgent,
  onAction,
}: {
  skill: EngineElement;
  skillName: string;
  collection: string;
  agent: string;
  cell: SkillAgentCell | null;
  onFilterCollection: (collection: string) => void;
  onFilterAgent: (agent: string) => void;
  onAction: (skill: EngineElement, agent: string, action: string, cell: SkillAgentCell | null) => void;
}) {
  const actions = cell?.actions ?? [];

  return (
    <tr className="border-b last:border-0 hover:bg-muted/50">
      <td className="py-2.5 pr-4">
        <span className="font-mono text-xs font-medium text-foreground">{skillName}</span>
      </td>
      <td className="py-2.5 pr-4">
        <button
          type="button"
          onClick={() => onFilterCollection(collection)}
          className="font-mono text-xs text-muted-foreground hover:text-primary hover:underline"
          title={`筛选仅查看集合 ${collection}`}
        >
          {collection}
        </button>
      </td>
      <td className="py-2.5 pr-4 font-mono text-xs text-foreground">
        <button
          type="button"
          onClick={() => onFilterAgent(agent)}
          className="hover:text-primary hover:underline"
          title={`筛选仅查看助手 ${agent}`}
        >
          {agent}
        </button>
      </td>
      <td className="py-2.5 pr-4 font-mono text-xs whitespace-nowrap text-muted-foreground" title={cell?.available_version ?? undefined}>
        {cell?.available_version ? shortCommit(cell.available_version) : "未获取"}
      </td>
      <td className="py-2.5 pr-4 font-mono text-xs whitespace-nowrap text-muted-foreground" title={cell?.version ?? undefined}>
        {/* 分配语义优先：extra-link 虽有链接但属未分配，链接存在由状态徽章表达。 */}
        {cell ? (cell.version ? shortCommit(cell.version) : cell.state === "extra-link" ? "未分配" : LINKED_CELL_STATES.has(cell.state) ? "已链接" : cell.assigned ? "已分配" : "未分配") : "—"}
      </td>
      <td className="py-2.5 pr-4">
        <div className="flex flex-col gap-1">
          <SkillStatusBadge agent={agent} cell={cell} />
          {cell?.blocked_reason && (
            <span className="text-xs text-warning" title={cell.blocked_reason}>
              {cell.blocked_reason}
            </span>
          )}
        </div>
      </td>
      <td className="py-2.5">
        <div className="flex flex-wrap items-center gap-1.5">
          {SKILL_ACTIONS.filter(({ action }) => actions.includes(action)).map(({ action, label, className }) => {
            const extraView = cell?.state === "extra-link" ? EXTRA_LINK_ACTION_VIEW[action] : undefined;
            return (
              <button
                key={action}
                type="button"
                data-testid={`action-${action}-${skill.id}-${agent}`}
                title={extraView?.title}
                onClick={() => onAction(skill, agent, action, cell)}
                className={cn("rounded border px-2 py-0.5 text-xs", className)}
              >
                {extraView?.label ?? label}
              </button>
            );
          })}
          {cell && actions.length === 0 && <span className="text-xs text-muted-foreground">当前引擎未提供操作</span>}
          {!cell && <span className="text-xs text-muted-foreground">未配置</span>}
        </div>
      </td>
    </tr>
  );
}
