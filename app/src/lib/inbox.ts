import { buildAccounts } from "./accounts";
import type { EngineElement, McpTarget, StatusData } from "./contract";
import { mcpFillRows, type McpFillRow } from "./mcpFill";
import { missingVariables } from "./mcpVariables";
import type {
  AliasesObserved,
  McpObserved,
  RulesObserved,
  RulesSpec,
  SkillCollectionObserved,
  SkillCollectionSpec,
  SkillObserved,
  SkillSpec,
} from "./observed";
import { ignoreKey, useIgnored } from "./inboxIgnore";
import { useEngineQuery } from "./queries";
import { rulesFillRows, type RulesFillRow } from "./rulesFill";
import { isCollection } from "./skillCollections";
import { addToAllRows, fillRows, versionUpdateRows } from "./skillFill";
import type { BatchApplyRow } from "./useBatchApply";

/**
 * 管理闭环（docs/MULTI-ACCOUNT-DESIGN.md「管理体系」）：各领域的实际状态统一成同一套「需要处理」的状态，
 * 每条只对应一个处理动作；动作经写入管线执行、回读后状态回到正常，这一条就从待处理里消失。
 * 导航角标、待处理页都只从这里算，页面里不再各算各的。数据全部来自只读的 status 查询。
 */
export type InboxState = "broken" | "drifted" | "missing" | "update" | "unmanaged";

export const INBOX_STATES: { state: InboxState; label: string; hint: string }[] = [
  { state: "broken", label: "损坏", hint: "链接断了、格式无效、找不到命令或读不了" },
  { state: "drifted", label: "已偏离", hint: "被外部改过，和来源或预期不一致" },
  { state: "missing", label: "缺失", hint: "同类账号在用，这些账号还没有" },
  { state: "update", label: "有新版本", hint: "来源有新内容" },
  { state: "unmanaged", label: "未纳管", hint: "在账号里，但不来自任何来源" },
];

/** 处理一条待处理项的唯一动作：能直接执行的直接执行，需要看差异或选择的跳到负责的页面。 */
export type InboxFix =
  | { kind: "skills"; label: string; rows: BatchApplyRow[] }
  | { kind: "mcp-fill"; label: string; rows: McpFillRow[] }
  | { kind: "rules-fill"; label: string; rows: RulesFillRow[] }
  | { kind: "open"; label: string; target: string };

export type InboxPage = "accounts" | "skills" | "mcp" | "rules" | "commands" | "sources";

export interface InboxItem {
  id: string;
  /** 这一项落在哪个元素上（引擎元素 id；命令为 `command:<名称>`）：资源页的「只看待处理」按它筛选。 */
  subject: string;
  page: InboxPage;
  state: InboxState;
  title: string;
  detail: string;
  fix: InboxFix;
  /** 同一原因、同一来源的项合成一行（例如某个来源里还没用的技能），点开看明细。 */
  group?: { key: string; title: string; open: string };
}

export interface InboxInput {
  skills: EngineElement[];
  unmanaged?: StatusData["unmanaged"] | null;
  mcp: EngineElement[];
  mcpTargets: McpTarget[];
  rules: EngineElement[];
  agents: EngineElement[];
  profiles: EngineElement[];
  aliases: EngineElement[];
  nameOf?: (agent: string) => string;
}

const PAGE_LABEL: Record<InboxPage, string> = {
  accounts: "账号",
  skills: "技能",
  mcp: "MCP",
  rules: "规则",
  commands: "命令",
  sources: "来源",
};

export function pageLabel(page: InboxPage): string {
  return PAGE_LABEL[page];
}

function names(agents: string[], nameOf: (agent: string) => string): string {
  return agents.map(nameOf).join("、");
}

/** 版本号原样显示；40 位提交号只留前 7 位。 */
export function shortRef(ref: string): string {
  return /^[0-9a-f]{40}$/.test(ref) ? ref.slice(0, 7) : ref;
}

function sourceItems(collections: EngineElement[]): InboxItem[] {
  return collections.flatMap((collection): InboxItem[] => {
    const name = collection.id.replace(/^skill:/, "");
    const spec = collection.spec as SkillCollectionSpec;
    const observed = collection.observed as SkillCollectionObserved;
    if ((collection.policy as { enabled?: boolean }).enabled === false) return [];
    const open = (label: string) => ({ kind: "open" as const, label, target: `sources:${name}` });
    const problem = observed.discovery_error?.message ?? observed.version?.track_error ?? observed.blocked_reason ?? null;
    if (problem) return [{ id: `source:${name}:broken`, subject: collection.id, page: "sources", state: "broken", title: `${name}`, detail: problem, fix: open("去处理") }];
    if (!observed.repo_cached) {
      return [{
        id: `source:${name}:fetch`,
        subject: collection.id,
        page: "sources",
        state: spec.source_path ? "broken" : "update",
        title: `${name}`,
        detail: spec.source_path ? `本机目录不存在：${spec.source_path}` : "还没有取回内容",
        fix: open(spec.source_path ? "去处理" : "取回"),
      }];
    }
    if (collection.freshness.state === "behind") {
      return [{
        id: `source:${name}:behind`,
        subject: collection.id,
        page: "sources",
        state: "update",
        title: `${name}`,
        detail: collection.freshness.upstream ? `可更新到 ${shortRef(collection.freshness.upstream)}` : "有新内容",
        fix: open("预览并更新"),
      }];
    }
    return [];
  });
}

