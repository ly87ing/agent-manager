import type { EngineElement, Envelope, StatusData } from "./contract";
import type { AgentUsage, SkillAgentCell } from "./observed";
import { isStale, tightestWindow } from "./agentUsage";
import { skillNeedsUpdate } from "../components/SkillAgentActions";

/**
 * 总览清单模型：本机有哪些助手 / MCP / 技能 / 规则 / Hooks / 快捷命令，各自什么版本、
 * 要不要更新、同步到位没有。
 *
 * 只出这四件事，不出路径、摘要、逐助手明细——那些在各自页面里看。判定全部依据引擎给的
 * state / actions / freshness，不自己推断。
 */

/** 更新轴：引擎 freshness 的四种取值收敛成三种展示，not-applicable 表示这类东西没有版本概念。 */
export type UpdateState = "current" | "behind" | "unknown" | "none";

/** 同步轴：已到位 / 待同步（可一键做）/ 要你决定（被外部改过，不能替用户做主）/ 没装 / 已停用。 */
export type SyncState = "ok" | "pending" | "decision" | "absent" | "disabled";

export interface OverviewItem {
  id: string;
  name: string;
  domain: string;
  version: string | null;
  upstream: string | null;
  update: UpdateState;
  sync: SyncState;
  /** 待同步时的 apply 参数；要你决定的没有（那条路要用户自己走）。 */
  syncArgs?: string[];
  /** 要你决定时的一句原因。 */
  reason?: string;
  /** 助手的额度：最吃紧那个窗口的已用百分比，没有本地记账的为 null。 */
  usedPercent?: number | null;
  /** 额度数字是否已经过旧（助手很久没记账）。 */
  usageStale?: boolean;
}

export interface OverviewGroup {
  domain: string;
  label: string;
  items: OverviewItem[];
  /** 需要处理的条数（待同步 + 要你决定）。 */
  pending: number;
}

export const OVERVIEW_DOMAINS: { domain: string; label: string }[] = [
  { domain: "agents", label: "助手" },
  { domain: "profiles", label: "实例" },
  { domain: "mcp", label: "MCP" },
  { domain: "skills", label: "技能" },
  { domain: "rules", label: "规则" },
  { domain: "hooks", label: "Hooks" },
  { domain: "aliases", label: "终端快捷命令" },
];

/** 这些状态表示「就该是这样」。 */
const HEALTHY = new Set(["managed-ok", "linked", "absent", "not-assigned", "agent-disabled", "unsupported"]);
/** 被外部改过 / 旧体系残留：不能替用户做主。 */
const DECISION = new Set(["unmanaged-detected", "leftover", "conflict", "shared"]);

function actionIds(actions: unknown): string[] {
  if (!Array.isArray(actions)) return [];
  return actions.map((a) => (typeof a === "string" ? a : String((a as { id?: unknown })?.id ?? "")));
}

function updateOf(element: EngineElement): { update: UpdateState; version: string | null; upstream: string | null } {
  const f = element.freshness;
  // 助手的版本是装出来的（observed.version），查新版本才走 freshness；两处都要认。
  const version = f.current ?? (element.observed as { version?: string | null }).version ?? null;
  const upstream = f.upstream ?? null;
  if (f.state === "behind") return { update: "behind", version, upstream };
  if (f.state === "current" || f.state === "ahead") return { update: "current", version, upstream };
  if (f.state === "unknown") return { update: "unknown", version, upstream };
  return { update: "none", version, upstream };
}

interface Cell {
  state?: string;
  actions?: string[];
  blocked_reason?: string | null;
}

/** 逐助手的域（mcp；技能另见 skillSyncOf）：把各助手单元格聚合成这个资源的一个同步状态。 */
function syncFromCells(element: EngineElement, cells: Record<string, Cell>): Pick<OverviewItem, "sync" | "syncArgs" | "reason"> {
  let pendingArgs: string[] | null = null;
  let decisionReason: string | null = null;
  let sawAny = false;
  for (const [agent, cell] of Object.entries(cells)) {
    const state = cell?.state ?? "";
    if (!state || state === "absent" || state === "not-assigned" || state === "unsupported") continue;
    sawAny = true;
    if (state === "disabled" || state === "agent-disabled" || HEALTHY.has(state)) continue;
    if (DECISION.has(state)) {
      decisionReason ??= cell?.blocked_reason ?? "被本工具以外改过";
      continue;
    }
    const acts = actionIds(cell?.actions);
    const action = acts.includes("update") ? "update" : acts.includes("add") ? "add" : null;
    if (action) {
      pendingArgs ??= ["--only", element.id, "--agent", agent, "--action", action];
    } else {
      decisionReason ??= cell?.blocked_reason ?? "暂时没有可执行的操作";
    }
  }
  if (decisionReason) return { sync: "decision", reason: decisionReason };
  if (pendingArgs) return { sync: "pending", syncArgs: pendingArgs };
  return { sync: sawAny ? "ok" : "ok" };
}

