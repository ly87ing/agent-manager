//! skills domain kernels — port of build_skills_status / build_skills_plan from
//! src/engine_v1.py, with the doctor/runtime-probe primitives they build on
//! (discovery, v2 enablement matrix, link topology) ported here byte-faithfully:
//! manage_doctor.py: read_skill_name / resolve_skill_search_paths /
//! _discover_repo_skill_targets / expected_managed_skill_targets /
//! actual_skill_targets / is_managed_skill_target_path / _paths_match /
//! collect_skill_discovery_inbox; manage_runtime_probe.py: the six matrix
//! symbols; link_skills.py: is_manager_owned_repo.

use crate::envelope::{diagnostic, Diagnostic};
use crate::ids::{canonicalize_agent_id, canonicalize_agent_list, canonicalize_agent_mapping};
use crate::mcp::ConfigInvalid;
use crate::upstream;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

const SKILL_REPO_PULL_ACTION: &str =
    "{\"id\": \"pull\", \"risk\": \"network\", \"needs_lock\": true, \"offline_ok\": false}";
const SKILL_APPLY_ACTION: &str =
    "{\"id\": \"apply\", \"risk\": \"exec-local\", \"needs_lock\": true, \"offline_ok\": true}";

const DISCOVERY_EXCLUDED_DIRS: &[&str] = &[".git", "node_modules", "__pycache__", "venv", ".venv"];

fn expanduser(path: &str) -> String {
    crate::providers::expanduser(path)
}

/// Python Path.resolve(strict=False) 语义：存在的最长前缀 canonicalize，
/// 剩余部分原样拼接，最后词法消化 "." / ".."（macOS 的 /var→/private/var 也会被解析）。
pub fn resolve_loose(path: &Path) -> String {
    let mut missing: Vec<String> = Vec::new();
    let mut cursor = path.to_path_buf();
    let resolved = loop {
        match std::fs::canonicalize(&cursor) {
            Ok(resolved) => {
                let mut out = resolved;
                for part in missing.iter().rev() {
                    out = out.join(part);
                }
                break out.to_string_lossy().to_string();
            }
            Err(_) => {
                match cursor.file_name() {
                    Some(name) => {
                        missing.push(name.to_string_lossy().to_string());
                        cursor = cursor.parent().map(|p| p.to_path_buf()).unwrap_or(cursor);
                    }
                    None => break path.to_string_lossy().to_string(),
                }
            }
        }
    };
    normpath_posix(&resolved)
}

fn normpath_posix(path: &str) -> String {
    let absolute = path.starts_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." if !parts.is_empty() && parts.last() != Some(&"..") => {
                parts.pop();
            }
            ".." if absolute => {}
            _ => parts.push(part),
        }
    }
    let joined = parts.join("/");
    if absolute {
        format!("/{}", joined)
    } else if joined.is_empty() {
        ".".to_string()
    } else {
        joined
    }
}

fn normpath_nt(path: &str) -> String {
    // ntpath.normpath 的 posix 输入最小语义：折叠重复分隔符与 "." 段，解析 ".."。
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            _ => parts.push(part),
        }
    }
    let joined = parts.join("/");
    if path.starts_with('/') {
        format!("/{}", joined)
    } else {
        joined
    }
}

fn is_abs_nt(path: &str) -> bool {
    // ntpath.isabs：/ 开头、X: 盘符、或 \\ 开头。
    path.starts_with('/') || path.starts_with("\\\\") || {
        let bytes = path.as_bytes();
        bytes.len() > 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() && (bytes[2] == b'/' || bytes[2] == b'\\')
    }
}

/// read_skill_name：SKILL.md frontmatter 的 name（首行 ---、逐行、首个 name:、去引号）。
pub fn read_skill_name(skill_md: &Path) -> Option<String> {
    let content = std::fs::read_to_string(skill_md).ok()?;
    let mut lines = content.lines();
    if lines.next().map(str::trim) != Some("---") {
        return None;
    }
    for line in lines {
        let stripped = line.trim();
        if stripped == "---" {
            break;
        }
        if !line.contains(':') {
            continue;
        }
        let (key, value) = line.split_once(':')?;
        if key.trim() == "name" {
            let name = value.trim().trim_matches(|c| c == '\'' || c == '"');
            if !name.is_empty() {
                return Some(name.to_string());
            }
            return None;
        }
    }
    None
}

/// resolve_skill_search_paths port（fail-close：越界/缺失/非目录均报错）。
pub fn resolve_skill_search_paths(repo_path: &Path, config: &Map<String, Value>) -> Result<Vec<PathBuf>, String> {
    let raw_paths = config.get("skill_paths");
    if raw_paths.is_none() {
        return Ok(vec![repo_path.to_path_buf()]);
    }
    let raw_paths = raw_paths.unwrap();
    let items = match raw_paths.as_array() {
        Some(items) if !items.is_empty() => items,
        _ => return Err("skill_paths 必须是非空数组".to_string()),
    };
    let mut search_paths = Vec::new();
    for raw in items {
        let raw = match raw.as_str() {
            Some(value) if !value.trim().is_empty() => value,
            _ => return Err("skill_paths 中的每一项都必须是非空字符串".to_string()),
        };
        let search_path = PathBuf::from(resolve_loose(&repo_path.join(raw)));
        let within = search_path == repo_path || search_path.starts_with(repo_path);
        if !within {
            return Err(format!("skill_paths 包含越界路径: {}", raw));
        }
        if !search_path.exists() {
            return Err(format!("skill_paths 指向的目录不存在: {}", raw));
        }
        if !search_path.is_dir() {
            return Err(format!("skill_paths 指向的路径不是目录: {}", raw));
        }
        search_paths.push(search_path);
    }
    Ok(search_paths)
}

