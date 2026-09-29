/**
 * 集合「用哪些技能」与 skill_paths 之间的换算。
 *
 * skill_paths 的一项可以是一个技能（`alpha`、`skills/alpha`，单段名引擎会回退到 skills/<名>），
 * 也可以是装技能的目录（`skills`：里面每个子目录都是技能，来源以后新增的也自动算进来）；
 * null 表示按来源根目录发现。界面只让用户勾技能，这里负责两头换算，并尽量保留原写法。
 */

/** 来源里的一个技能（引擎 observed.available）：名字与相对来源根的路径。 */
export interface AvailableSkill {
  name: string;
  path: string;
}

export interface SkillScope {
  /** 按当前配置实际在用的技能名。 */
  selected: string[];
  /** 当前配置是否让来源以后新增的技能也自动算进来（null 或只有目录项）。 */
  auto: boolean;
  /** 配着、但来源里既不是技能也不是装技能的目录的项（改名或删了）。 */
  missing: string[];
  /** 技能名 → 原配置里指向它的写法，保存时沿用。 */
  entryOf: Record<string, string>;
}

const normalize = (entry: string) => entry.replace(/^\.\//, "").replace(/\/+$/, "");
const parentOf = (path: string) => (path.includes("/") ? path.slice(0, path.lastIndexOf("/")) : "");

export function readScope(skillPaths: string[] | null | undefined, available: AvailableSkill[]): SkillScope {
  if (!skillPaths || skillPaths.length === 0) {
    return { selected: available.map((skill) => skill.name), auto: true, missing: [], entryOf: {} };
  }
  const selected = new Set<string>();
  const entryOf: Record<string, string> = {};
  const missing: string[] = [];
  let skillEntries = 0;
  for (const raw of skillPaths) {
    const entry = normalize(raw);
    const skill = available.find((item) => item.path === entry || item.name === entry);
    if (skill) {
      selected.add(skill.name);
      entryOf[skill.name] = raw;
      skillEntries += 1;
      continue;
    }
    const inside = available.filter((item) => item.path.startsWith(`${entry}/`));
    if (inside.length > 0) inside.forEach((item) => selected.add(item.name));
    else missing.push(raw);
  }
  return {
    selected: available.map((skill) => skill.name).filter((name) => selected.has(name)),
    auto: skillEntries === 0 && missing.length === 0,
    missing,
    entryOf,
  };
}

/** 全选时能否表达成「以后新增的也自动算进来」：技能都在顶层，或都在子目录里（混着放的只能逐个列）。 */
export function canAutoInclude(available: AvailableSkill[]): boolean {
  if (available.length === 0) return false;
  const parents = available.map((skill) => parentOf(skill.path));
  return parents.every((parent) => parent === "") || parents.every((parent) => parent !== "");
}

/**
 * 勾选结果 → skill_paths。全选且要自动包含新技能：原本就是这样的配置原样保留，否则按技能所在目录写；
 * 其余逐个列出技能（沿用原写法），再加上仍保留的缺失项。返回 [] 表示一个都没选。
 */
export function toSkillPaths({
  available,
  selected,
  auto,
  keptMissing,
  initialPaths,
}: {
  available: AvailableSkill[];
  selected: string[];
  auto: boolean;
  keptMissing: string[];
  initialPaths: string[] | null;
}): string[] | null {
  const initial = readScope(initialPaths, available);
  const all = available.length > 0 && available.every((skill) => selected.includes(skill.name));
  if (auto && all && keptMissing.length === 0 && canAutoInclude(available)) {
    if (initial.auto && initial.selected.length === available.length) return initialPaths && initialPaths.length ? initialPaths : null;
    const parents = [...new Set(available.map((skill) => parentOf(skill.path)))];
    return parents.every((parent) => parent === "") ? null : parents;
  }
  const listed = available
    .filter((skill) => selected.includes(skill.name))
    .map((skill) => initial.entryOf[skill.name] ?? skill.path);
  return [...new Set([...listed, ...keptMissing])];
}