/**
 * 技能的同步轴。只有两种算待同步：用户已添加的集合有可用更新（集合 freshness=behind），
 * 以及已启用的技能链接缺失/失效或来源有新内容（与技能页「更新/修复」同一判定）。
 * 推荐来源不是元素，永不计入；没拉取的集合、没启用到任何助手的技能是「没装」而不是待办；
 * 未分配的外部同名链接不是待处理项；链接被本工具以外改过的要你决定。
 */
function skillSyncOf(element: EngineElement): Pick<OverviewItem, "sync" | "syncArgs" | "reason"> {
  if (!isSingleSkill(element)) {
    if (element.state === "disabled") return { sync: "disabled" };
    if (element.freshness.state === "behind" && actionIds(element.actions).includes("update")) {
      return { sync: "pending", syncArgs: ["--only", element.id, "--action", "update"] };
    }
    if (element.state === "not-materialized") return { sync: "absent" };
    return { sync: "ok" };
  }
  const cells = (element.observed as { agents?: Record<string, SkillAgentCell> }).agents ?? {};
  let pendingArgs: string[] | null = null;
  let decisionReason: string | null = null;
  let anyAssigned = false;
  let anyEnabled = false;
  for (const [agent, cell] of Object.entries(cells)) {
    if (!cell?.assigned) continue;
    anyAssigned = true;
    if (cell.enabled) anyEnabled = true;
    if (cell.state === "drifted") {
      decisionReason ??= cell.blocked_reason ?? "链接被本工具以外改过";
    } else if (skillNeedsUpdate(cell)) {
      pendingArgs ??= ["--only", element.id, "--agent", agent, "--action", "update"];
    } else if (cell.state === "missing-link" || cell.state === "dead-link") {
      decisionReason ??= cell.blocked_reason ?? "来源里找不到这个技能，补不了链接";
    }
  }
  if (decisionReason) return { sync: "decision", reason: decisionReason };
  if (pendingArgs) return { sync: "pending", syncArgs: pendingArgs };
  if (!anyAssigned) return { sync: "absent" };
  return { sync: anyEnabled ? "ok" : "disabled" };
}

function itemOf(element: EngineElement, name: string): OverviewItem {
  // AI CLI 的软件生命周期交还给各自安装渠道；总览只保留助手状态与额度，
  // 版本和安装来源集中到「设置 → 客户端诊断」只读展示。
  const { update, version, upstream } =
    element.domain === "agents"
      ? { update: "none" as const, version: null, upstream: null }
      : updateOf(element);
  const usage = (element.observed as { usage?: AgentUsage | null }).usage ?? null;
  const base = {
    id: element.id,
    name,
    domain: element.domain,
    version,
    upstream,
    update,
    usedPercent: tightestWindow(usage)?.used_percent ?? null,
    usageStale: usage ? isStale(usage.checked_at) : false,
  };
  if (element.domain === "skills") return { ...base, ...skillSyncOf(element) };
  const observed = element.observed as { agents?: Record<string, Cell>; blocked_reason?: string | null };

  if (observed.agents && Object.keys(observed.agents).length > 0) {
    return { ...base, ...syncFromCells(element, observed.agents) };
  }
  if (element.state === "disabled") return { ...base, sync: "disabled" };
  if (HEALTHY.has(element.state)) return { ...base, sync: "ok" };
  if (element.state === "missing") return { ...base, sync: "absent" };
  const acts = actionIds(element.actions);
  if (acts.includes("sync")) {
    return { ...base, sync: "pending", syncArgs: ["--only", element.id] };
  }
  return { ...base, sync: "decision", reason: observed.blocked_reason ?? "被本工具以外改过" };
}

