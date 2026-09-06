//! rules domain kernels — port of build_rules_status / build_rules_plan from
//! src/engine_v1.py, with doctor's rules_match/build_tree_signature and
//! runtime-probe scope functions (collect_active_rule_agents /
//! rule_target_agent_id) ported alongside.
//!
//! Note: doctor's tree signature uses MD5 per file; this port uses SHA-256 for
//! the internal signature map instead (md5 is outside the dependency
//! whitelist). Verdicts are identical; only directory digests differ from the
//! Python engine (file targets — all current manifest entries — match exactly).

use crate::canon::{sha256_canonical, sha256_hex};
use crate::envelope::{diagnostic, Diagnostic};
use crate::ids::{canonicalize_agent_id, canonicalize_agent_list, canonicalize_agent_mapping};
use crate::mcp::ConfigInvalid;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

const RULES_APPLY_ACTION: &str =
    "{\"id\": \"apply\", \"risk\": \"write-local\", \"needs_lock\": true, \"offline_ok\": true}";

struct ManifestEntry {
    name: String,
    group: String,
    source: String,
    target: String,
}

/// _parse_rules_manifest port：name|group|source|target 四列，非法行进 problems。
fn parse_rules_manifest(path: &Path) -> (Vec<ManifestEntry>, Vec<String>) {
    let mut entries = Vec::new();
    let mut problems = Vec::new();
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(_) => return (entries, problems),
    };
    for raw_line in text.lines() {
        let stripped = raw_line.trim();
        if stripped.is_empty() || stripped.starts_with('#') {
            continue;
        }
        let parts: Vec<String> = stripped.split('|').map(|p| p.trim().to_string()).collect();
        if parts.len() < 4 {
            problems.push(stripped.to_string());
            continue;
        }
        entries.push(ManifestEntry {
            name: parts[0].clone(),
            group: parts[1].clone(),
            source: parts[2].clone(),
            target: parts[3].clone(),
        });
    }
    (entries, problems)
}

/// rule_target_agent_id port：前缀映射 + canonicalize 兜底。
fn rule_target_agent_id(target_name: &str) -> String {
    let prefix = target_name.split('_').next().unwrap_or(target_name);
    match prefix {
        "claude" => "claude-code".to_string(),
        "codex" => "codex".to_string(),
        other => canonicalize_agent_id(other),
    }
}

fn expand_targeted_agents(agent_ids: &Value, available: &BTreeSet<String>) -> BTreeSet<String> {
    match canonicalize_agent_list(agent_ids) {
        None => available.clone(),
        Some(list) => list.into_iter().filter(|id| available.contains(id)).collect(),
    }
}

/// collect_active_rule_agents port：启用的 agents ∪ skills 目标 ∪ mcp 目标。
fn collect_active_rule_agents(
    root_dir: &Path,
    config: &Map<String, Value>,
) -> Result<BTreeSet<String>, ConfigInvalid> {
    let mut active = BTreeSet::new();
    let agents_raw = config.get("agents").cloned().unwrap_or(Value::Object(Map::new()));
    let agents_policy = canonicalize_agent_mapping(&agents_raw).map_err(ConfigInvalid)?;
    for (agent_id, policy) in &agents_policy {
        let enabled = policy
            .as_object()
            .and_then(|p| p.get("enabled"))
            .and_then(Value::as_bool)
            .unwrap_or(true);
        if enabled {
            active.insert(agent_id.clone());
        }
    }

    let skills_raw = crate::mcp::load_json_object(&root_dir.join("config").join("skills.json"))?;
    let repositories = crate::mcp::typed_mapping(&skills_raw, "repositories", "skills.json", false)?;
    let skill_agents = canonicalize_agent_mapping(&Value::Object(
        crate::mcp::typed_mapping(&skills_raw, "agents", "skills.json", true)?,
    ))
    .map_err(ConfigInvalid)?;
    let skill_available: BTreeSet<String> = skill_agents.keys().cloned().collect();
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
        let agents_value = merged.get("agents").cloned().unwrap_or(Value::Null);
        active.extend(expand_targeted_agents(&agents_value, &skill_available));
    }

    let mcp_raw = crate::mcp::load_json_object(&root_dir.join("config").join("mcp.json"))?;
    let mcp_servers = crate::mcp::typed_mapping(&mcp_raw, "servers", "mcp.json", false)?;
    let mcp_agent_paths = canonicalize_agent_mapping(&Value::Object(
        crate::mcp::typed_mapping(&mcp_raw, "agent_paths", "mcp.json", true)?,
    ))
    .map_err(ConfigInvalid)?;
    let mcp_available: BTreeSet<String> = mcp_agent_paths.keys().cloned().collect();
    let mcp_policy = config.get("mcp").and_then(Value::as_object).cloned().unwrap_or_default();
    for (server_name, policy) in &mcp_policy {
        let policy_map = match policy.as_object() {
            Some(map) => map,
            None => continue,
        };
        if !policy_map.get("enabled").and_then(Value::as_bool).unwrap_or(true) {
            continue;
        }
        let definition = match mcp_servers.get(server_name).and_then(Value::as_object) {
            Some(def) => def.clone(),
            None => continue,
        };
        let mut merged = definition;
        for (key, value) in policy_map {
            merged.insert(key.clone(), value.clone());
        }
        let agents_value = merged.get("agents").cloned().unwrap_or(Value::Null);
        active.extend(expand_targeted_agents(&agents_value, &mcp_available));
    }
    Ok(active)
}

