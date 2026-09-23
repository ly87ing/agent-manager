//! 只读发现：扫描各 Agent 技能目录中「未纳入管理」的条目，按来源分类，并把可软链接
//! 接管的外部条目按源根目录聚合成建议集合。
//!
//! 分类顺序（先判定的优先，互斥）：
//! 1. 已有 assigned binding → 已受管，不出现在候选里；
//! 2. 真实目录/文件 → `native-dir`：位于助手官方技能目录内的原生安装（Claude `synced/` 等），
//!    只标识、保持其原生管理方式，不提供接管（软链接接管会与原生体系互相覆盖）；
//! 3. 断链 → `dead-link`：只建议清理；
//! 4. 名称命中某个已定义集合的 `skill_paths` → `collection-extra-link`：走既有集合接管通道；
//! 5. 链接目标落在 manager 自管基目录内 → `legacy-link`：旧体系残留，只建议清理；
//! 6. 其余符号链接 → `external-link`：按源根目录聚合为建议的本地来源集合，软链接纳管。
//!
//! 同一扫描与分类也供 skills:status 的实际安装视图（`installed`）使用：那里列出全部条目，
//! 已有 assigned binding 且链接落在受管基准内的记为 `managed`（本工具链接），其余同上。
//!
//! 全程只读：仅 `read_dir`/`symlink_metadata`/`read_link` 与目标存在性判断，不创建
//! data_dir/DB/目录，不跟随链接递归扫描。

use anyhow::Result;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use super::paths::{
    collection_source_root, is_link_entry, is_local_source, is_managed_link, is_path_within,
    managed_bases, manager_bases, resolve_link_target, resolve_loose,
};
use super::persistence::load_bindings_readonly;
use super::status::{load_metadata, DomainMetadata};
use super::BindingEntry;
use crate::native::Context;

type Bindings = BTreeMap<(String, String, String), BindingEntry>;

/// 绝对路径转 `~/...` 展示/持久化形式；不在 home 下时原样返回。
fn tildify(path: &Path, home_dir: &Path) -> String {
    if let Ok(rel) = path.strip_prefix(home_dir) {
        return format!("~/{}", rel.to_string_lossy());
    }
    // 链接目标经 canonicalize 后可能落在 home 的真实路径上（如 macOS /var → /private/var），
    // 两种写法都归一到 ~，避免输出里混入解析后的绝对路径。
    if let Ok(canonical_home) = home_dir.canonicalize() {
        if let Ok(rel) = path.strip_prefix(&canonical_home) {
            return format!("~/{}", rel.to_string_lossy());
        }
    }
    path.to_string_lossy().to_string()
}

/// 建议集合名：取源根路径里最后一个非 skills/skill 的目录名做 slug（小写字母数字与连字符，
/// 与集合名校验一致）；为空或冲突时退化为 external-skills 并追加序号。
fn suggest_collection_name(source_root: &Path, taken: &BTreeSet<String>) -> String {
    let mut base = String::new();
    for component in source_root.components().rev() {
        let raw = component.as_os_str().to_string_lossy().to_lowercase();
        if raw == "skills" || raw == "skill" || raw.is_empty() {
            continue;
        }
        let slug: String = raw
            .trim_start_matches('.')
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        let slug = slug.trim_matches('-').to_string();
        if !slug.is_empty()
            && slug
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphanumeric())
        {
            base = slug;
            break;
        }
    }
    if base.is_empty() {
        base = "external-skills".to_string();
    }
    if !taken.contains(&base) {
        return base;
    }
    let mut n = 2;
    loop {
        let candidate = format!("{}-{}", base, n);
        if !taken.contains(&candidate) {
            return candidate;
        }
        n += 1;
    }
}

struct RawEntry {
    agent: String,
    name: String,
    link_path: PathBuf,
    target: Option<PathBuf>,
    is_link: bool,
    target_exists: bool,
}