/** 别名单独处理：用户关心的是「有哪些快捷命令、写进终端没有」，不是 zshrc/zprofile 两个文件。 */
function aliasItems(envelope: Envelope<StatusData> | undefined): OverviewItem[] {
  if (!envelope?.ok) return [];
  const byName = new Map<string, { applied: boolean; blocked: boolean }>();
  for (const element of envelope.data.elements) {
    const entries = (element.observed as { entries?: { name: string; state: string }[] }).entries ?? [];
    const blocked = (element.observed as { blocked_reason?: string | null }).blocked_reason != null;
    for (const entry of entries) {
      const prev = byName.get(entry.name) ?? { applied: true, blocked: false };
      byName.set(entry.name, {
        applied: prev.applied && entry.state === "applied",
        blocked: prev.blocked || blocked,
      });
    }
  }
  return [...byName.entries()]
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([name, info]) => ({
      id: `alias:${name}`,
      name,
      domain: "aliases",
      version: null,
      upstream: null,
      update: "none" as const,
      sync: info.blocked ? ("decision" as const) : info.applied ? ("ok" as const) : ("pending" as const),
      // 别名是整块区块写入的，逐条没有单独的 apply 参数
      syncArgs: info.blocked || info.applied ? undefined : [],
      reason: info.blocked ? "本工具写的那段被外面改过" : undefined,
    }));
}

/** 技能只列单个技能（skill:集合/名称），集合本身不占一行——用户问的是「有哪些技能」。 */
function isSingleSkill(element: EngineElement): boolean {
  return element.id.startsWith("skill:") && element.id.includes("/");
}

/**
 * 技能来源还没拉下来时展不开单个技能。这时候留下来源那一行，否则整段技能凭空消失，
 * 用户看不出「配了个来源但还没生效」。来源有可用更新时也留一行：更新是按来源做的。
 */
function skillElements(elements: EngineElement[]): EngineElement[] {
  const singles = elements.filter(isSingleSkill);
  const covered = new Set(singles.map((element) => element.id.slice(0, element.id.lastIndexOf("/"))));
  const bare = elements.filter(
    (element) => !isSingleSkill(element) && (!covered.has(element.id) || element.freshness.state === "behind"),
  );
  return [...bare, ...singles];
}

function displayName(element: EngineElement): string {
  const spec = element.spec as { display_name?: string; name?: string };
  if (spec.display_name) return spec.display_name;
  if (spec.name) return spec.name;
  return element.id.replace(/^[a-z]+:/, "");
}

export function buildOverview(results: Record<string, Envelope<StatusData> | undefined>): OverviewGroup[] {
  return OVERVIEW_DOMAINS.map(({ domain, label }) => {
    let items: OverviewItem[];
    if (domain === "aliases") {
      items = aliasItems(results.aliases);
    } else {
      const envelope = results[domain];
      const all = envelope?.ok ? envelope.data.elements : [];
      const elements = domain === "skills" ? skillElements(all) : all;
      items = elements.map((element) => itemOf(element, displayName(element)));
    }
    const pending = items.filter((item) => item.sync === "pending" || item.sync === "decision").length;
    return { domain, label, items, pending };
  }).filter((group) => group.items.length > 0);
}

/**
 * 顶部一键同步要执行的那些（要你决定的不进批量）。
 *
 * 同参数的合成一次：快捷命令是整块写入的，三条待同步只需要执行一次 aliases:apply。
 */
export function syncableOf(groups: OverviewGroup[]): { domain: string; name: string; args: string[] }[] {
  const byCommand = new Map<string, { domain: string; name: string; args: string[]; count: number }>();
  for (const group of groups) {
    for (const item of group.items) {
      if (item.sync !== "pending" || item.syncArgs === undefined) continue;
      const key = `${item.domain} ${item.syncArgs.join(" ")}`;
      const existing = byCommand.get(key);
      if (existing) {
        existing.count += 1;
        existing.name = `${group.label} ${existing.count} 项`;
      } else {
        byCommand.set(key, { domain: item.domain, name: item.name, args: item.syncArgs, count: 1 });
      }
    }
  }
  return [...byCommand.values()].map(({ domain, name, args }) => ({ domain, name, args }));
}

export const UPDATE_LABEL: Record<UpdateState, string> = {
  current: "已是最新",
  behind: "可更新",
  unknown: "未检查",
  none: "",
};

export const SYNC_LABEL: Record<SyncState, string> = {
  ok: "已同步",
  pending: "待同步",
  decision: "要你决定",
  absent: "没装",
  disabled: "已停用",
};