function skillItems(elements: EngineElement[], nameOf: (agent: string) => string): InboxItem[] {
  const skills = elements.filter((element) => !isCollection(element));
  const fill = fillRows(elements);
  const versions = versionUpdateRows(elements);
  return skills.flatMap((skill): InboxItem[] => {
    const spec = skill.spec as SkillSpec;
    const name = spec.name ?? skill.id;
    const observed = skill.observed as SkillObserved;
    const cells = Object.entries(observed.agents ?? {});
    const where = (test: (state: string, cell: (typeof cells)[number][1]) => boolean) =>
      cells.filter(([, cell]) => test(cell.state, cell)).map(([agent]) => agent);
    const open = (label: string) => ({ kind: "open" as const, label, target: `skills:${name}` });
    const out: InboxItem[] = [];
    const dropped = where((_, cell) => Boolean(cell.source_missing));
    if (dropped.length) {
      out.push({ id: `skill:${skill.id}:dropped`, subject: skill.id, page: "skills", state: "broken", title: `${name}`, detail: `来源里已删除：${names(dropped, nameOf)}`, fix: open("去移除") });
    }
    const repair = cells
      .filter(([, cell]) => !cell.source_missing && (cell.state === "missing-link" || cell.state === "dead-link") && (cell.actions ?? []).includes("update"))
      .map(([agent]): BatchApplyRow => ({ elementId: skill.id, agent, action: "update", label: `修复 ${name} @ ${agent}`, takeover: false }));
    if (repair.length) {
      out.push({ id: `skill:${skill.id}:broken`, subject: skill.id, page: "skills", state: "broken", title: `${name}`, detail: `链接损坏：${names(repair.map((row) => row.agent), nameOf)}`, fix: { kind: "skills", label: "修复", rows: repair } });
    }
    const drifted = where((state, cell) => state === "drifted" && !cell.source_missing);
    if (drifted.length) {
      out.push({ id: `skill:${skill.id}:drifted`, subject: skill.id, page: "skills", state: "drifted", title: `${name}`, detail: `入口被外部内容占用：${names(drifted, nameOf)}`, fix: open("去查看") });
    }
    const extra = where((state) => state === "extra-link");
    if (extra.length) {
      out.push({ id: `skill:${skill.id}:extra`, subject: skill.id, page: "skills", state: "unmanaged", title: `${name}`, detail: `有外部链接：${names(extra, nameOf)}`, fix: open("去纳入") });
    }
    const missing = fill.filter((row) => row.elementId === skill.id);
    if (missing.length) {
      out.push({ id: `skill:${skill.id}:missing`, subject: skill.id, page: "skills", state: "missing", title: `${name}`, detail: `还没有：${names(missing.map((row) => row.agent), nameOf)}`, fix: { kind: "skills", label: "补齐", rows: missing } });
    }
    const behind = versions.filter((row) => row.elementId === skill.id);
    if (behind.length) {
      out.push({ id: `skill:${skill.id}:version`, subject: skill.id, page: "skills", state: "update", title: `${name}`, detail: `换到来源里的新版本：${names(behind.map((row) => row.agent), nameOf)}`, fix: { kind: "skills", label: "换到新版本", rows: behind } });
    }
    if (observed.discovery_inbox === true) {
      const rows = addToAllRows(skill);
      out.push({
        id: `skill:${skill.id}:new`,
        subject: skill.id,
        page: "skills",
        state: "update",
        title: `${name}`,
        detail: `来源 ${spec.collection} 里有、还没用的技能`,
        fix: rows.length ? { kind: "skills", label: "添加到全部账号", rows } : open("去查看"),
        group: { key: `new:${spec.collection}`, title: `${spec.collection} 里还没用的技能`, open: `skills:${spec.collection}` },
      });
    }
    return out;
  });
}