/// _discover_repo_skill_targets port：walk（排序、排除目录），含 SKILL.md 的目录
/// 读 frontmatter name → (name → 目录绝对路径)，重名进 duplicates。
pub fn discover_repo_skill_targets(
    search_paths: &[PathBuf],
) -> (BTreeMap<String, String>, BTreeSet<String>) {
    let mut repo_targets = BTreeMap::new();
    let mut duplicates = BTreeSet::new();
    for search_path in search_paths {
        let mut stack = vec![search_path.clone()];
        while let Some(dir) = stack.pop() {
            let mut dirs: Vec<PathBuf> = Vec::new();
            let mut has_skill_md = false;
            if let Ok(entries) = std::fs::read_dir(&dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() && !path.is_symlink() {
                        let name = entry.file_name().to_string_lossy().to_string();
                        if !DISCOVERY_EXCLUDED_DIRS.contains(&name.as_str()) {
                            dirs.push(path);
                        }
                    } else if entry.file_name() == "SKILL.md" {
                        has_skill_md = true;
                    }
                }
            }
            if has_skill_md {
                let skill_dir = PathBuf::from(resolve_loose(&dir));
                if let Some(skill_name) = read_skill_name(&dir.join("SKILL.md")) {
                    let rendered = skill_dir.to_string_lossy().to_string();
                    match repo_targets.get(&skill_name) {
                        Some(existing) if *existing != rendered => {
                            duplicates.insert(skill_name);
                        }
                        Some(_) => {}
                        None => {
                            repo_targets.insert(skill_name, rendered);
                        }
                    }
                }
            }
            dirs.sort();
            for child in dirs.into_iter().rev() {
                stack.push(child);
            }
        }
    }
    (repo_targets, duplicates)
}

// ---------------------------------------------------------------------------
// v2 启用矩阵（manage_runtime_probe.py 六符号移植；错误文案逐字一致）
// ---------------------------------------------------------------------------

pub fn repo_skill_matrix_mode(repo_config: &Map<String, Value>) -> bool {
    repo_config.contains_key("skills") || repo_config.contains_key("new_skill_default")
}

pub fn validate_repo_new_skill_default(repo_name: &str, repo_config: &Map<String, Value>) -> Result<(), String> {
    match repo_config.get("new_skill_default") {
        None | Some(Value::Null) => Ok(()),
        Some(Value::String(raw)) if raw == "disabled" || raw == "enabled" => Ok(()),
        _ => Err(format!(
            "{}: skills.{}.new_skill_default 必须是 \"disabled\" 或 \"enabled\"",
            repo_name, repo_name
        )),
    }
}

pub fn validate_repo_skill_matrix(
    repo_name: &str,
    repo_config: &Map<String, Value>,
    collection_agents: &Option<Vec<String>>,
    declared_target_ids: &BTreeSet<String>,
) -> Result<(), String> {
    let raw = match repo_config.get("skills") {
        None => return Ok(()),
        Some(raw) => raw,
    };
    let matrix = raw
        .as_object()
        .ok_or_else(|| format!("{}: skills.{}.skills 必须是对象", repo_name, repo_name))?;
    let collection_scope: Option<BTreeSet<&str>> = collection_agents
        .as_ref()
        .map(|agents| agents.iter().map(String::as_str).collect());
    for (skill_name, entry) in matrix {
        if skill_name.trim().is_empty() {
            return Err(format!("{}: skills.{}.skills 的键必须是非空字符串", repo_name, repo_name));
        }
        let entry = entry
            .as_object()
            .ok_or_else(|| format!("{}: skills.{}.skills.{} 必须是对象", repo_name, repo_name, skill_name))?;
        let unknown: Vec<&String> = entry.keys().filter(|key| key.as_str() != "agents").collect();
        if !unknown.is_empty() {
            return Err(format!(
                "{}: skills.{}.skills.{} 含未知键: {}",
                repo_name,
                repo_name,
                skill_name,
                unknown.iter().map(|k| k.as_str()).collect::<Vec<_>>().join(", ")
            ));
        }
        let agents_spec = match entry.get("agents") {
            None => continue,
            Some(Value::Null) => continue,
            Some(spec) => spec,
        };
        let agents_map = agents_spec.as_object().ok_or_else(|| {
            format!(
                "{}: skills.{}.skills.{}.agents 必须是 null 或对象",
                repo_name, repo_name, skill_name
            )
        })?;
        for (agent_key, switch) in agents_map {
            if !switch.is_boolean() {
                return Err(format!(
                    "{}: skills.{}.skills.{}.agents.{} 必须是布尔值",
                    repo_name, repo_name, skill_name, agent_key
                ));
            }
            let agent_id = canonicalize_agent_id(agent_key);
            if !declared_target_ids.contains(&agent_id) {
                return Err(format!(
                    "{}: skills.{}.skills.{}.agents 包含未声明的 target: {}",
                    repo_name, repo_name, skill_name, agent_key
                ));
            }
            if let Some(ref scope) = collection_scope {
                if !scope.contains(agent_id.as_str()) {
                    return Err(format!(
                        "{}: skills.{}.skills.{}.agents 越出 collection agents 范围: {}",
                        repo_name, repo_name, skill_name, agent_key
                    ));
                }
            }
        }
    }
    Ok(())
}

/// resolve_skill_agent_switch：None=跟随 collection；集合=显式收敛（空集=收件箱）。
pub fn resolve_skill_agent_switch(repo_config: &Map<String, Value>, skill_name: &str) -> Option<BTreeSet<String>> {
    let matrix = repo_config.get("skills").and_then(Value::as_object);
    let entry = matrix
        .and_then(|m| m.get(skill_name))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if !entry.contains_key("agents") {
        let default = repo_config
            .get("new_skill_default")
            .and_then(Value::as_str)
            .unwrap_or("disabled");
        if default == "enabled" {
            return None;
        }
        return Some(BTreeSet::new());
    }
    let agents_spec = entry.get("agents").unwrap();
    if agents_spec.is_null() {
        return None;
    }
    let mut out = BTreeSet::new();
    if let Some(map) = agents_spec.as_object() {
        for (agent_key, on) in map {
            if on.as_bool().unwrap_or(false) {
                out.insert(canonicalize_agent_id(agent_key));
            }
        }
    }
    Some(out)
}