/// 扫描单个 Agent 技能目录的直接子项；隐藏项（`.` 开头，如 Codex 内置 `.system/`）跳过。
fn scan_agent_dir(agent: &str, skills_dir: &Path) -> Vec<RawEntry> {
    let mut out = Vec::new();
    let Ok(read) = std::fs::read_dir(skills_dir) else {
        return out;
    };
    let mut paths: Vec<PathBuf> = read.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    paths.sort();
    for path in paths {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        if name.is_empty() || name.starts_with('.') {
            continue;
        }
        let is_link = is_link_entry(&path);
        let target = if is_link {
            resolve_link_target(&path).ok()
        } else {
            None
        };
        let target_exists = match &target {
            Some(t) => t.exists(),
            None => path.exists(),
        };
        out.push(RawEntry {
            agent: agent.to_string(),
            name,
            link_path: path,
            target,
            is_link,
            target_exists,
        });
    }
    out
}

/// 未纳入管理的技能条目发现结果（items 逐条 + groups 按源根聚合）。
/// 源目录下含 SKILL.md、但还没被任何助手链接的技能名（按名排序）。
/// 读不了目录时返回空——发现是只读路径，不因为一个目录读不了就整体失败。
fn unlinked_skills_in(
    root: &std::path::Path,
    linked: &BTreeMap<String, BTreeSet<String>>,
) -> Vec<String> {
    let Ok(read_dir) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut out: Vec<String> = Vec::new();
    for entry in read_dir.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') || linked.contains_key(&name) {
            continue;
        }
        if entry.path().join("SKILL.md").is_file() {
            out.push(name);
        }
    }
    out.sort();
    out
}

/// skill 名 → 已定义集合名（来自集合 skill_paths；整仓发现的集合无法按名判定，留给 external）。
fn collection_of_skill_map(meta: &DomainMetadata) -> BTreeMap<String, String> {
    let mut collection_of_skill: BTreeMap<String, String> = BTreeMap::new();
    for (coll_name, coll) in &meta.collections {
        if coll.deleted {
            continue;
        }
        if let Some(paths) = &coll.skill_paths {
            for rel in paths {
                let leaf = Path::new(rel)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| rel.clone());
                collection_of_skill
                    .entry(leaf)
                    .or_insert_with(|| coll_name.clone());
            }
        }
    }
    collection_of_skill
}

/// 逐助手扫描技能目录（助手按 id、条目按路径排序），每条带上「已有 assigned binding
/// 时所属集合」。discover 与实际安装视图共用这一次扫描。
fn scan_all(meta: &DomainMetadata, bindings: &Bindings) -> Vec<(RawEntry, Option<String>)> {
    // (skill, agent) → 已有 assigned binding 的集合
    let mut bound: BTreeMap<(String, String), String> = BTreeMap::new();
    for ((coll, skill, agent), entry) in bindings {
        if entry.assigned {
            bound
                .entry((skill.clone(), agent.clone()))
                .or_insert_with(|| coll.clone());
        }
    }
    let mut out = Vec::new();
    for (agent_id, agent_info) in &meta.agents {
        for entry in scan_agent_dir(agent_id, &agent_info.skills_dir) {
            let collection = bound
                .get(&(entry.name.clone(), entry.agent.clone()))
                .cloned();
            out.push((entry, collection));
        }
    }
    out
}