/// _latest_mtime port：目录树最新 mtime（目录不存在 → None）。仅作观测字段，
/// 不再驱动状态（mtime 在全新 checkout 上不可信）。
fn latest_mtime(root: &Path) -> Option<f64> {
    if !root.is_dir() {
        return None;
    }
    let mut latest: Option<f64> = None;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if let Ok(meta) = path.metadata() {
                    if let Ok(mtime) = meta.modified() {
                        if let Ok(secs) = mtime.duration_since(std::time::UNIX_EPOCH) {
                            let value = secs.as_secs_f64();
                            if latest.map_or(true, |current| value > current) {
                                latest = Some(value);
                            }
                        }
                    }
                }
            }
        }
    }
    latest
}

fn file_digest(path: &Path) -> Option<String> {
    std::fs::read(path).ok().map(|bytes| sha256_hex(&bytes))
}

/// build_tree_signature port（sha256 替 md5，见模块注释；结构与语义一致）。
fn build_tree_signature(root: &Path) -> BTreeMap<String, String> {
    let mut signature = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let mut dirs = Vec::new();
        let mut files = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                if entry.path().is_dir() {
                    dirs.push(entry.path());
                } else {
                    files.push(entry.path());
                }
            }
        }
        dirs.sort();
        files.sort();
        for dirname in dirs.iter().rev() {
            stack.push(dirname.clone());
        }
        let rel_dir = dir.strip_prefix(root).unwrap_or(&dir).to_string_lossy().replace('\\', "/");
        let rel_dir = rel_dir.trim_start_matches("./").to_string();
        for dirname in &dirs {
            let name = dirname.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let rel = if rel_dir.is_empty() {
                format!("{}/", name)
            } else {
                format!("{}/{}/", rel_dir, name)
            };
            signature.insert(rel, "dir".to_string());
        }
        for file in files {
            let name = file.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let rel = if rel_dir.is_empty() {
                name
            } else {
                format!("{}/{}", rel_dir, name)
            };
            signature.insert(rel, file_digest(&file).unwrap_or_default());
        }
    }
    signature
}

/// rules_match port：目录/文件形态不一致即不等；目录比签名，文件比内容。
fn rules_match(src: &Path, dst: &Path) -> bool {
    if src.is_dir() || dst.is_dir() {
        if !(src.is_dir() && dst.is_dir()) {
            return false;
        }
        return build_tree_signature(src) == build_tree_signature(dst);
    }
    file_digest(src) == file_digest(dst)
}

fn sha256_path(path: &Path) -> Option<String> {
    if path.is_dir() {
        let signature = build_tree_signature(path);
        let map: Map<String, Value> = signature
            .into_iter()
            .map(|(k, v)| (k, Value::String(v)))
            .collect();
        return Some(sha256_canonical(&Value::Object(map)));
    }
    crate::canon::sha256_file(path)
}

fn make_element(id: String, spec: Value, policy: Value, observed: Value, state: &str, actions: Vec<Value>) -> Value {
    let mut element = Map::new();
    element.insert("id".to_string(), Value::String(id));
    element.insert("domain".to_string(), Value::String("rules".to_string()));
    element.insert("spec".to_string(), spec);
    element.insert("policy".to_string(), policy);
    element.insert("observed".to_string(), observed);
    element.insert("state".to_string(), Value::String(state.to_string()));
    element.insert("freshness".to_string(), crate::mcp::freshness_not_applicable());
    element.insert("actions".to_string(), Value::Array(actions));
    Value::Object(element)
}