pub fn resolve_repo_skill_agent_overrides(
    repo_name: &str,
    repo_config: &Map<String, Value>,
    discovered: &BTreeSet<String>,
) -> Result<BTreeMap<String, Option<BTreeSet<String>>>, String> {
    let empty = Map::new();
    let matrix = repo_config.get("skills").and_then(Value::as_object).unwrap_or(&empty);
    let unknown: Vec<&String> = matrix
        .keys()
        .filter(|name| !discovered.contains((*name).as_str()))
        .collect();
    if !unknown.is_empty() {
        let mut names: Vec<&str> = unknown.iter().map(|s| s.as_str()).collect();
        names.sort();
        return Err(format!(
            "skills.{}.skills 含未发现的 skill: {}",
            repo_name,
            names.join(", ")
        ));
    }
    Ok(discovered
        .iter()
        .map(|name| (name.clone(), resolve_skill_agent_switch(repo_config, name)))
        .collect())
}

// ---------------------------------------------------------------------------
// 链接拓扑（actual/expected/managed 判定）
// ---------------------------------------------------------------------------

fn normalize_comparable_path(path: &str) -> (String, String) {
    if is_abs_nt(path) && (path.contains('\\') || path.as_bytes().get(1) == Some(&b':')) {
        // windows 分支：normcase（小写）+ 反斜杠归一。
        let mut normalized = path.replace('/', "\\");
        normalized = normpath_nt(&normalized.replace('\\', "/")).replace('/', "\\");
        return ("windows".to_string(), normalized.to_lowercase());
    }
    let expanded = expanduser(path);
    ("posix".to_string(), resolve_loose(Path::new(&expanded)))
}

pub fn paths_match(left: Option<&str>, right: Option<&str>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => normalize_comparable_path(left) == normalize_comparable_path(right),
        _ => false,
    }
}

pub fn is_managed_skill_target_path(target_path: &str, managed_repo_base: &Path) -> bool {
    if target_path == "<broken>" {
        return false;
    }
    let (_, path) = normalize_comparable_path(target_path);
    let (_, base) = normalize_comparable_path(&managed_repo_base.to_string_lossy());
    path == base || path.starts_with(&format!("{}/", base))
}

/// actual_skill_targets port：目录条目中的软链 → (name → 解析目标) + 死链集合。
pub fn actual_skill_targets(target_path: &Path) -> (BTreeMap<String, String>, BTreeSet<String>) {
    let mut actual = BTreeMap::new();
    let mut broken = BTreeSet::new();
    let entries = match std::fs::read_dir(target_path) {
        Ok(entries) => entries,
        Err(_) => return (actual, broken),
    };
    for entry in entries.flatten() {
        let link_path = entry.path();
        if !link_path.is_symlink() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        match std::fs::read_link(&link_path) {
            Ok(raw) => {
                let raw = raw.to_string_lossy().to_string();
                let resolved = if is_abs_nt(&raw) {
                    normpath_nt(&raw)
                } else {
                    let expanded = expanduser(&raw);
                    let candidate = if Path::new(&expanded).is_absolute() {
                        PathBuf::from(expanded)
                    } else {
                        link_path
                            .parent()
                            .map(|p| p.join(&expanded))
                            .unwrap_or_else(|| PathBuf::from(&expanded))
                    };
                    resolve_loose(&candidate)
                };
                if !link_path.exists() {
                    broken.insert(name.clone());
                }
                actual.insert(name, resolved);
            }
            Err(_) => {
                actual.insert(name.clone(), "<broken>".to_string());
                broken.insert(name);
            }
        }
    }
    (actual, broken)
}

/// expected_managed_skill_targets port（problems 通道保留但引擎不使用）。
#[allow(clippy::too_many_arguments)]
pub fn expected_managed_skill_targets(
    root_dir: &Path,
    config: &Map<String, Value>,
    repositories: &Map<String, Value>,
    agent_targets: &Map<String, Value>,
    agent_id: &str,
) -> (BTreeMap<String, String>, Vec<String>) {
    let mut expected = BTreeMap::new();
    let mut problems = Vec::new();
    let repo_base = root_dir.join("agent-skill").join("repositories");
    let repo_base = PathBuf::from(resolve_loose(&repo_base));
    let declared_target_ids: BTreeSet<String> = agent_targets.keys().cloned().collect();
    let skills_policy = config.get("skills").and_then(Value::as_object).cloned().unwrap_or_default();

    for (repo_name, policy) in &skills_policy {
        let policy_map = match policy.as_object() {
            Some(map) => map,
            None => continue,
        };
        if !policy_map.get("enabled").and_then(Value::as_bool).unwrap_or(true) {
            continue;
        }
        let definition = match repositories.get(repo_name).and_then(Value::as_object) {
            Some(def) => def.clone(),
            None => continue,
        };
        let mut merged = definition;
        for (key, value) in policy_map {
            merged.insert(key.clone(), value.clone());
        }
        let targeted_agents = canonicalize_agent_list(merged.get("agents").unwrap_or(&Value::Null));
        if let Some(ref targeted) = targeted_agents {
            if !targeted.iter().any(|item| item == agent_id) {
                continue;
            }
        }
        let repo_path = repo_base.join(repo_name);
        let repo_path = PathBuf::from(resolve_loose(&repo_path));
        if !repo_path.exists() {
            problems.push(format!("{}: 仓库缓存缺失", repo_name));
            continue;
        }
        let search_paths = match resolve_skill_search_paths(&repo_path, &merged) {
            Ok(paths) => paths,
            Err(exc) => {
                problems.push(format!("{}: {}", repo_name, exc));
                continue;
            }
        };
        let (mut repo_targets, duplicate_names) = discover_repo_skill_targets(&search_paths);
        if !duplicate_names.is_empty() {
            problems.push(format!(
                "{}: 重复 skill name: {}",
                repo_name,
                duplicate_names.iter().cloned().collect::<Vec<_>>().join(", ")
            ));
            continue;
        }
        if repo_skill_matrix_mode(&merged) {
            let overrides = (|| {
                validate_repo_new_skill_default(repo_name, &merged)?;
                validate_repo_skill_matrix(repo_name, &merged, &targeted_agents, &declared_target_ids)?;
                resolve_repo_skill_agent_overrides(
                    repo_name,
                    &merged,
                    &repo_targets.keys().cloned().collect(),
                )
            })();
            let overrides = match overrides {
                Ok(overrides) => overrides,
                Err(exc) => {
                    problems.push(format!("{}: {}", repo_name, exc));
                    continue;
                }
            };
            let disabled_names: BTreeSet<String> = merged
                .get("disabled")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(Value::as_str)
                        .map(|s| s.trim().to_string())
                        .collect()
                })
                .unwrap_or_default();
            let names: Vec<String> = repo_targets.keys().cloned().collect();
            for skill_name in names {
                let override_set = overrides.get(&skill_name).cloned().unwrap_or(None);
                let drop = disabled_names.contains(&skill_name)
                    || matches!(&override_set, Some(set) if !set.contains(agent_id));
                if drop {
                    repo_targets.remove(&skill_name);
                }
            }
        }
        for (skill_name, rendered_path) in repo_targets {
            match expected.get(&skill_name) {
                Some(existing) if *existing != rendered_path => {
                    problems.push(format!("重复 skill name: {}", skill_name));
                }
                Some(_) => {}
                None => {
                    expected.insert(skill_name, rendered_path);
                }
            }
        }
    }
    (expected, problems)
}