/// skills:status 的 `installed`：各助手技能目录里的实际条目（「当前」视图），与 discover
/// 同一扫描与分类。本工具链接（origin=managed）= 有 assigned binding、链接目标存在且落在
/// 受管基准内（自管基目录或已登记的本地来源）；其余沿用 discover 的分类。只展示、不产生
/// 待处理项；只读零写。
pub(crate) fn installed(ctx: &Context, meta: &DomainMetadata, bindings: &Bindings) -> Value {
    let bases = manager_bases(ctx);
    let link_bases = managed_bases(ctx, &meta.collections);
    let collection_of_skill = collection_of_skill_map(meta);
    let items: Vec<Value> = scan_all(meta, bindings)
        .into_iter()
        .map(|(entry, bound)| {
            let managed = bound.is_some()
                && entry.is_link
                && entry.target_exists
                && is_managed_link(&entry.link_path, &link_bases);
            let (origin, collection) = if managed {
                ("managed", bound)
            } else {
                let (origin, _, _) = classify(&entry, &bases, &collection_of_skill);
                let collection = if origin == "collection-extra-link" {
                    collection_of_skill.get(&entry.name).cloned()
                } else {
                    None
                };
                (origin, collection)
            };
            json!({
                "agent": entry.agent,
                "name": entry.name,
                "link_path": tildify(&entry.link_path, &ctx.home_dir),
                "target": entry.target.as_ref().map(|t| Value::String(tildify(t, &ctx.home_dir))).unwrap_or(Value::Null),
                "origin": origin,
                "collection": collection.map(Value::String).unwrap_or(Value::Null),
            })
        })
        .collect();
    Value::Array(items)
}

pub(crate) fn discover(ctx: &Context) -> Result<Value> {
    let meta = load_metadata(ctx)?;
    let bindings = load_bindings_readonly(ctx)?;
    let bases = manager_bases(ctx);
    let collection_of_skill = collection_of_skill_map(&meta);
    let mut taken_names: BTreeSet<String> = meta.collections.keys().cloned().collect();

    // 已登记的本地来源集合：源根 → 集合名。指向这些源根的未绑定链接直接接管进该集合，
    // 不另建同源集合。
    let mut local_roots: BTreeMap<PathBuf, String> = BTreeMap::new();
    for (coll_name, coll) in &meta.collections {
        if coll.deleted || !is_local_source(coll) {
            continue;
        }
        local_roots.insert(collection_source_root(ctx, coll), coll_name.clone());
    }

    let mut items: Vec<Value> = Vec::new();
    // 源根目录 → (建议集合名, skill 名 → 出现的 agent 列表)
    let mut group_roots: BTreeMap<PathBuf, BTreeMap<String, BTreeSet<String>>> = BTreeMap::new();

    for (entry, bound) in scan_all(&meta, &bindings) {
        // 已有 assigned binding 的条目已受管，不进候选
        if bound.is_some() {
            continue;
        }
        let (origin, manage, group_root) = classify(&entry, &bases, &collection_of_skill);
        let collection = collection_of_skill.get(&entry.name).cloned();
        if let Some(root) = group_root.clone() {
            group_roots
                .entry(root)
                .or_default()
                .entry(entry.name.clone())
                .or_default()
                .insert(entry.agent.clone());
        }
        let source = entry
            .target
            .clone()
            .unwrap_or_else(|| entry.link_path.clone());
        items.push(json!({
            "agent": entry.agent,
            "name": entry.name,
            "link_path": tildify(&entry.link_path, &ctx.home_dir),
            "target": entry.target.as_ref().map(|t| Value::String(tildify(t, &ctx.home_dir))).unwrap_or(Value::Null),
            "origin": origin,
            "manage": manage,
            "collection": collection.map(Value::String).unwrap_or(Value::Null),
            "group": group_root
                .map(|root| Value::String(tildify(&root, &ctx.home_dir)))
                .unwrap_or(Value::Null),
            "has_skill_md": entry.target_exists && source.join("SKILL.md").is_file(),
        }));
    }

    let mut groups: Vec<Value> = Vec::new();
    for (root, skills) in group_roots {
        let existing = local_roots.get(&root).cloned();
        let name = match &existing {
            Some(name) => name.clone(),
            None => {
                let name = suggest_collection_name(&root, &taken_names);
                taken_names.insert(name.clone());
                name
            }
        };
        let mut agents: BTreeSet<String> = BTreeSet::new();
        let mut skill_values: Vec<Value> = Vec::new();
        let mut skill_paths: Vec<Value> = Vec::new();
        for (skill, skill_agents) in &skills {
            agents.extend(skill_agents.iter().cloned());
            skill_paths.push(json!(skill));
            skill_values.push(json!({
                "name": skill,
                "agents": skill_agents.iter().cloned().collect::<Vec<_>>(),
                "has_skill_md": root.join(skill).join("SKILL.md").is_file(),
                "linked": true,
            }));
        }
        // 源目录里还有、但还没被任何助手链接过的技能也一并收录：用户的心智是
        // 「把这个目录接管进来」，只拿已链接的那几个会让人以为其余的丢了。
        for extra in unlinked_skills_in(&root, &skills) {
            skill_paths.push(json!(extra));
            skill_values.push(json!({
                "name": extra,
                "agents": Vec::<String>::new(),
                "has_skill_md": true,
                "linked": false,
            }));
        }
        let agents_list: Vec<String> = agents.into_iter().collect();
        let source_path = tildify(&root, &ctx.home_dir);
        let mut definition = Map::new();
        definition.insert("source_path".into(), json!(source_path));
        definition.insert("skill_paths".into(), Value::Array(skill_paths));
        definition.insert("agents".into(), json!(agents_list.clone()));
        groups.push(json!({
            "name": name,
            "source_path": source_path,
            "manage": "symlink",
            "existing_collection": existing.is_some(),
            "agents": agents_list,
            "skills": skill_values,
            "suggested_definition": Value::Object(definition),
        }));
    }

    // group 中的条目回填建议集合名（前端按 group 名分组执行接管）
    let name_by_root: BTreeMap<String, String> = groups
        .iter()
        .filter_map(|g| {
            Some((
                g["source_path"].as_str()?.to_string(),
                g["name"].as_str()?.to_string(),
            ))
        })
        .collect();
    for item in items.iter_mut() {
        if let Some(root) = item["group"].as_str().map(|s| s.to_string()) {
            if let Some(name) = name_by_root.get(&root) {
                item["suggested_collection"] = json!(name);
            }
        }
    }

    Ok(json!({"items": items, "groups": groups}))
}

