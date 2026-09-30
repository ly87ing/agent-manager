//! 只读发现：扫描各 Agent 技能目录中「未纳入管理」的条目，按来源分类，并把可软链接
//! 接管的外部条目按源根目录聚合成建议集合。
//!
//! 分类顺序（先判定的优先，互斥）：
//! 1. 已有 assigned binding → 已受管，不出现在候选里；
//! 2. 真实目录/文件 → `native-dir`：位于助手官方技能目录内的原生安装（Claude `synced/` 等），
//!    只标识、保持其原生管理方式，不提供接管（软链接接管会与原生体系互相覆盖）；
//! 3. 断链 → `dead-link`：只建议清理；
//! 4. 名称命中某个已定义集合（按 `skill_paths` 发现的技能名）→ `collection-extra-link`：走既有集合接管通道；
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
use super::snapshot::discover_repo_skills;
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
/// skill_paths 可以是装技能的目录（如 `skills`），所以来源已在本地时按实际发现的技能名；
/// 还没取回或发现失败时按 skill_paths 末段。只读。
fn collection_of_skill_map(ctx: &Context, meta: &DomainMetadata) -> BTreeMap<String, String> {
    let mut collection_of_skill: BTreeMap<String, String> = BTreeMap::new();
    for (coll_name, coll) in &meta.collections {
        if coll.deleted {
            continue;
        }
        if let Some(paths) = &coll.skill_paths {
            let names: Vec<String> =
                match discover_repo_skills(&collection_source_root(ctx, coll), &coll.raw) {
                    Ok((found, _)) if !found.is_empty() => found.into_keys().collect(),
                    _ => paths
                        .iter()
                        .map(|rel| {
                            Path::new(rel)
                                .file_name()
                                .map(|n| n.to_string_lossy().to_string())
                                .unwrap_or_else(|| rel.clone())
                        })
                        .collect(),
                };
            for name in names {
                collection_of_skill
                    .entry(name)
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

/// 各账号实际在用的技能：各助手技能目录与 Codex 共用目录里、内容目录下有 SKILL.md 的条目，
/// 按内容目录（链接解析后的真实位置）去重，记下用到它的账号（共用目录记 `codex-shared`）。
/// 供程序域读取技能声明的依赖；只读零写。
pub(crate) struct SkillInUse {
    pub name: String,
    pub dir: PathBuf,
    pub agents: Vec<String>,
}

pub(crate) fn skills_in_use(ctx: &Context) -> Result<Vec<SkillInUse>> {
    let meta = load_metadata(ctx)?;
    let bindings = load_bindings_readonly(ctx)?;
    let mut entries: Vec<RawEntry> = scan_all(&meta, &bindings)
        .into_iter()
        .map(|(entry, _)| entry)
        .collect();
    entries.extend(scan_agent_dir(
        CODEX_SHARED_ID,
        &codex_shared_dir(&ctx.home_dir),
    ));
    let mut by_dir: BTreeMap<PathBuf, SkillInUse> = BTreeMap::new();
    for entry in entries {
        let dir = if entry.is_link {
            match entry.target.filter(|_| entry.target_exists) {
                Some(target) => target,
                None => continue,
            }
        } else {
            entry.link_path.clone()
        };
        if !dir.join("SKILL.md").is_file() {
            continue;
        }
        let dir = dir.canonicalize().unwrap_or(dir);
        let slot = by_dir.entry(dir.clone()).or_insert_with(|| SkillInUse {
            name: entry.name.clone(),
            dir,
            agents: Vec::new(),
        });
        if !slot.agents.contains(&entry.agent) {
            slot.agents.push(entry.agent);
        }
    }
    Ok(by_dir.into_values().collect())
}

/// skills:status 的 `installed`：各助手技能目录里的实际条目（「当前」视图），与 discover
/// 同一扫描与分类。本工具链接（origin=managed）= 有 assigned binding、链接目标存在且落在
/// 受管基准内（自管基目录或已登记的本地来源）；其余沿用 discover 的分类。只展示、不产生
/// 待处理项；只读零写。
pub(crate) fn installed(ctx: &Context, meta: &DomainMetadata, bindings: &Bindings) -> Value {
    let bases = manager_bases(ctx);
    let link_bases = managed_bases(ctx, &meta.collections);
    let collection_of_skill = collection_of_skill_map(ctx, meta);
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

/// Codex 共用技能目录：Codex 在各账号自己的 `$CODEX_HOME/skills` 之外还读 `~/.agents/skills`，
/// 对所有 Codex 账号生效（2026-09-29 codex-cli 0.159.0 实测）；Claude 不读它。
const CODEX_SHARED_ID: &str = "codex-shared";
/// `npx skills` 的锁文件；只读，从不改写。
const SKILL_LOCK_FILE: &str = ".agents/.skill-lock.json";
/// 读锁文件、发布信息与 git 配置的上限：这些都是小文件，超限按读不到处理。
const SMALL_FILE_LIMIT: u64 = 1024 * 1024;

fn codex_shared_dir(home_dir: &Path) -> PathBuf {
    home_dir.join(".agents/skills")
}

fn read_small_json(path: &Path) -> Option<Value> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > SMALL_FILE_LIMIT {
        return None;
    }
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

/// 锁文件里的技能记录：技能名 → 记录。读不了或格式不对时为空。
fn read_skill_lock(home_dir: &Path) -> Map<String, Value> {
    read_small_json(&home_dir.join(SKILL_LOCK_FILE))
        .and_then(|lock| lock.get("skills").and_then(Value::as_object).cloned())
        .unwrap_or_default()
}

/// 从 `.git/config` 里读 `[remote "origin"]` 的 url；只读文件，不起 git 进程。
fn origin_url(config: &Path) -> Option<String> {
    let meta = std::fs::metadata(config).ok()?;
    if meta.len() > SMALL_FILE_LIMIT {
        return None;
    }
    let text = std::fs::read_to_string(config).ok()?;
    let mut in_origin = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_origin = line.replace(' ', "") == "[remote\"origin\"]";
            continue;
        }
        if in_origin {
            if let Some((key, value)) = line.split_once('=') {
                if key.trim() == "url" {
                    let url = value.trim();
                    return (!url.is_empty()).then(|| url.to_string());
                }
            }
        }
    }
    None
}

/// 技能目录所在的 git 仓库：向上找 `.git` 目录（到主目录为止），返回 (origin url, 技能在仓库里的相对路径)。
fn git_source(dir: &Path, home_dir: &Path) -> Option<(String, String)> {
    let mut current = dir.to_path_buf();
    for _ in 0..16 {
        let git = current.join(".git");
        if git.is_dir() {
            let url = origin_url(&git.join("config"))?;
            let rel = dir
                .strip_prefix(&current)
                .map(|rel| rel.to_string_lossy().to_string())
                .unwrap_or_default();
            return Some((url, rel));
        }
        if current == home_dir || !current.pop() {
            return None;
        }
    }
    None
}

/// 按锁文件、技能自带的 `skill-release.json`、所在 git 仓库识别来源；都没有时为 null。
/// 只读：不访问网络，不起进程。
fn detect_source(
    name: &str,
    content_dir: Option<&Path>,
    lock: &Map<String, Value>,
    home_dir: &Path,
) -> Value {
    let mut evidence: Vec<&str> = Vec::new();
    let mut repository: Option<String> = None;
    let mut path: Option<String> = None;
    let mut version: Option<String> = None;
    let mut installer: Option<&str> = None;
    if let Some(entry) = lock.get(name) {
        let url = entry
            .get("sourceUrl")
            .and_then(Value::as_str)
            .filter(|url| !url.is_empty());
        if let Some(url) = url {
            evidence.push("skill-lock");
            installer = Some("npx-skills");
            repository = Some(url.to_string());
            path = entry.get("skillPath").and_then(Value::as_str).map(|raw| {
                raw.trim_end_matches("SKILL.md")
                    .trim_end_matches('/')
                    .to_string()
            });
        }
    }
    if let Some(dir) = content_dir {
        if let Some(release) = read_small_json(&dir.join("skill-release.json")) {
            let repo = release
                .pointer("/source/repository")
                .and_then(Value::as_str)
                .filter(|url| !url.is_empty());
            let ver = release
                .get("version")
                .and_then(Value::as_str)
                .filter(|v| !v.is_empty());
            if repo.is_some() || ver.is_some() {
                evidence.push("skill-release");
                if repository.is_none() {
                    repository = repo.map(str::to_string);
                }
                version = ver.map(str::to_string);
            }
        }
        if repository.is_none() {
            if let Some((url, rel)) = git_source(dir, home_dir) {
                evidence.push("git");
                repository = Some(url);
                path = Some(rel);
            }
        }
    }
    if evidence.is_empty() {
        return Value::Null;
    }
    json!({
        "repository": repository,
        "path": path.filter(|p| !p.is_empty()),
        "version": version,
        "installer": installer,
        "evidence": evidence,
    })
}

/// skills:status 的 `unmanaged`：本工具没有管理的技能（「其他来源」）、Codex 共用目录与重名。
///
/// - `skills`：按技能名聚合各账号目录里的真实目录与外部链接，以及 Codex 共用目录里的条目；
///   带识别出的来源，以及能不能纳入管理（Claude 自己同步的 `synced/`、没有 SKILL.md 的不能）。
///   共用目录里与本工具已在 Codex 账号上管理的技能同名的，不算「其他来源」，只记为重名。
/// - `duplicates`：同一个技能名既在某个 Codex 账号的技能目录里、又在共用目录里——Codex 会加载两份。
///
/// 只读零写：只 `read_dir`、读链接与小文件，不建数据目录，不访问网络。
pub(crate) fn unmanaged(ctx: &Context, meta: &DomainMetadata, bindings: &Bindings) -> Value {
    use crate::native_agent_adapters::AdapterKind;
    let bases = manager_bases(ctx);
    let link_bases = managed_bases(ctx, &meta.collections);
    let collection_of_skill = collection_of_skill_map(ctx, meta);
    let lock = read_skill_lock(&ctx.home_dir);
    let codex_agents: BTreeSet<String> = meta
        .agents
        .iter()
        .filter(|(_, info)| info.kind == Some(AdapterKind::Codex))
        .map(|(id, _)| id.clone())
        .collect();
    let claude_agents: BTreeSet<String> = meta
        .agents
        .iter()
        .filter(|(_, info)| info.kind == Some(AdapterKind::Claude))
        .map(|(id, _)| id.clone())
        .collect();

    // 技能名 → (位置列表, 内容目录, 不能纳入的原因)
    let mut groups: BTreeMap<String, (Vec<Value>, Option<PathBuf>, Option<&'static str>)> =
        BTreeMap::new();
    let mut add =
        |name: &str, location: Value, content: Option<PathBuf>, blocked: Option<&'static str>| {
            let slot = groups.entry(name.to_string()).or_default();
            slot.0.push(location);
            if slot.1.is_none() {
                slot.1 = content;
            }
            if slot.2.is_none() {
                slot.2 = blocked;
            }
        };
    let content_of = |entry: &RawEntry| -> Option<PathBuf> {
        if entry.is_link {
            entry.target.clone().filter(|_| entry.target_exists)
        } else {
            Some(entry.link_path.clone())
        }
    };
    let blocked_of =
        |entry: &RawEntry, content: Option<&Path>, claude: bool| -> Option<&'static str> {
            if claude && !entry.is_link && entry.name == "synced" {
                return Some("claude-synced");
            }
            if !content.is_some_and(|dir| dir.join("SKILL.md").is_file()) {
                return Some("no-skill-md");
            }
            None
        };

    // Codex 账号上本工具管理着的技能名：共用目录里的同名项只算重名
    let mut codex_managed: BTreeSet<String> = BTreeSet::new();
    let mut codex_names: BTreeMap<String, Vec<(String, PathBuf)>> = BTreeMap::new();
    for (entry, bound) in scan_all(meta, bindings) {
        let managed = bound.is_some()
            && entry.is_link
            && entry.target_exists
            && is_managed_link(&entry.link_path, &link_bases);
        if codex_agents.contains(&entry.agent) {
            codex_names
                .entry(entry.name.clone())
                .or_default()
                .push((entry.agent.clone(), entry.link_path.clone()));
            if managed {
                codex_managed.insert(entry.name.clone());
            }
        }
        if managed {
            continue;
        }
        let (origin, _, _) = classify(&entry, &bases, &collection_of_skill);
        if !matches!(origin, "native-dir" | "external-link") {
            continue;
        }
        let content = content_of(&entry);
        let blocked = blocked_of(
            &entry,
            content.as_deref(),
            claude_agents.contains(&entry.agent),
        );
        add(
            &entry.name,
            json!({
                "agent": entry.agent,
                "shared": Value::Null,
                "path": tildify(&entry.link_path, &ctx.home_dir),
                "target": entry.target.as_ref().map(|t| Value::String(tildify(t, &ctx.home_dir))).unwrap_or(Value::Null),
                "origin": origin,
            }),
            content,
            blocked,
        );
    }

    let mut shared_dirs: Vec<Value> = Vec::new();
    let mut duplicates: Vec<Value> = Vec::new();
    if !codex_agents.is_empty() {
        let shared_dir = codex_shared_dir(&ctx.home_dir);
        shared_dirs.push(json!({
            "id": CODEX_SHARED_ID,
            "kind": "codex",
            "path": tildify(&shared_dir, &ctx.home_dir),
            "exists": shared_dir.is_dir(),
        }));
        for entry in scan_agent_dir(CODEX_SHARED_ID, &shared_dir) {
            for (agent, path) in codex_names.get(&entry.name).into_iter().flatten() {
                duplicates.push(json!({
                    "name": entry.name,
                    "agent": agent,
                    "paths": [tildify(path, &ctx.home_dir), tildify(&entry.link_path, &ctx.home_dir)],
                }));
            }
            if codex_managed.contains(&entry.name) {
                continue;
            }
            let origin = if !entry.is_link {
                "native-dir"
            } else if entry.target_exists {
                "external-link"
            } else {
                continue;
            };
            let content = content_of(&entry);
            let blocked = blocked_of(&entry, content.as_deref(), false);
            add(
                &entry.name,
                json!({
                    "agent": Value::Null,
                    "shared": CODEX_SHARED_ID,
                    "path": tildify(&entry.link_path, &ctx.home_dir),
                    "target": entry.target.as_ref().map(|t| Value::String(tildify(t, &ctx.home_dir))).unwrap_or(Value::Null),
                    "origin": origin,
                }),
                content,
                blocked,
            );
        }
    }

    let skills: Vec<Value> = groups
        .into_iter()
        .map(|(name, (locations, content, blocked))| {
            let source = if blocked == Some("claude-synced") {
                Value::Null
            } else {
                detect_source(&name, content.as_deref(), &lock, &ctx.home_dir)
            };
            json!({
                "name": name,
                "locations": locations,
                "source": source,
                "adoptable": blocked.is_none(),
                "blocked_reason": blocked,
            })
        })
        .collect();
    json!({"skills": skills, "shared_dirs": shared_dirs, "duplicates": duplicates})
}

pub(crate) fn discover(ctx: &Context) -> Result<Value> {
    let meta = load_metadata(ctx)?;
    let bindings = load_bindings_readonly(ctx)?;
    let bases = manager_bases(ctx);
    let collection_of_skill = collection_of_skill_map(ctx, &meta);
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