/// collect_skill_discovery_inbox port。
pub fn collect_skill_discovery_inbox(
    root_dir: &Path,
    config: &Map<String, Value>,
    repositories: &Map<String, Value>,
    agent_targets: &Map<String, Value>,
) -> BTreeMap<String, Vec<String>> {
    let mut inbox = BTreeMap::new();
    let repo_base = PathBuf::from(resolve_loose(&root_dir.join("agent-skill").join("repositories")));
    let declared_target_ids: BTreeSet<String> = agent_targets.keys().cloned().collect();
    let skills_policy = config.get("skills").and_then(Value::as_object).cloned().unwrap_or_default();

    for (repo_name, policy) in &skills_policy {
        let policy_map = match policy.as_object() {
            Some(map) => map,
            None => continue,
        };
        if !policy_map.get("enabled").and_then(Value::as_bool).unwrap_or(true) {
            continue;
        }
        let definition = match repositories.get(repo_name).and_then(Value::as_object) {
            Some(def) => def.clone(),
            None => continue,
        };
        let mut merged = definition;
        for (key, value) in policy_map {
            merged.insert(key.clone(), value.clone());
        }
        if !repo_skill_matrix_mode(&merged) {
            continue;
        }
        let repo_path = PathBuf::from(resolve_loose(&repo_base.join(repo_name)));
        if !repo_path.exists() {
            continue;
        }
        let search_paths = match resolve_skill_search_paths(&repo_path, &merged) {
            Ok(paths) => paths,
            Err(_) => continue,
        };
        let (repo_targets, duplicate_names) = discover_repo_skill_targets(&search_paths);
        if !duplicate_names.is_empty() {
            continue;
        }
        let targeted_agents = canonicalize_agent_list(merged.get("agents").unwrap_or(&Value::Null));
        let overrides = (|| {
            validate_repo_new_skill_default(repo_name, &merged)?;
            validate_repo_skill_matrix(repo_name, &merged, &targeted_agents, &declared_target_ids)?;
            resolve_repo_skill_agent_overrides(
                repo_name,
                &merged,
                &repo_targets.keys().cloned().collect(),
            )
        })();
        let overrides = match overrides {
            Ok(overrides) => overrides,
            Err(_) => continue,
        };
        let disabled_names: BTreeSet<String> = merged
            .get("disabled")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(|s| s.trim().to_string())
                    .collect()
            })
            .unwrap_or_default();
        let mut pending: Vec<String> = overrides
            .iter()
            .filter(|(name, override_set)| {
                matches!(override_set, Some(set) if set.is_empty()) && !disabled_names.contains(*name)
            })
            .map(|(name, _)| name.clone())
            .collect();
        pending.sort();
        if !pending.is_empty() {
            inbox.insert(repo_name.clone(), pending);
        }
    }
    inbox
}

/// link_skills.is_manager_owned_repo port。
pub fn is_manager_owned_repo(repo_path: &Path) -> bool {
    repo_path.join(".agent-manager-owned").is_file()
}

/// engine_v1._skill_cell_state port。
fn skill_cell_state(
    _skill_name: &str,
    expected: Option<&String>,
    actual: Option<&String>,
    actual_broken: bool,
    repo_base: &Path,
) -> &'static str {
    if let Some(expected_path) = expected {
        let actual_path = match actual {
            Some(path) => path,
            None => return "missing-link",
        };
        if actual_broken {
            return "dead-link";
        }
        if paths_match(Some(actual_path), Some(expected_path)) {
            return "linked";
        }
        return "drifted";
    }
    let actual_path = match actual {
        Some(path) => path,
        None => return "disabled",
    };
    if actual_broken {
        return if is_managed_skill_target_path(actual_path, repo_base) {
            "dead-link"
        } else {
            "disabled"
        };
    }
    if is_managed_skill_target_path(actual_path, repo_base) {
        return "extra-link";
    }
    "disabled"
}

fn aggregate_skill_state(cell_states: &[&str]) -> &'static str {
    if cell_states
        .iter()
        .any(|state| matches!(*state, "dead-link" | "drifted" | "extra-link"))
    {
        return "drifted";
    }
    if cell_states.contains(&"missing-link") {
        return "missing";
    }
    if !cell_states.is_empty() && cell_states.iter().all(|state| *state == "disabled") {
        return "disabled";
    }
    if cell_states.contains(&"linked") {
        return "managed-ok";
    }
    "disabled"
}