/// build-agent-rules.py 渲染原语移植：compact_lines（右裁剪、去空行）。
fn compact_lines(text: &str) -> Vec<String> {
    text.lines()
        .map(|line| line.trim_end().to_string())
        .filter(|line| !line.is_empty())
        .collect()
}

/// render_target_content port：core + adapter 拼接，尾部换行。
fn render_target_content(core_lines: &[String], adapter_lines: &[String]) -> String {
    let mut lines: Vec<String> = Vec::new();
    lines.extend(core_lines.iter().cloned());
    lines.extend(adapter_lines.iter().cloned());
    format!("{}\n", lines.join("\n"))
}

/// TARGET_SPECS 与 build-agent-rules.py 的静态表一致（跨语言内联拷贝）。
fn target_spec(dir_name: &str) -> Option<(&'static str, &'static str)> {
    match dir_name {
        "codex" => Some(("codex", "AGENTS.md")),
        "claude" => Some(("claude", "CLAUDE.md")),
        _ => None,
    }
}

/// _rules_dist_expected_content port：rules/dist/<target>/<filename> 形态的产物
/// 才参与渲染比对；未知产物或 src 缺失 → None（不参与 stale 判定）。
fn rules_dist_expected_content(root_dir: &Path, source_rel: &str) -> Option<String> {
    let parts: Vec<&str> = source_rel.split('/').collect();
    if parts.len() != 4 || parts[0] != "rules" || parts[1] != "dist" {
        return None;
    }
    let (adapter, filename) = target_spec(parts[2])?;
    if filename != parts[3] {
        return None;
    }
    let src_root = root_dir.join("agent-rule").join("rules").join("src");
    let core = std::fs::read_to_string(src_root.join("shared").join("core.md")).ok()?;
    let adapter_text =
        std::fs::read_to_string(src_root.join("agents").join(adapter).join("adapter.md")).ok()?;
    Some(render_target_content(
        &compact_lines(&core),
        &compact_lines(&adapter_text),
    ))
}

