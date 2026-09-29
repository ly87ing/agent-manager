/**
 * 备份导入前的差异对比（纯函数）。
 *
 * 导入是整体替换且不可撤销，所以点确认之前要让人看清楚：哪些定义会新增、哪些会被改、
 * 哪些会消失。只比定义与来源本身，不比本机落地状态（那本来就不在备份里）。
 * MCP 例外：生效条目在各助手配置文件里、不进备份；备份只带「已保存未生效」的 MCP 项
 * （mcp_saved，旧备份则是 policy.mcp.definitions），导入时逐项写入、本机已有的不删除。
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

/** 已保存的 MCP 项：键为「服务 @ 助手」（未分配的只写服务名）；旧备份的服务库定义按未分配处理。 */
function mcpSavedItems(backup: Json): Json {
  const out: Json = {};
  const rows = Array.isArray(backup.mcp_saved) ? backup.mcp_saved : [];
  for (const raw of rows) {
    const row = asObject(raw);
    const key = row.agent ? `${String(row.server)} @ ${String(row.agent)}` : String(row.server);
    out[key] = { entry: row.entry, resolve: row.resolve };
  }
  for (const [name, def] of Object.entries(asObject(asObject(asObject(backup.policy).mcp).definitions))) {
    if (!(name in out)) out[name] = { entry: def, resolve: { env: [], headers: [] } };
  }
  return out;
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
  // 旧版本保存、还没生效的 hooks：导入后在 Hooks 页作为未生效建议出现，不改实际配置
  "hooks.claude": "旧版已保存的 Claude Hooks（未生效）",
  "hooks.codex": "旧版已保存的 Codex Hooks（未生效）",
  // 旧版本保存、还没写进终端配置的快捷命令：导入后在快捷命令页作为已保存未生效建议出现
  "aliases.shell": "旧版已保存的终端快捷命令（未生效）",
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
    // 导入只新增或覆盖同名项，本机已有的 MCP 已保存项不会消失
    { ...diffMaps("MCP 已保存项", mcpSavedItems(current), mcpSavedItems(incoming)), removed: [] },
    diffMaps(
      "技能来源",
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