fn local_git_head(repo_dir: &Path) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo_dir)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let first = text.trim().lines().next()?.trim().to_string();
    if first.is_empty() {
        None
    } else {
        Some(first)
    }
}

fn action_json(template: &str) -> Value {
    serde_json::from_str(template).unwrap()
}

fn make_element(
    id: String,
    spec: Value,
    policy: Value,
    observed: Value,
    state: &str,
    actions: Vec<Value>,
    freshness: Value,
) -> Value {
    let mut element = Map::new();
    element.insert("id".to_string(), Value::String(id));
    element.insert("domain".to_string(), Value::String("skills".to_string()));
    element.insert("spec".to_string(), spec);
    element.insert("policy".to_string(), policy);
    element.insert("observed".to_string(), observed);
    element.insert("state".to_string(), Value::String(state.to_string()));
    element.insert("freshness".to_string(), freshness);
    element.insert("actions".to_string(), Value::Array(actions));
    Value::Object(element)
}

/// build_skills_status port。
pub fn build_skills_status(
    root_dir: &Path,
    config_file: &Path,
    def_files: &[PathBuf],
) -> Result<(Value, Vec<Diagnostic>, Vec<Diagnostic>), ConfigInvalid> {
    let mut warnings = Vec::new();
    let mut errors = Vec::new();
    let config = crate::mcp::load_json_object(config_file)?;
    let config_name = config_file
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let skills_policy = crate::mcp::typed_mapping(&config, "skills", &config_name, false)?;
    let def_data = crate::mcp::load_json_object(&def_files[0])?;
    let def_name = def_files[0]
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let repositories = crate::mcp::typed_mapping(&def_data, "repositories", &def_name, false)?;
    // 定义覆盖层：merged 策略里的 skills.repositories 片段深合并到 skills.json 之上。
    let (repositories, overlay_warnings) = crate::overlay::apply_definition_overlays(
        &repositories,
        &crate::overlay::overlay_definitions(&config, "skills"),
        "skills",
    ).map_err(ConfigInvalid)?;
    for overlay_warning in overlay_warnings {
        warnings.push(diagnostic("DEFINITION_OVERLAY_UNKNOWN_KEYS", None, overlay_warning));
    }
    let agent_targets = canonicalize_agent_mapping(&Value::Object(
        crate::mcp::typed_mapping(&def_data, "agents", &def_name, true)?,
    ))
    .map_err(ConfigInvalid)?;
    let repo_base = PathBuf::from(resolve_loose(&root_dir.join("agent-skill").join("repositories")));
    let declared_target_ids: BTreeSet<String> = agent_targets.keys().cloned().collect();
    let cache = upstream::load_cache();

    // Per-agent expected/actual once（doctor 的矩阵感知语义）。
    let mut expected_by_agent: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    let mut actual_by_agent: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    let mut broken_by_agent: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut target_path_by_agent: BTreeMap<String, String> = BTreeMap::new();
    let mut target_exists_by_agent: BTreeMap<String, bool> = BTreeMap::new();
    for (agent_id, target_cfg) in &agent_targets {
        let target_path = match target_cfg {
            Value::String(path) => path.clone(),
            Value::Object(map) => map.get("path").and_then(Value::as_str).unwrap_or("").to_string(),
            _ => String::new(),
        };
        let target_path = expanduser(&target_path);
        target_path_by_agent.insert(agent_id.clone(), target_path.clone());
        let exists = Path::new(&target_path).is_dir();
        target_exists_by_agent.insert(agent_id.clone(), exists);
        let (expected, _problems) =
            expected_managed_skill_targets(root_dir, &config, &repositories, &agent_targets, agent_id);
        expected_by_agent.insert(agent_id.clone(), expected);
        if exists {
            let (actual, broken) = actual_skill_targets(Path::new(&target_path));
            actual_by_agent.insert(agent_id.clone(), actual);
            broken_by_agent.insert(agent_id.clone(), broken);
        } else {
            actual_by_agent.insert(agent_id.clone(), BTreeMap::new());
            broken_by_agent.insert(agent_id.clone(), BTreeSet::new());
        }
    }

    let inbox = collect_skill_discovery_inbox(root_dir, &config, &repositories, &agent_targets);

    // 额外受管链接：未被任何启用集合期望、但指向某集合的托管缓存仓库。
    let mut extras_by_repo: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for agent_id in agent_targets.keys() {
        let expected_names: BTreeSet<String> = expected_by_agent
            .get(agent_id.as_str())
            .map(|e| e.keys().cloned().collect())
            .unwrap_or_default();
        let actual = actual_by_agent.get(agent_id.as_str()).cloned().unwrap_or_default();
        for (name, target) in actual {
            if expected_names.contains(&name) || !is_managed_skill_target_path(&target, &repo_base) {
                continue;
            }
            for candidate in repositories.keys() {
                let candidate_base = PathBuf::from(resolve_loose(&repo_base.join(candidate)));
                if is_managed_skill_target_path(&target, &candidate_base) {
                    let mut extra = Map::new();
                    extra.insert("agent".to_string(), Value::String(agent_id.clone()));
                    extra.insert("name".to_string(), Value::String(name.clone()));
                    extra.insert(
                        "link_path".to_string(),
                        Value::String(format!(
                            "{}/{}",
                            target_path_by_agent.get(agent_id).cloned().unwrap_or_default(),
                            name
                        )),
                    );
                    extras_by_repo.entry(candidate.clone()).or_default().push(Value::Object(extra));
                    break;
                }
            }
        }
    }
    for extras in extras_by_repo.values_mut() {
        extras.sort_by_key(|extra| {
            let map = extra.as_object().unwrap();
            format!(
                "{}|{}",
                map.get("agent").and_then(Value::as_str).unwrap_or(""),
                map.get("name").and_then(Value::as_str).unwrap_or("")
            )
        });
    }

    let mut elements = Vec::new();
    let mut seen_skill_names: BTreeMap<String, String> = BTreeMap::new();
    let mut repo_names: Vec<&String> = repositories.keys().collect();
    repo_names.sort();
    for repo_name in repo_names {
        let definition = repositories[repo_name].as_object().cloned().unwrap_or_default();
        let policy = skills_policy.get(repo_name).and_then(Value::as_object).cloned();
        let declared = policy.is_some();
        let enabled = declared
            && policy
                .as_ref()
                .and_then(|p| p.get("enabled"))
                .and_then(Value::as_bool)
                .unwrap_or(true);
        let mut merged = definition.clone();
        if let Some(ref policy_map) = policy {
            for (key, value) in policy_map {
                merged.insert(key.clone(), value.clone());
            }
        }
        let matrix_mode = repo_skill_matrix_mode(&merged);
        let targeted_agents = canonicalize_agent_list(merged.get("agents").unwrap_or(&Value::Null));
        let repo_path = repo_base.join(repo_name);
        let repo_cached = repo_path.exists();
        let owner_marked = repo_cached && is_manager_owned_repo(&repo_path);

        let mut discovered: BTreeMap<String, String> = BTreeMap::new();
        let mut config_error: Option<String> = None;
        if enabled && repo_cached {
            let result = (|| -> Result<(), String> {
                validate_repo_new_skill_default(repo_name, &merged)?;
                if matrix_mode {
                    validate_repo_skill_matrix(repo_name, &merged, &targeted_agents, &declared_target_ids)?;
                }
                let repo_resolved = PathBuf::from(resolve_loose(&repo_path));
                let search_paths = resolve_skill_search_paths(&repo_resolved, &merged)?;
                let (found, duplicate_names) = discover_repo_skill_targets(&search_paths);
                if !duplicate_names.is_empty() {
                    return Err(format!(
                        "重复 skill name: {}",
                        duplicate_names.iter().cloned().collect::<Vec<_>>().join(", ")
                    ));
                }
                discovered = found;
                if matrix_mode {
                    resolve_repo_skill_agent_overrides(
                        repo_name,
                        &merged,
                        &discovered.keys().cloned().collect(),
                    )?;
                }
                Ok(())
            })();
            if let Err(exc) = result {
                config_error = Some(exc.clone());
                errors.push(diagnostic(
                    "SKILL_CONFIG_INVALID",
                    Some(format!("skill:{}", repo_name)),
                    exc,
                ));
            }
            for skill_name in discovered.keys() {
                match seen_skill_names.get(skill_name) {
                    Some(first_repo) if first_repo != repo_name => {
                        errors.push(diagnostic(
                            "SKILL_DUPLICATE_NAME",
                            Some(format!("skill:{}/{}", repo_name, skill_name)),
                            format!(
                                "skill 名 '{}' 同时由集合 {} 与 {} 提供",
                                skill_name, first_repo, repo_name
                            ),
                        ));
                    }
                    _ => {
                        seen_skill_names.insert(skill_name.clone(), repo_name.clone());
                    }
                }
            }
        }

        let mut collection_state = if !enabled {
            "disabled"
        } else if !repo_cached {
            warnings.push(diagnostic(
                "SKILL_REPO_CACHE_MISSING",
                Some(format!("skill:{}", repo_name)),
                "集合仓库缓存缺失（需 skills:apply / update 拉取）".to_string(),
            ));
            "not-materialized"
        } else if config_error.is_some() {
            "unsupported"
        } else {
            "managed-ok"
        };
        let extra_links = extras_by_repo.get(repo_name).cloned().unwrap_or_default();
        if !extra_links.is_empty() && matches!(collection_state, "managed-ok" | "disabled") {
            collection_state = "drifted";
        }

        let freshness = crate::mcp::freshness_block(
            &format!("skill:{}", repo_name),
            enabled,
            &cache,
            if repo_cached {
                local_git_head(&repo_path)
            } else {
                None
            },
            &mut warnings,
        );

        let mut spec = Map::new();
        spec.insert(
            "url".to_string(),
            definition.get("url").cloned().unwrap_or(Value::Null),
        );
        spec.insert(
            "skill_paths".to_string(),
            merged.get("skill_paths").cloned().unwrap_or(Value::Null),
        );
        spec.insert(
            "agents".to_string(),
            targeted_agents
                .clone()
                .map_or(Value::Null, |list| {
                    Value::Array(list.into_iter().map(Value::String).collect())
                }),
        );
        spec.insert("matrix_mode".to_string(), Value::Bool(matrix_mode));
        spec.insert(
            "new_skill_default".to_string(),
            Value::String(
                merged
                    .get("new_skill_default")
                    .and_then(Value::as_str)
                    .unwrap_or("disabled")
                    .to_string(),
            ),
        );
        let mut policy_block = Map::new();
        policy_block.insert("enabled".to_string(), Value::Bool(enabled));
        policy_block.insert("declared_in_policy".to_string(), Value::Bool(declared));
        let mut observed = Map::new();
        observed.insert(
            "repo_cache".to_string(),
            Value::String(repo_path.to_string_lossy().to_string()),
        );
        observed.insert("repo_cached".to_string(), Value::Bool(repo_cached));
        observed.insert("owner_marked".to_string(), Value::Bool(owner_marked));
        observed.insert(
            "discovered".to_string(),
            Value::Array(discovered.keys().cloned().map(Value::String).collect()),
        );
        observed.insert(
            "discovery_inbox".to_string(),
            Value::Array(
                inbox
                    .get(repo_name)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .map(Value::String)
                    .collect(),
            ),
        );
        observed.insert("extra_links".to_string(), Value::Array(extra_links));
        elements.push(make_element(
            format!("skill:{}", repo_name),
            Value::Object(spec),
            Value::Object(policy_block),
            Value::Object(observed),
            collection_state,
            if enabled {
                vec![action_json(SKILL_REPO_PULL_ACTION)]
            } else {
                vec![]
            },
            freshness,
        ));

        if !(enabled && repo_cached) || config_error.is_some() {
            continue;
        }
        let in_scope_agents: Vec<&String> = agent_targets
            .keys()
            .filter(|agent_id| {
                targeted_agents
                    .as_ref()
                    .map_or(true, |targeted| targeted.contains(*agent_id))
            })
            .collect();
        for skill_name in discovered.keys() {
            let mut cells = Map::new();
            for agent_id in &in_scope_agents {
                let expected = expected_by_agent
                    .get(*agent_id)
                    .and_then(|expected| expected.get(skill_name));
                let actual = actual_by_agent
                    .get(*agent_id)
                    .and_then(|actual| actual.get(skill_name));
                let broken = broken_by_agent
                    .get(*agent_id)
                    .map(|set| set.contains(skill_name))
                    .unwrap_or(false);
                let mut cell = Map::new();
                cell.insert(
                    "state".to_string(),
                    Value::String(
                        skill_cell_state(skill_name, expected, actual, broken, &repo_base).to_string(),
                    ),
                );
                cell.insert(
                    "link_path".to_string(),
                    Value::String(format!(
                        "{}/{}",
                        target_path_by_agent.get(*agent_id).cloned().unwrap_or_default(),
                        skill_name
                    )),
                );
                cell.insert(
                    "target_dir_exists".to_string(),
                    Value::Bool(*target_exists_by_agent.get(*agent_id).unwrap_or(&false)),
                );
                cells.insert((*agent_id).clone(), Value::Object(cell));
            }
            let cell_states: Vec<&str> = cells
                .values()
                .filter_map(|cell| cell.get("state").and_then(Value::as_str))
                .collect();
            let mut state = aggregate_skill_state(&cell_states);
            // 全部启用格都缺链且所有目标目录尚未建立 → 提示级 not-materialized。
            if state == "missing"
                && !cells.is_empty()
                && cells
                    .values()
                    .all(|cell| matches!(cell.get("state").and_then(Value::as_str), Some("missing-link") | Some("disabled")))
                && !cells
                    .values()
                    .any(|cell| cell.get("target_dir_exists").and_then(Value::as_bool).unwrap_or(false))
            {
                state = "not-materialized";
            }
            let in_inbox = inbox
                .get(repo_name)
                .map(|pending| pending.contains(skill_name))
                .unwrap_or(false);
            if in_inbox {
                state = "disabled";
            }

            let mut skill_spec = Map::new();
            skill_spec.insert("collection".to_string(), Value::String(repo_name.clone()));
            skill_spec.insert("name".to_string(), Value::String(skill_name.clone()));
            skill_spec.insert(
                "source_path".to_string(),
                Value::String(discovered[skill_name].clone()),
            );
            let mut skill_policy = Map::new();
            skill_policy.insert(
                "enabled".to_string(),
                Value::Bool(
                    cells
                        .values()
                        .any(|cell| cell.get("state").and_then(Value::as_str) != Some("disabled")),
                ),
            );
            let mut agents_map = Map::new();
            for agent_id in &in_scope_agents {
                let on = expected_by_agent
                    .get(*agent_id)
                    .map(|expected| expected.contains_key(skill_name))
                    .unwrap_or(false);
                agents_map.insert((*agent_id).clone(), Value::Bool(on));
            }
            skill_policy.insert("agents".to_string(), Value::Object(agents_map));
            let mut skill_observed = Map::new();
            skill_observed.insert("agents".to_string(), Value::Object(cells));
            skill_observed.insert("discovery_inbox".to_string(), Value::Bool(in_inbox));
            elements.push(make_element(
                format!("skill:{}/{}", repo_name, skill_name),
                Value::Object(skill_spec),
                Value::Object(skill_policy),
                Value::Object(skill_observed),
                state,
                vec![action_json(SKILL_APPLY_ACTION)],
                crate::mcp::freshness_not_applicable(),
            ));
        }
    }

    let mut data = Map::new();
    data.insert("domain".to_string(), Value::String("skills".to_string()));
    data.insert("elements".to_string(), Value::Array(elements));
    Ok((Value::Object(data), warnings, errors))
}