/// build_rules_status port。
pub fn build_rules_status(
    root_dir: &Path,
    config_file: &Path,
    def_files: &[PathBuf],
) -> Result<(Value, Vec<Diagnostic>, Vec<Diagnostic>), ConfigInvalid> {
    let mut warnings = Vec::new();
    let mut errors = Vec::new();
    let config = crate::mcp::load_json_object(config_file)?;
    let (entries, problems) = parse_rules_manifest(&def_files[0]);
    for problem in problems {
        errors.push(diagnostic(
            "RULES_MANIFEST_INVALID",
            None,
            format!("manifest 非法行: {}", problem),
        ));
    }
    let active_agents = collect_active_rule_agents(root_dir, &config)?;
    let rules_src_dir = root_dir.join("agent-rule").join("rules").join("src");
    let src_latest_mtime = latest_mtime(&rules_src_dir);
    let home = std::env::var("HOME").unwrap_or_else(|_| "~".to_string());

    let mut elements = Vec::new();
    let mut stale_products: Vec<String> = Vec::new();
    let mut missing_products: Vec<String> = Vec::new();
    let mut seen_sources: Vec<String> = Vec::new();
    for entry in &entries {
        let agent_id = rule_target_agent_id(&entry.name);
        let is_repo_root_target = entry.target.contains("{{REPO_ROOT}}");
        let in_scope = is_repo_root_target || active_agents.contains(&agent_id);
        let src = root_dir.join("agent-rule").join(&entry.source);
        let dst = crate::providers::expanduser(
            &entry
                .target
                .replace("{{HOME}}", &home)
                .replace("{{REPO_ROOT}}", &root_dir.to_string_lossy()),
        );
        if !seen_sources.contains(&entry.source) {
            seen_sources.push(entry.source.clone());
        }
        let source_exists = src.exists();
        let target_exists = Path::new(&dst).exists();
        // dist 新鲜度 = 内容语义：当前 src 渲染期望 ≠ dist 现有内容 → 落后。
        let expected_content = rules_dist_expected_content(root_dir, &entry.source);
        let build_stale = match (&expected_content, source_exists) {
            (Some(expected), true) => {
                std::fs::read_to_string(&src).ok().as_deref() != Some(expected.as_str())
            }
            _ => false,
        };
        if !source_exists {
            missing_products.push(entry.source.clone());
        } else if build_stale {
            stale_products.push(entry.source.clone());
        }

        let state = if !in_scope {
            "disabled"
        } else if !source_exists {
            warnings.push(diagnostic(
                "RULE_SOURCE_NOT_BUILT",
                Some(format!("rules:{}", entry.name)),
                format!("dist 产物 {} 尚未生成（需先构建 rules）", entry.source),
            ));
            "not-materialized"
        } else if !target_exists {
            "not-materialized"
        } else if rules_match(&src, Path::new(&dst)) {
            "managed-ok"
        } else {
            "drifted"
        };
        if build_stale && in_scope && state == "managed-ok" {
            warnings.push(diagnostic(
                "RULE_BUILD_STALE",
                Some(format!("rules:{}", entry.name)),
                "rules/src 比 dist 产物新，构建落后（提示级）".to_string(),
            ));
        }

        let mut spec = Map::new();
        spec.insert("name".to_string(), Value::String(entry.name.clone()));
        spec.insert("group".to_string(), Value::String(entry.group.clone()));
        spec.insert("source".to_string(), Value::String(entry.source.clone()));
        spec.insert("target".to_string(), Value::String(entry.target.clone()));
        let mut policy = Map::new();
        policy.insert("enabled".to_string(), Value::Bool(in_scope));
        let mut observed = Map::new();
        observed.insert("source".to_string(), Value::String(src.to_string_lossy().to_string()));
        observed.insert("target".to_string(), Value::String(dst.clone()));
        observed.insert("source_exists".to_string(), Value::Bool(source_exists));
        observed.insert("target_exists".to_string(), Value::Bool(target_exists));
        observed.insert("build_stale".to_string(), Value::Bool(build_stale));
        elements.push(make_element(
            format!("rules:{}", entry.name),
            Value::Object(spec),
            Value::Object(policy),
            Value::Object(observed),
            state,
            if in_scope {
                vec![serde_json::from_str(RULES_APPLY_ACTION).unwrap()]
            } else {
                vec![]
            },
        ));
    }

    let dist_state = if !missing_products.is_empty() {
        "not-materialized"
    } else if !stale_products.is_empty() {
        "drifted"
    } else {
        "managed-ok"
    };
    let mut dist_spec = Map::new();
    dist_spec.insert("source_dir".to_string(), Value::String("agent-rule/rules/src".to_string()));
    dist_spec.insert("dist_dir".to_string(), Value::String("agent-rule/rules/dist".to_string()));
    let mut dist_policy = Map::new();
    dist_policy.insert("enabled".to_string(), Value::Bool(true));
    let mut dist_observed = Map::new();
    dist_observed.insert(
        "src_latest_mtime".to_string(),
        src_latest_mtime.map_or(Value::Null, |v| {
            serde_json::Number::from_f64(v).map_or(Value::Null, Value::Number)
        }),
    );
    dist_observed.insert(
        "products".to_string(),
        Value::Array(seen_sources.iter().cloned().map(Value::String).collect()),
    );
    dist_observed.insert(
        "missing_products".to_string(),
        Value::Array(missing_products.iter().cloned().map(Value::String).collect()),
    );
    dist_observed.insert(
        "stale_products".to_string(),
        Value::Array(stale_products.iter().cloned().map(Value::String).collect()),
    );
    elements.push(make_element(
        "rules:dist".to_string(),
        Value::Object(dist_spec),
        Value::Object(dist_policy),
        Value::Object(dist_observed),
        dist_state,
        vec![serde_json::from_str(RULES_APPLY_ACTION).unwrap()],
    ));

    let mut data = Map::new();
    data.insert("domain".to_string(), Value::String("rules".to_string()));
    data.insert("elements".to_string(), Value::Array(elements));
    Ok((Value::Object(data), warnings, errors))
}

