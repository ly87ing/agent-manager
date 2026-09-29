import type { ChipTone } from "../components/AccountChip";
import type { SkillAgentCell } from "./observed";

/** update 是引擎能力，不等于当前需要更新；只对版本变化或链接损坏提示。 */
export function skillNeedsUpdate(cell: SkillAgentCell | null): boolean {
  if (!cell?.actions?.includes("update")) return false;
  if (cell.state === "missing-link" || cell.state === "dead-link") return true;
  if (cell.state === "drifted") return false;
  return Boolean(cell.available_version && cell.version !== cell.available_version);
}

export interface ChipAction {
  action: string;
  label: string;
}

/**
 * 技能在一个账号上的开关：主动作放在开关上（点一下），其余可做的放进「⋯」。
 * 动作只取引擎在 cell.actions 里给出的；已分配技能的 remove 与可逆的停用重复，不暴露。
 */
export function skillChip(cell: SkillAgentCell): { tone: ChipTone; hint: string; primary: ChipAction | null; secondary: ChipAction[] } {
  const actions = cell.actions ?? [];
  const has = (action: string) => actions.includes(action);
  if (cell.source_missing) {
    return {
      tone: "warn",
      hint: "来源里已经没有这个技能，点击移除这条分配",
      primary: has("remove") ? { action: "remove", label: "移除" } : null,
      secondary: [],
    };
  }
  if (cell.state === "extra-link") {
    return {
      tone: "warn",
      hint: "技能目录里有一条本工具以外的链接，点击纳入管理",
      primary: has("add") ? { action: "add", label: "纳入管理" } : null,
      secondary: has("remove") ? [{ action: "remove", label: "取消这条外部链接" }] : [],
    };
  }
  if ((cell.state === "missing-link" || cell.state === "dead-link") && has("update")) {
    return {
      tone: "warn",
      hint: "链接损坏，点击修复",
      primary: { action: "update", label: "修复" },
      secondary: has("disable") ? [{ action: "disable", label: "停用" }] : [],
    };
  }
  if (cell.state === "drifted") {
    return {
      tone: "warn",
      hint: cell.blocked_reason ?? "入口被本工具以外的内容占用",
      primary: null,
      secondary: has("disable") ? [{ action: "disable", label: "停用" }] : [],
    };
  }
  const update = skillNeedsUpdate(cell) ? [{ action: "update", label: "更新到新版本" }] : [];
  if (cell.state === "linked" && has("disable")) {
    return { tone: "on", hint: update.length ? "已启用，有新版本；点击停用" : "已启用，点击停用", primary: { action: "disable", label: "停用" }, secondary: update };
  }
  if (cell.state === "disabled" && has("enable")) {
    return { tone: "off", hint: "已停用，点击启用", primary: { action: "enable", label: "启用" }, secondary: update };
  }
  if (cell.state === "not-assigned" && has("add")) {
    return { tone: "off", hint: "未启用，点击启用", primary: { action: "add", label: "启用" }, secondary: [] };
  }
  return {
    tone: cell.state === "linked" ? "on" : "blocked",
    hint: cell.blocked_reason ?? "这里没有可做的",
    primary: null,
    secondary: update,
  };
}