/// build_skills_plan port（集合动作在 skill 动作之前）。
pub fn build_skills_plan(
    root_dir: &Path,
    config_file: &Path,
    def_files: &[PathBuf],
) -> Result<(Value, Vec<Diagnostic>, Vec<Diagnostic>), ConfigInvalid> {
    let (status, warnings, errors) = build_skills_status(root_dir, config_file, def_files)?;
    let mut actions = Vec::new();
    let elements = status
        .get("elements")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut push_action = |op: &str, element: &str, target: Value, risk: &str, note: String| {
        let mut action = Map::new();
        action.insert("op".to_string(), Value::String(op.to_string()));
        action.insert("element".to_string(), Value::String(element.to_string()));
        action.insert("target".to_string(), target);
        action.insert("risk".to_string(), Value::String(risk.to_string()));
        action.insert("before_digest".to_string(), Value::Null);
        action.insert("after_digest".to_string(), Value::Null);
        action.insert("backup_path".to_string(), Value::Null);
        action.insert("cmd".to_string(), Value::Null);
        action.insert("note".to_string(), Value::String(note));
        actions.push(Value::Object(action));
    };

    for element in &elements {
        let element_id = element.get("id").and_then(Value::as_str).unwrap_or("");
        if element_id.contains('/') {
            continue;
        }
        let state = element.get("state").and_then(Value::as_str).unwrap_or("");
        let repo_cache = element
            .get("observed")
            .and_then(|o| o.get("repo_cache"))
            .cloned()
            .unwrap_or(Value::Null);
        if state == "not-materialized" {
            let url = element
                .get("spec")
                .and_then(|s| s.get("url"))
                .and_then(Value::as_str)
                .unwrap_or("?");
            push_action(
                "git-pull",
                element_id,
                repo_cache.clone(),
                "network",
                format!("拉取/克隆集合仓库缓存（url: {}）", url),
            );
        } else if element
            .get("freshness")
            .and_then(|f| f.get("state"))
            .and_then(Value::as_str)
            == Some("behind")
        {
            let freshness = element.get("freshness").cloned().unwrap_or(Value::Null);
            let current = freshness
                .get("current")
                .and_then(Value::as_str)
                .unwrap_or("?")
                .chars()
                .take(12)
                .collect::<String>();
            let upstream_value = freshness
                .get("upstream")
                .and_then(Value::as_str)
                .unwrap_or("?")
                .chars()
                .take(12)
                .collect::<String>();
            push_action(
                "git-pull",
                element_id,
                repo_cache.clone(),
                "network",
                format!("集合仓库落后上游（{} → {}）", current, upstream_value),
            );
        }
        let extras = element
            .get("observed")
            .and_then(|o| o.get("extra_links"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for extra in extras {
            let name = extra.get("name").and_then(Value::as_str).unwrap_or("");
            let agent = extra.get("agent").and_then(Value::as_str).unwrap_or("");
            push_action(
                "unlink",
                element_id,
                extra.get("link_path").cloned().unwrap_or(Value::Null),
                "exec-local",
                format!("移除额外受管链接 {}@{}（策略未启用）", name, agent),
            );
        }
    }
    for element in &elements {
        let element_id = element.get("id").and_then(Value::as_str).unwrap_or("");
        if !element_id.contains('/') {
            continue;
        }
        let source_path = element
            .get("spec")
            .and_then(|s| s.get("source_path"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let skill_name = element
            .get("spec")
            .and_then(|s| s.get("name"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let agents = element
            .get("observed")
            .and_then(|o| o.get("agents"))
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        for (agent_id, cell) in &agents {
            let cell_state = cell.get("state").and_then(Value::as_str).unwrap_or("");
            let link_path = cell.get("link_path").cloned().unwrap_or(Value::Null);
            match cell_state {
                "missing-link" => push_action(
                    "symlink",
                    element_id,
                    link_path,
                    "exec-local",
                    format!("启用并链接到 {}", source_path),
                ),
                "dead-link" => push_action(
                    "unlink",
                    element_id,
                    link_path,
                    "exec-local",
                    "死链清理（链接目标已不存在）".to_string(),
                ),
                "extra-link" => push_action(
                    "unlink",
                    element_id,
                    link_path,
                    "exec-local",
                    format!("移除额外受管链接（策略未启用 {}@{}）", skill_name, agent_id),
                ),
                "drifted" => push_action(
                    "symlink",
                    element_id,
                    link_path,
                    "exec-local",
                    format!("链接重定向到 {}", source_path),
                ),
                _ => {}
            }
        }
    }

    let mut summary = Map::new();
    for key in ["read", "write-local", "exec-local", "network", "install-remove"] {
        summary.insert(key.to_string(), Value::Number(0.into()));
    }
    for action in &actions {
        if let Some(risk) = action.get("risk").and_then(Value::as_str) {
            if let Some(current) = summary.get(risk).and_then(Value::as_i64) {
                summary.insert(risk.to_string(), Value::Number((current + 1).into()));
            }
        }
    }
    let mut data = Map::new();
    data.insert("domain".to_string(), Value::String("skills".to_string()));
    data.insert("actions".to_string(), Value::Array(actions));
    data.insert("summary".to_string(), Value::Object(summary));
    Ok((Value::Object(data), warnings, errors))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn frontmatter_name_parsing() {
        let dir = std::env::temp_dir().join(format!("engine_rs_skill_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let skill = dir.join("SKILL.md");
        std::fs::write(&skill, "---\nname: my-skill\ndescription: x\n---\nbody\n").unwrap();
        assert_eq!(read_skill_name(&skill), Some("my-skill".to_string()));
        std::fs::write(&skill, "---\nname: 'quoted'\n---\n").unwrap();
        assert_eq!(read_skill_name(&skill), Some("quoted".to_string()));
        std::fs::write(&skill, "no frontmatter\n").unwrap();
        assert_eq!(read_skill_name(&skill), None);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn matrix_switch_resolution() {
        let config = json!({
            "new_skill_default": "disabled",
            "skills": {
                "a": {"agents": {"Claude Code": true, "codex": false}},
                "b": {"agents": null},
                "c": {}
            }
        });
        let map = config.as_object().unwrap();
        assert_eq!(
            resolve_skill_agent_switch(map, "a"),
            Some(BTreeSet::from(["claude-code".to_string()]))
        );
        assert_eq!(resolve_skill_agent_switch(map, "b"), None);
        assert_eq!(resolve_skill_agent_switch(map, "c"), Some(BTreeSet::new()));
        let enabled_default = json!({"new_skill_default": "enabled"});
        assert_eq!(resolve_skill_agent_switch(enabled_default.as_object().unwrap(), "x"), None);
    }

    #[test]
    fn matrix_fail_close_on_unknown_skill() {
        let config = json!({"skills": {"ghost": {}}});
        let result = resolve_repo_skill_agent_overrides(
            "repo",
            config.as_object().unwrap(),
            &BTreeSet::from(["real".to_string()]),
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("含未发现的 skill: ghost"));
    }

    #[test]
    fn skill_state_aggregation() {
        assert_eq!(aggregate_skill_state(&["linked", "disabled"]), "managed-ok");
        assert_eq!(aggregate_skill_state(&["linked", "dead-link"]), "drifted");
        assert_eq!(aggregate_skill_state(&["missing-link", "disabled"]), "missing");
        assert_eq!(aggregate_skill_state(&["disabled", "disabled"]), "disabled");
        assert_eq!(aggregate_skill_state(&["extra-link"]), "drifted");
    }

    #[test]
    fn paths_match_normalizes() {
        assert!(paths_match(Some("/a/b"), Some("/a/b")));
        assert!(paths_match(Some("/a/x/../b"), Some("/a/b")));
        assert!(!paths_match(Some("/a/b"), Some("/a/bc")));
        assert!(paths_match(None, None));
        assert!(!paths_match(Some("/a"), None));
    }
}