function unmanagedItems(view: StatusData["unmanaged"] | null | undefined): InboxItem[] {
  if (!view) return [];
  const open = { kind: "open" as const, label: "去纳入", target: "sources:unmanaged" };
  return [
    ...view.skills.map((skill): InboxItem => ({
      id: `unmanaged:${skill.name}`,
      subject: `unmanaged:${skill.name}`,
      page: "sources",
      state: "unmanaged",
      title: `${skill.name}`,
      detail: skill.source?.repository ? `装自 ${skill.source.repository}，不来自任何来源` : "不来自任何来源",
      fix: open,
      group: { key: "unmanaged", title: "不来自任何来源的技能", open: "sources:unmanaged" },
    })),
    ...view.duplicates.map((dup): InboxItem => ({
      id: `duplicate:${dup.agent}:${dup.name}`,
      subject: `unmanaged:${dup.name}`,
      page: "sources",
      state: "drifted",
      title: `${dup.name}`,
      detail: "Codex 会加载两份（账号目录与共用目录各一份）",
      fix: { ...open, label: "去清理" },
    })),
  ];
}

function mcpItems(elements: EngineElement[], targets: McpTarget[], nameOf: (agent: string) => string): InboxItem[] {
  const fill = mcpFillRows(elements, targets);
  return elements.flatMap((element): InboxItem[] => {
    const name = element.id.replace(/^mcp:/, "");
    const cells = Object.entries((element.observed as McpObserved).agents ?? {});
    const open = (label: string) => ({ kind: "open" as const, label, target: `mcp:${name}` });
    const out: InboxItem[] = [];
    const invalid = cells.filter(([, cell]) => cell.state === "invalid").map(([agent]) => agent);
    const noCommand = cells.filter(([, cell]) => cell.state === "enabled" && cell.command_available === false).map(([agent]) => agent);
    const vars = missingVariables(element);
    if (invalid.length) out.push({ id: `mcp:${name}:invalid`, subject: element.id, page: "mcp", state: "broken", title: `${name}`, detail: `条目无法识别：${names(invalid, nameOf)}`, fix: open("去查看") });
    if (noCommand.length) out.push({ id: `mcp:${name}:command`, subject: element.id, page: "mcp", state: "broken", title: `${name}`, detail: `找不到要执行的命令：${names(noCommand, nameOf)}`, fix: open("去查看") });
    if (vars.length) out.push({ id: `mcp:${name}:vars`, subject: element.id, page: "mcp", state: "broken", title: `${name}`, detail: `变量没有取值：${vars.join("、")}`, fix: open("去填写") });
    const missing = fill.filter((row) => row.elementId === element.id);
    if (missing.length) {
      out.push({ id: `mcp:${name}:missing`, subject: element.id, page: "mcp", state: "missing", title: `${name}`, detail: `还没有：${names(missing.map((row) => row.to), nameOf)}`, fix: { kind: "mcp-fill", label: "补齐", rows: missing } });
    }
    return out;
  });
}

function ruleItems(elements: EngineElement[]): InboxItem[] {
  const fill = rulesFillRows(elements);
  return elements.flatMap((element): InboxItem[] => {
    const spec = element.spec as unknown as RulesSpec;
    const follow = (element.observed as unknown as RulesObserved).source_follow;
    const open = (label: string) => ({ kind: "open" as const, label, target: "rules" });
    const out: InboxItem[] = [];
    const row = fill.find((item) => item.elementId === element.id);
    if (row) out.push({ id: `rules:${element.id}:missing`, subject: element.id, page: "rules", state: "missing", title: `${spec.name}`, detail: "还没有规则文件，可从同类主账号复制", fix: { kind: "rules-fill", label: "补齐", rows: [row] } });
    if (follow?.update_available) {
      out.push({ id: `rules:${element.id}:update`, subject: element.id, page: "rules", state: "update", title: `${spec.name}`, detail: `跟随的「${follow.rule}」有新版本`, fix: open("看差异后更新") });
    } else if (follow && !follow.file_unchanged && follow.current_revision && follow.current_revision !== follow.applied_revision) {
      out.push({ id: `rules:${element.id}:drifted`, subject: element.id, page: "rules", state: "drifted", title: `${spec.name}`, detail: `改过，跟随的「${follow.rule}」也有新版本`, fix: open("看差异") });
    }
    return out;
  });
}

