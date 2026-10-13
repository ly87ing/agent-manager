import { t } from "./i18n";
import type { RunOperation } from "./contract";

/**
 * 操作记录里哪些能撤回、怎么撤回。只做不需要保留原内容就能干净回到原状的：
 * - 技能的分配与启停（记录带「集合/技能@账号」）：做反向动作；
 * - 规则覆盖且当时另存了备份模板的：把备份写回（写入前照常核对文件）；
 * - 纳入管理：按当时的备份清单放回原目录、撤掉链接与登记。
 * 其余写入没有保留原内容——为了不把可能含凭据的文件原文存进库——不能自动撤回。
 */
export type RollbackPlan =
  | { kind: "skills"; label: string; elementId: string; agent: string; action: string }
  | { kind: "rules"; label: string; target: string; template: string }
  | { kind: "adopt"; label: string; backup: string }
  | { kind: "none"; reason: string };

const SKILL_REVERSE: Record<string, { action: string; verb: string }> = {
  "skills:add": { action: "remove", get verb() { return t("取消分配"); } },
  "skills:enable": { action: "disable", get verb() { return t("停用"); } },
  "skills:disable": { action: "enable", get verb() { return t("重新启用"); } },
};

export function rollbackPlan(operation: RunOperation): RollbackPlan {
  if (operation.operation_type === "skills:add" && (operation.detail as { replaced_from?: unknown } | null)?.replaced_from) {
    return { kind: "none", reason: t("这是来源切换，不能通过移除新分配恢复旧版本；请在技能页选择所需来源") };
  }
  const reverse = SKILL_REVERSE[operation.operation_type];
  if (reverse) {
    const match = /^([^/@]+)\/([^@]+)@(.+)$/.exec(operation.target ?? "");
    if (!match) return { kind: "none", reason: t("记录里没有完整的技能与账号") };
    const [, collection, skill, agent] = match;
    return { kind: "skills", label: `${reverse.verb} ${skill} @ ${agent}`, elementId: `skill:${collection}/${skill}`, agent, action: reverse.action };
  }
  if (operation.operation_type === "skills:adopt") {
    const backup = (operation.detail as { backup?: string } | null)?.backup;
    if (!backup) return { kind: "none", reason: t("记录里没有备份编号") };
    return { kind: "adopt", label: t("撤回纳入 {0}：放回原来的目录，撤掉这次的链接与登记", operation.target ?? ""), backup };
  }
  if (operation.operation_type === "rules:apply") {
    const detail = (operation.detail ?? {}) as { action?: string; backup_template?: string | null };
    if (detail.action === "write" && detail.backup_template && operation.target) {
      return { kind: "rules", label: t("把 {0} 恢复成覆盖前的内容", operation.target), target: operation.target, template: detail.backup_template };
    }
    return { kind: "none", reason: t("覆盖前的内容是已知模板或来源里的规则，没有另存备份；到规则页选回那一条即可") };
  }
  return { kind: "none", reason: t("这次写入没有保留原内容，不能自动撤回") };
}
