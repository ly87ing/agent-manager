/**
 * 备份导入前的差异对比（纯函数）。
 *
 * 导入是整体替换且不可撤销，所以点确认之前要让人看清楚：哪些定义会新增、哪些会被改、
 * 哪些会消失。只比定义与来源本身，不比本机落地状态（那本来就不在备份里）。
 */

export interface DiffSection {
  label: string;
  added: string[];
  changed: string[];
  removed: string[];
}

export interface BackupDiff {
  sections: DiffSection[];
  /** 内容会变的来源键。 */
  sourcesChanged: string[];
  /** true = 两边完全一样，导入不会改变任何东西。 */
  identical: boolean;
}

type Json = Record<string, unknown>;

function asObject(value: unknown): Json {
  return typeof value === "object" && value !== null ? (value as Json) : {};
}

/** 取某个定义容器，并按是否带 profile 段拆成「助手」与「实例」两类。 */
function agentBuckets(policy: Json): { agents: Json; profiles: Json } {
  const defs = asObject(asObject(policy.agents).definitions);
  const agents: Json = {};
  const profiles: Json = {};
  for (const [name, value] of Object.entries(defs)) {
    if (asObject(value).profile) profiles[name] = value;
    else agents[name] = value;
  }
  return { agents, profiles };
}

function diffMaps(label: string, before: Json, after: Json): DiffSection {
  const added: string[] = [];
  const changed: string[] = [];
  const removed: string[] = [];
  for (const key of Object.keys(after)) {
    if (!(key in before)) added.push(key);
    else if (JSON.stringify(before[key]) !== JSON.stringify(after[key])) changed.push(key);
  }
  for (const key of Object.keys(before)) {
    if (!(key in after)) removed.push(key);
  }
  return { label, added: added.sort(), changed: changed.sort(), removed: removed.sort() };
}

/** 规则模板按名称比正文（名称在本机唯一；id 导入时重新生成，不参与比较）。 */
function templatesByName(value: unknown): Json {
  const out: Json = {};
  if (!Array.isArray(value)) return out;
  for (const item of value) {
    const template = asObject(item);
    if (typeof template.name === "string") out[template.name] = template.body ?? "";
  }
  return out;
}

const SOURCE_LABELS: Record<string, string> = {
  // 旧备份里的规则来源：导入时转成「旧规则来源」模板，不再作为来源保存
  "rules.shared": "旧规则来源（共享，导入后转为模板）",
  "rules.claude": "旧规则来源（Claude 专用，导入后转为模板）",
  "rules.codex": "旧规则来源（Codex 专用，导入后转为模板）",
  "hooks.claude": "Claude Hooks",
  "hooks.codex": "Codex Hooks",
  "aliases.shell": "终端快捷命令",
};

export function sourceLabel(key: string): string {
  return SOURCE_LABELS[key] ?? key;
}

export function diffBackup(current: Json, incoming: Json): BackupDiff {
  const currentPolicy = asObject(current.policy);
  const incomingPolicy = asObject(incoming.policy);
  const currentAgents = agentBuckets(currentPolicy);
  const incomingAgents = agentBuckets(incomingPolicy);

  const sections = [
    diffMaps("助手定义", currentAgents.agents, incomingAgents.agents),
    diffMaps("实例", currentAgents.profiles, incomingAgents.profiles),
    diffMaps("MCP 服务", asObject(asObject(currentPolicy.mcp).definitions), asObject(asObject(incomingPolicy.mcp).definitions)),
    diffMaps(
      "技能集合",
      asObject(asObject(currentPolicy.skills).repositories),
      asObject(asObject(incomingPolicy.skills).repositories),
    ),
    // 备份里带了模板才会整体替换；没带就不动现有模板，不报消失
    ...("rule_templates" in incoming
      ? [diffMaps("规则模板", templatesByName(current.rule_templates), templatesByName(incoming.rule_templates))]
      : []),
  ].filter((section) => section.added.length + section.changed.length + section.removed.length > 0);

  const currentSources = asObject(current.sources);
  const incomingSources = asObject(incoming.sources);
  const sourcesChanged = Object.keys(incomingSources)
    .filter((key) => JSON.stringify(currentSources[key] ?? "") !== JSON.stringify(incomingSources[key]))
    .sort();

  return {
    sections,
    sourcesChanged,
    identical: sections.length === 0 && sourcesChanged.length === 0,
  };
}