/// 返回 (origin, manage, 参与聚合的源根目录)。
fn classify(
    entry: &RawEntry,
    bases: &[PathBuf],
    collection_of_skill: &BTreeMap<String, String>,
) -> (&'static str, &'static str, Option<PathBuf>) {
    if !entry.is_link {
        return ("native-dir", "native", None);
    }
    if !entry.target_exists {
        return ("dead-link", "cleanup", None);
    }
    if collection_of_skill.contains_key(&entry.name) {
        return ("collection-extra-link", "collection", None);
    }
    let target = entry
        .target
        .as_ref()
        .map(|t| resolve_loose(t))
        .unwrap_or_default();
    if bases.iter().any(|base| is_path_within(&target, base)) {
        return ("legacy-link", "cleanup", None);
    }
    let root = target.parent().map(|p| p.to_path_buf());
    ("external-link", "symlink", root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggested_name_skips_skills_segment_and_slugifies() {
        let taken = BTreeSet::new();
        assert_eq!(
            suggest_collection_name(Path::new("/Users/x/.xiaoyu-dev/skills"), &taken),
            "xiaoyu-dev"
        );
        assert_eq!(
            suggest_collection_name(Path::new("/Users/x/.tool/plugins/rd_plugin/skills"), &taken),
            "rd-plugin"
        );
    }

    #[test]
    fn suggested_name_avoids_collisions() {
        let mut taken = BTreeSet::new();
        taken.insert("xiaoyu-dev".to_string());
        assert_eq!(
            suggest_collection_name(Path::new("/Users/x/.xiaoyu-dev/skills"), &taken),
            "xiaoyu-dev-2"
        );
    }
}
