import type { EngineElement, StatusData, Envelope } from "./contract";

/**
 * 全局同步计划：把各域 status 归成「点一下就能做」和「要你先决定」两堆。
 *
 * 为什么需要：配置分散在六个域，用户得挨个页面看有没有差异、再各点一次同步。这里在一个
 * 地方汇总，安全的批量做掉，需要决策的明确列出来并指向该去哪儿。
 *
 * 判定只依据引擎给的 state 与 actions，不自己推断：
 * - 元素级 actions 含 sync（rules/hooks/aliases/profiles）→ 整个目标待写入
 * - 逐助手的域（mcp/skills）看单元格：状态不良且该格给了 update/add 才算待办
 * - 被外部改过（unmanaged-detected / leftover / 带 blocked_reason 且无动作）归入「要你决定」，
 *   绝不混进批量——那些要用户看清代价后显式接管
 * - 用户主动停用的（disabled / not-assigned / absent）不催
 */

/** 这些状态表示「就该是这样」，不算待办。 */
const HEALTHY_STATES = new Set([
  "managed-ok",
  "linked",
  "absent",
  "not-assigned",
  "disabled",
  "agent-disabled",
  "unsupported",
]);

/** 这些状态是「被外部改过 / 旧体系残留」，需要用户显式决定，不能批量替他做主。 */
const DECISION_STATES = new Set(["unmanaged-detected", "leftover", "conflict", "shared"]);

export interface SyncItem {
  /** 元素 id，用于去重与展示。 */
  id: string;
  domain: string;
  label: string;
  /** 为什么待办，一句人话。 */
  reason: string;
  /** 执行用的 apply 参数。 */
  args: string[];
}

export interface DecisionItem {
  id: string;
  domain: string;
  label: string;
  reason: string;
}

export interface SyncPlan {
  /** 可以一键做掉的。 */
  ready: SyncItem[];
  /** 要用户先决定的（多半是被外部改过，得看清代价再接管）。 */
  decisions: DecisionItem[];
}

const STATE_REASON: Record<string, string> = {
  missing: "还没写入目标文件",
  "not-materialized": "还没写入目标文件",
  "missing-link": "还没在助手目录里生效",
  drifted: "内容和这里的配置不一致",
  outdated: "内容该更新了",
  "dead-link": "链接指向的东西不见了",
  "unmanaged-detected": "被本工具以外改过，同步已停下",
  leftover: "以前配过、现在没在管",
  conflict: "和已有内容冲突",
  shared: "多处共用同一份，状态异常",
};

function reasonOf(state: string): string {
  return STATE_REASON[state] ?? `状态为 ${state}`;
}

function actionIds(actions: unknown): string[] {
  if (!Array.isArray(actions)) return [];
  return actions.map((item) =>
    typeof item === "string" ? item : String((item as { id?: unknown })?.id ?? ""),
  );
}

interface AgentCell {
  state?: string;
  assigned?: boolean;
  actions?: string[];
  blocked_reason?: string | null;
}

/** 单个元素 → 待办项（可能产出多条：逐助手的域每个助手各一条）。 */
function itemsFor(element: EngineElement): { ready: SyncItem[]; decisions: DecisionItem[] } {
  const ready: SyncItem[] = [];
  const decisions: DecisionItem[] = [];
  const domain = element.domain;
  const observed = element.observed as { agents?: Record<string, AgentCell> };
  const cells = observed.agents;

  // 逐助手的域（mcp / skills）：以单元格为准，元素级状态是聚合结果
  if (cells && Object.keys(cells).length > 0) {
    for (const [agent, cell] of Object.entries(cells)) {
      const state = cell?.state ?? "";
      if (!state || HEALTHY_STATES.has(state)) continue;
      const acts = actionIds(cell?.actions);
      if (DECISION_STATES.has(state)) {
        decisions.push({ id: `${element.id}@${agent}`, domain, label: `${element.id} @ ${agent}`, reason: reasonOf(state) });
        continue;
      }
      const action = acts.includes("update") ? "update" : acts.includes("add") ? "add" : null;
      if (!action) {
        decisions.push({
          id: `${element.id}@${agent}`,
          domain,
          label: `${element.id} @ ${agent}`,
          reason: cell?.blocked_reason ?? reasonOf(state),
        });
        continue;
      }
      ready.push({
        id: `${element.id}@${agent}`,
        domain,
        label: `${element.id} @ ${agent}`,
        reason: reasonOf(state),
        args: ["--only", element.id, "--agent", agent, "--action", action],
      });
    }
    return { ready, decisions };
  }

  // 整目标的域（rules / hooks / aliases / profiles）
  const state = element.state;
  if (HEALTHY_STATES.has(state)) return { ready, decisions };
  const acts = actionIds(element.actions);
  if (acts.includes("sync")) {
    ready.push({ id: element.id, domain, label: element.id, reason: reasonOf(state), args: ["--only", element.id] });
  } else {
    const blocked = (element.observed as { blocked_reason?: string | null }).blocked_reason;
    decisions.push({ id: element.id, domain, label: element.id, reason: blocked ?? reasonOf(state) });
  }
  return { ready, decisions };
}

/** 把若干域的 status 结果汇成一份同步计划。查询失败或被拒的域直接跳过，不假装它没事。 */
export function buildSyncPlan(results: (Envelope<StatusData> | undefined)[]): SyncPlan {
  const ready: SyncItem[] = [];
  const decisions: DecisionItem[] = [];
  for (const envelope of results) {
    if (!envelope?.ok) continue;
    for (const element of envelope.data.elements) {
      const part = itemsFor(element);
      ready.push(...part.ready);
      decisions.push(...part.decisions);
    }
  }
  const byId = (list: { id: string }[]) => [...new Map(list.map((item) => [item.id, item])).values()];
  return { ready: byId(ready) as SyncItem[], decisions: byId(decisions) as DecisionItem[] };
}

/** 按域分组，展示与执行都按固定顺序，结果可预期。 */
export const SYNC_DOMAIN_ORDER = ["rules", "hooks", "aliases", "profiles", "mcp", "skills"] as const;

export function groupByDomain<T extends { domain: string }>(items: T[]): { domain: string; items: T[] }[] {
  return SYNC_DOMAIN_ORDER.map((domain) => ({
    domain,
    items: items.filter((item) => item.domain === domain),
  })).filter((group) => group.items.length > 0);
}

export const SYNC_DOMAIN_LABEL: Record<string, string> = {
  rules: "规则",
  hooks: "Hooks",
  aliases: "终端快捷命令",
  profiles: "实例",
  mcp: "MCP",
  skills: "技能",
};