/// build_rules_plan port（dist 构建动作排在分发动作前）。
pub fn build_rules_plan(
    root_dir: &Path,
    config_file: &Path,
    def_files: &[PathBuf],
) -> Result<(Value, Vec<Diagnostic>, Vec<Diagnostic>), ConfigInvalid> {
    let (status, warnings, errors) = build_rules_status(root_dir, config_file, def_files)?;
    let elements = status
        .get("elements")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut ordered: Vec<&Value> = elements.iter().filter(|e| e.get("id").and_then(Value::as_str) == Some("rules:dist")).collect();
    ordered.extend(elements.iter().filter(|e| e.get("id").and_then(Value::as_str) != Some("rules:dist")));

    let mut actions = Vec::new();
    for element in ordered {
        let element_id = element.get("id").and_then(Value::as_str).unwrap_or("").to_string();
        let state = element.get("state").and_then(Value::as_str).unwrap_or("");
        if element_id == "rules:dist" {
            if state == "drifted" || state == "not-materialized" {
                actions.push(crate::canon::plan_action(
                    "run-cmd",
                    &element_id,
                    Value::Null,
                    "exec-local",
                    None,
                    None,
                    Some("python3 agent-rule/scripts/build-agent-rules.py".to_string()),
                    Some("先重建 dist，再分发（构建落后/产物缺失）".to_string()),
                ));
            }
            continue;
        }
        if !matches!(state, "drifted" | "missing" | "not-materialized") {
            continue;
        }
        let observed = element.get("observed").cloned().unwrap_or(Value::Null);
        if observed.get("source_exists").and_then(Value::as_bool) != Some(true) {
            continue; // 构建动作会覆盖该情况
        }
        let target = observed.get("target").and_then(Value::as_str).unwrap_or("").to_string();
        let source = observed.get("source").and_then(Value::as_str).unwrap_or("").to_string();
        let target_exists = observed.get("target_exists").and_then(Value::as_bool).unwrap_or(false);
        let is_dir = Path::new(&source).is_dir();
        let before = if target_exists {
            sha256_path(Path::new(&target))
        } else {
            None
        };
        let after = sha256_path(Path::new(&source));
        actions.push(crate::canon::plan_action(
            "write-file",
            &element_id,
            Value::String(target),
            "write-local",
            before,
            after,
            None,
            Some(if is_dir { "目录收敛同步" } else { "分发 rules 产物" }.to_string()),
        ));
    }

    let summary = crate::canon::plan_summary(&actions);
    let mut data = Map::new();
    data.insert("domain".to_string(), Value::String("rules".to_string()));
    data.insert("actions".to_string(), Value::Array(actions));
    data.insert("summary".to_string(), summary);
    Ok((Value::Object(data), warnings, errors))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_matches_python_builder() {
        // python3 -c 对齐 build-agent-rules.py 的 compact_lines/render_target_content:
        // core "a\n\n b \n" → ["a","b"]; adapter "c\n" → ["c"]; render → "a\nb\nc\n"
        // compact_lines 对齐 Python line.rstrip()：只裁右端，左端空白保留。
        assert_eq!(compact_lines("a\n\n b \n"), vec!["a", " b"]);
        assert_eq!(render_target_content(&["x".to_string()], &["y".to_string()]), "x\ny\n");
        let core = compact_lines("core line\n\nsecond\n");
        let adapter = compact_lines("adapter\n");
        assert_eq!(render_target_content(&core, &adapter), "core line\nsecond\nadapter\n");
    }

    #[test]
    fn manifest_parsing_collects_problems() {
        let dir = std::env::temp_dir().join(format!("engine_rs_rules_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let manifest = dir.join("rules-targets.list");
        std::fs::write(&manifest, "# comment\n\nok|global|src/a.md|{{HOME}}/.x/a.md\nbroken-line\n").unwrap();
        let (entries, problems) = parse_rules_manifest(&manifest);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "ok");
        assert_eq!(problems, vec!["broken-line".to_string()]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn target_agent_prefix_mapping() {
        assert_eq!(rule_target_agent_id("claude_global"), "claude-code");
        assert_eq!(rule_target_agent_id("codex_global"), "codex");
        assert_eq!(rule_target_agent_id("Kimi Code_x"), "kimi-code");
    }

    #[test]
    fn tree_signature_detects_content_and_shape() {
        let dir = std::env::temp_dir().join(format!("engine_rs_tree_test_{}", std::process::id()));
        let left = dir.join("left");
        let right = dir.join("right");
        std::fs::create_dir_all(left.join("sub")).unwrap();
        std::fs::create_dir_all(right.join("sub")).unwrap();
        std::fs::write(left.join("sub/a.md"), "same").unwrap();
        std::fs::write(right.join("sub/a.md"), "same").unwrap();
        assert_eq!(build_tree_signature(&left), build_tree_signature(&right));
        std::fs::write(right.join("sub/a.md"), "different").unwrap();
        assert_ne!(build_tree_signature(&left), build_tree_signature(&right));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rules_match_file_and_dir_semantics() {
        let dir = std::env::temp_dir().join(format!("engine_rs_match_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("a.md");
        let b = dir.join("b.md");
        std::fs::write(&a, "x").unwrap();
        std::fs::write(&b, "x").unwrap();
        assert!(rules_match(&a, &b));
        std::fs::write(&b, "y").unwrap();
        assert!(!rules_match(&a, &b));
        assert!(!rules_match(&a, &dir));
        std::fs::remove_dir_all(&dir).ok();
    }
}