function accountItems(agents: EngineElement[], profiles: EngineElement[], aliases: EngineElement[]): InboxItem[] {
  const lines = aliases.flatMap((element) => (element.observed as unknown as AliasesObserved).entries ?? []);
  const managed = Object.fromEntries(lines.filter((line) => line.value).map((line) => [line.name, line.value as string]));
  return buildAccounts(agents, profiles, managed).flatMap((group) =>
    group.accounts.flatMap((account): InboxItem[] => {
      const title = account.isMain ? `${group.label} 主号` : account.name;
      const open = (label: string) => ({ kind: "open" as const, label, target: "accounts" });
      const base = { page: "accounts" as const, subject: account.elementId, title };
      if (account.blockedReason) return [{ ...base, id: `account:${account.elementId}:blocked`, state: "broken", detail: account.blockedReason, fix: open("去处理") }];
      const out: InboxItem[] = [];
      if (account.mainHasThirdParty) out.push({ ...base, id: `account:${account.elementId}:third`, state: "drifted", detail: "主账号里混进了第三方接入", fix: open("去处理") });
      if (account.drifted) out.push({ ...base, id: `account:${account.elementId}:drifted`, state: "drifted", detail: "目录内容被外部改过", fix: open("去查看") });
      if (!account.isMain && !account.command) out.push({ ...base, id: `account:${account.elementId}:command`, state: "missing", detail: "还没有启动命令", fix: open("去写入") });
      return out;
    }),
  );
}

/**
 * 启动命令：只比较已经有受管区块的文件。~/.zprofile 没有区块时本工具不新建它，只在 ~/.zshrc 里的
 * 命令不算「缺一处」。
 */
function commandItems(targets: EngineElement[]): InboxItem[] {
  const managed = targets
    .map((target) => target.observed as unknown as AliasesObserved)
    .filter((observed) => observed.base !== null && observed.base !== "missing");
  const lines = new Map<string, string[]>();
  for (const observed of managed) {
    for (const entry of observed.entries ?? []) lines.set(entry.name, [...(lines.get(entry.name) ?? []), entry.line]);
  }
  return [...lines.entries()]
    .filter(([, found]) => found.length < managed.length || new Set(found).size > 1)
    .map(([name, found]): InboxItem => ({
      id: `command:${name}`,
      subject: `command:${name}`,
      page: "commands",
      state: "drifted",
      title: `${name}`,
      detail: found.length < managed.length ? "只在其中一个终端配置文件里" : "两个终端配置文件里写法不同",
      fix: { kind: "open", label: "去查看", target: "commands" },
    }));
}

export function buildInbox(input: InboxInput): InboxItem[] {
  const nameOf = input.nameOf ?? ((agent: string) => agent);
  return [
    ...sourceItems(input.skills.filter(isCollection)),
    ...unmanagedItems(input.unmanaged),
    ...skillItems(input.skills, nameOf),
    ...mcpItems(input.mcp, input.mcpTargets, nameOf),
    ...ruleItems(input.rules),
    ...accountItems(input.agents, input.profiles, input.aliases),
    ...commandItems(input.aliases),
  ];
}

/**
 * 各导航项的待处理数：按负责的页面数有待处理的对象（同一个技能既损坏又偏离算一个），与各页
 * 「只看待处理」的数字一致；「待处理」本身数全部条目。
 */
export function countByPage(items: InboxItem[]): Record<string, number> {
  const subjects: Record<string, Set<string>> = {};
  for (const item of items) (subjects[item.page] ??= new Set()).add(item.subject);
  const counts: Record<string, number> = Object.fromEntries(Object.entries(subjects).map(([page, set]) => [page, set.size]));
  counts.inbox = items.length;
  return counts;
}

/** 全应用的待处理项（已去掉用户忽略的，另给忽略的）；查询还没回来的领域按没有。 */
export function useInbox(nameOf?: (agent: string) => string): { items: InboxItem[]; ignored: InboxItem[]; loading: boolean } {
  const ignoredKeys = useIgnored();
  const skills = useEngineQuery<StatusData>("skills:status");
  const mcp = useEngineQuery<StatusData>("mcp:status");
  const rules = useEngineQuery<StatusData>("rules:status");
  const agents = useEngineQuery<StatusData>("agents:status");
  const profiles = useEngineQuery<StatusData>("profiles:status");
  const aliases = useEngineQuery<StatusData>("aliases:status");
  const elementsOf = (query: { data?: { ok: boolean; data: StatusData } }) => (query.data?.ok ? query.data.data.elements ?? [] : []);
  const items = buildInbox({
    skills: elementsOf(skills),
    unmanaged: skills.data?.ok ? skills.data.data.unmanaged ?? null : null,
    mcp: elementsOf(mcp),
    mcpTargets: mcp.data?.ok ? mcp.data.data.targets ?? [] : [],
    rules: elementsOf(rules),
    agents: elementsOf(agents),
    profiles: elementsOf(profiles),
    aliases: elementsOf(aliases),
    nameOf,
  });
  const loading = [skills, mcp, rules, agents, profiles, aliases].some((query) => query.isPending);
  return {
    items: items.filter((item) => !ignoredKeys.has(ignoreKey(item))),
    ignored: items.filter((item) => ignoredKeys.has(ignoreKey(item))),
    loading,
  };
}
