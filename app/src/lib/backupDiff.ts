import { t } from "./i18n";
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

export const BACKUP_MODULES = ["skills", "mcp", "rules", "launch", "environment"] as const;
export type BackupModule = typeof BACKUP_MODULES[number];
export const BACKUP_MODULE_LABELS: Record<BackupModule, string> = { get skills() { return t("技能来源"); }, get mcp() { return t("MCP 已保存项"); }, get rules() { return t("规则模板"); }, get launch() { return t("启动配置"); }, get environment() { return t("环境集合"); } };

export function selectBackupModules(current: Json, incoming: Json, modules: BackupModule[]): Json {
  const policy = { ...asObject(current.policy) };
  for (const key of ["skills", "mcp"] as const) {
    if (modules.includes(key)) policy[key] = asObject(incoming.policy)[key];
  }
  return { ...incoming, policy,
    mcp_saved: modules.includes("mcp") ? incoming.mcp_saved : current.mcp_saved,
    rule_templates: modules.includes("rules") ? incoming.rule_templates ?? current.rule_templates : current.rule_templates,
    launch_recipes: modules.includes("launch") ? incoming.launch_recipes ?? current.launch_recipes : current.launch_recipes,
    environment_profiles: modules.includes("environment") ? incoming.environment_profiles ?? current.environment_profiles : current.environment_profiles,
    sources: modules.includes("rules") ? incoming.sources : {},
  };
}

function asObject(value: unknown): Json {
  return typeof value === "object" && value !== null ? (value as Json) : {};
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

function diffSaved(label: string, current: unknown, incoming: unknown, name: (row: Json) => string): DiffSection {
  const byId = (items: unknown): Record<string, Json> => Object.fromEntries((Array.isArray(items) ? items : []).map((item) => {
    const row = asObject(item); return [String(row.id), row];
  }));
  const before = byId(current), after = byId(incoming);
  const diff = diffMaps(label, before, after);
  const display = (id: string) => name(after[id] ?? before[id]) || id;
  return { label, added: diff.added.map(display), changed: diff.changed.map(display), removed: diff.removed.map(display) };
}

const SOURCE_LABELS: Record<string, string> = {
  // 旧备份里的规则来源：导入时转成「旧规则来源」模板，不再作为来源保存
  get "rules.shared"() { return t("旧规则来源（共享，导入后转为模板）"); },
  get "rules.claude"() { return t("旧规则来源（Claude 专用，导入后转为模板）"); },
  get "rules.codex"() { return t("旧规则来源（Codex 专用，导入后转为模板）"); },
  // Hooks 已不再管理：旧备份里的这两段导入时跳过
  get "hooks.claude"() { return t("旧版 Claude Hooks（已不再管理，导入时跳过）"); },
  get "hooks.codex"() { return t("旧版 Codex Hooks（已不再管理，导入时跳过）"); },
  // 旧版终端快捷命令不再导入或管理
  get "aliases.shell"() { return t("旧版终端快捷命令（已不再管理，导入时跳过）"); },
};

export function sourceLabel(key: string): string {
  return SOURCE_LABELS[key] ?? key;
}

export function diffBackup(current: Json, incoming: Json): BackupDiff {
  const currentPolicy = asObject(current.policy);
  const incomingPolicy = asObject(incoming.policy);

  const sections = [
    // 导入只新增或覆盖同名项，本机已有的 MCP 已保存项不会消失
    { ...diffMaps(t("MCP 已保存项"), mcpSavedItems(current), mcpSavedItems(incoming)), removed: [] },
    diffMaps(
      t("技能来源"),
      asObject(asObject(currentPolicy.skills).repositories),
      asObject(asObject(incomingPolicy.skills).repositories),
    ),
    // 备份里带了模板才会整体替换；没带就不动现有模板，不报消失
    ...("rule_templates" in incoming
      ? [diffMaps(t("规则模板"), templatesByName(current.rule_templates), templatesByName(incoming.rule_templates))]
      : []),
    ...(incoming.launch_recipes != null ? [diffSaved(t("启动配置"), current.launch_recipes, incoming.launch_recipes, (row) => {
      const recipe = asObject(row.recipe);
      return `${(Array.isArray(recipe.variants) ? recipe.variants : []).map((variant) => String(asObject(variant).name)).join(", ")} · ${String(recipe.target ?? "")}`;
    })] : []),
    ...(incoming.environment_profiles != null ? [diffSaved(t("环境集合"), current.environment_profiles, incoming.environment_profiles, (row) => String(asObject(row.profile).name ?? ""))] : []),
  ].filter((section) => section.added.length + section.changed.length + section.removed.length > 0);

  const currentSources = asObject(current.sources);
  const incomingSources = asObject(incoming.sources);
  const sourcesChanged = Object.keys(incomingSources)
    .filter((key) => !["aliases.shell", "hooks.claude", "hooks.codex"].includes(key))
    .filter((key) => JSON.stringify(currentSources[key] ?? "") !== JSON.stringify(incomingSources[key]))
    .sort();

  return {
    sections,
    sourcesChanged,
    identical: sections.length === 0 && sourcesChanged.length === 0,
  };
}
