//! hooks domain kernels — port of build_hooks_status / build_hooks_plan from
//! src/engine_v1.py, with the sync-claude-hooks.py / sync-codex-hooks.py
//! primitives (signature collect / strip removed / strip managed / merge /
//! whole-file render) ported byte-faithfully.
//!
//! claude: signature-set merge into repo .claude/settings.json (user content
//! outside managed signatures is preserved). codex: whole-file render into
//! .codex/hooks.json (any deviation = drift).

use crate::canon::{json_equal, sha256_hex};
use crate::envelope::{diagnostic, Diagnostic};
use crate::mcp::ConfigInvalid;
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

const HOOKS_APPLY_ACTION: &str =
    "{\"id\": \"apply\", \"risk\": \"write-local\", \"needs_lock\": true, \"offline_ok\": true}";

type HookMap = Map<String, Value>; // event -> [entry]
type Signature = (String, Option<String>, String);
type RemovedRule = (String, Option<String>, Option<String>, Option<String>); // event, matcher, command, command_contains

fn validate_hook_entry(entry: &Value, source_name: &str) -> Result<Value, String> {
    let entry = entry
        .as_object()
        .ok_or_else(|| format!("{} 条目必须是对象", source_name))?;
    let hooks = entry.get("hooks");
    let hooks_list = match hooks.and_then(Value::as_array) {
        Some(list) if !list.is_empty() => list,
        _ => return Err(format!("{}.hooks 必须是非空数组", source_name)),
    };
    if let Some(matcher) = entry.get("matcher") {
        if !matcher.is_string() {
            return Err(format!("{}.matcher 必须是字符串", source_name));
        }
    }
    for (index, hook) in hooks_list.iter().enumerate() {
        let hook_map = hook
            .as_object()
            .ok_or_else(|| format!("{}.hooks[{}] 必须是对象", source_name, index))?;
        if hook_map.get("type").and_then(Value::as_str) != Some("command") {
            return Err(format!("{}.hooks[{}].type 必须是 command", source_name, index));
        }
        let command = hook_map.get("command").and_then(Value::as_str).unwrap_or("");
        if command.trim().is_empty() {
            return Err(format!("{}.hooks[{}].command 必须是非空字符串", source_name, index));
        }
    }
    Ok(Value::Object(entry.clone()))
}

fn validate_hooks_map(raw: Option<&Value>, source_name: &str) -> Result<HookMap, String> {
    let map = raw
        .and_then(Value::as_object)
        .ok_or_else(|| format!("{}.hooks 必须是对象", source_name))?;
    let mut validated = Map::new();
    for (event_name, entries) in map {
        let list = entries.as_array().filter(|list| !list.is_empty()).ok_or_else(|| {
            format!("{}.hooks.{} 必须是非空数组", source_name, event_name)
        })?;
        let mut kept = Vec::new();
        for (index, entry) in list.iter().enumerate() {
            kept.push(validate_hook_entry(
                entry,
                &format!("{}.hooks.{}[{}]", source_name, event_name, index),
            )?);
        }
        validated.insert(event_name.clone(), Value::Array(kept));
    }
    Ok(validated)
}

fn validate_removed_signatures(raw: Option<&Value>, source_name: &str) -> Result<Vec<RemovedRule>, String> {
    let mut validated = Vec::new();
    let raw = match raw {
        None | Some(Value::Null) => return Ok(validated),
        Some(value) => value,
    };
    let items = raw
        .as_array()
        .ok_or_else(|| format!("{}.removed_signatures 必须是数组", source_name))?;
    for (index, entry) in items.iter().enumerate() {
        let label = format!("{}.removed_signatures[{}]", source_name, index);
        let entry = entry
            .as_object()
            .ok_or_else(|| format!("{} 必须是对象", label))?;
        let event = entry.get("event").and_then(Value::as_str).unwrap_or("");
        if event.trim().is_empty() {
            return Err(format!("{}.event 必须是非空字符串", label));
        }
        let matcher = entry.get("matcher").and_then(Value::as_str).map(str::to_string);
        if entry.get("matcher").is_some() && matcher.is_none() {
            return Err(format!("{}.matcher 必须是字符串或省略", label));
        }
        let command = entry.get("command").and_then(Value::as_str).map(str::to_string);
        let command_contains = entry
            .get("command_contains")
            .and_then(Value::as_str)
            .map(str::to_string);
        if command.is_none() && command_contains.is_none() {
            return Err(format!("{} 必须提供 command 或 command_contains", label));
        }
        if let Some(ref exact) = command {
            if exact.is_empty() {
                return Err(format!("{}.command 必须是非空字符串", label));
            }
        }
        if let Some(ref sub) = command_contains {
            if sub.is_empty() {
                return Err(format!("{}.command_contains 必须是非空字符串", label));
            }
        }
        validated.push((event.to_string(), matcher, command, command_contains));
    }
    Ok(validated)
}

/// sync-claude-hooks.py build_managed_hooks port：(settings_path, hooks, removed)。
fn build_managed_hooks_claude(config_path: &Path) -> Result<(PathBuf, HookMap, Vec<RemovedRule>), String> {
    let config = crate::mcp::load_json_object(config_path).map_err(|exc| exc.0)?;
    let settings_rel = config.get("settings_path").and_then(Value::as_str).unwrap_or("");
    if settings_rel.trim().is_empty() {
        return Err("claude-hooks.json.settings_path 必须是非空字符串".to_string());
    }
    let hooks = validate_hooks_map(config.get("hooks"), "claude-hooks.json")?;
    let removed = validate_removed_signatures(config.get("removed_signatures"), "claude-hooks.json")?;
    let settings_path = PathBuf::from(settings_rel);
    let settings_path = if settings_path.is_absolute() {
        settings_path
    } else {
        config_path
            .parent()
            .map(|p| p.join(&settings_path))
            .unwrap_or(settings_path)
    };
    Ok((settings_path, hooks, removed))
}

/// sync-codex-hooks.py build_managed_hooks port：(hooks_path, hooks)。
fn build_managed_hooks_codex(config_path: &Path) -> Result<(PathBuf, HookMap), String> {
    let config = crate::mcp::load_json_object(config_path).map_err(|exc| exc.0)?;
    let hooks_rel = config.get("hooks_path").and_then(Value::as_str).unwrap_or("");
    if hooks_rel.trim().is_empty() {
        return Err("codex-hooks.json.hooks_path 必须是非空字符串".to_string());
    }
    let hooks = validate_hooks_map(config.get("hooks"), "codex-hooks.json")?;
    let hooks_path = PathBuf::from(hooks_rel);
    let hooks_path = if hooks_path.is_absolute() {
        hooks_path
    } else {
        config_path.parent().map(|p| p.join(&hooks_path)).unwrap_or(hooks_path)
    };
    Ok((hooks_path, hooks))
}

/// _collect_managed_signatures port。
fn collect_signatures(hooks: &HookMap) -> Vec<Signature> {
    let mut signatures = Vec::new();
    for (event_name, entries) in hooks {
        if let Some(list) = entries.as_array() {
            for entry in list {
                let matcher = entry
                    .as_object()
                    .and_then(|e| e.get("matcher"))
                    .and_then(Value::as_str)
                    .map(str::to_string);
                if let Some(hooks_list) = entry.as_object().and_then(|e| e.get("hooks")).and_then(Value::as_array) {
                    for hook in hooks_list {
                        if let Some(command) = hook.as_object().and_then(|h| h.get("command")).and_then(Value::as_str) {
                            let signature = (event_name.clone(), matcher.clone(), command.to_string());
                            if !signatures.contains(&signature) {
                                signatures.push(signature);
                            }
                        }
                    }
                }
            }
        }
    }
    signatures
}

/// engine_v1._collect_present_hook_signatures port（容错版）。
fn collect_present_signatures(existing_hooks: &Value) -> Vec<Signature> {
    let mut signatures = Vec::new();
    let map = match existing_hooks.as_object() {
        Some(map) => map,
        None => return signatures,
    };
    for (event_name, entries) in map {
        let entries = match entries.as_array() {
            Some(list) => list,
            None => continue,
        };
        for entry in entries {
            let entry = match entry.as_object() {
                Some(entry) => entry,
                None => continue,
            };
            let matcher = entry
                .get("matcher")
                .and_then(Value::as_str)
                .map(str::to_string);
            let hooks_list = match entry.get("hooks").and_then(Value::as_array) {
                Some(list) => list,
                None => continue,
            };
            for hook in hooks_list {
                if let Some(command) = hook.as_object().and_then(|h| h.get("command")).and_then(Value::as_str) {
                    let signature = (event_name.clone(), matcher.clone(), command.to_string());
                    if !signatures.contains(&signature) {
                        signatures.push(signature);
                    }
                }
            }
        }
    }
    signatures
}

fn removed_rule_present(rule: &RemovedRule, present: &[Signature]) -> bool {
    let (event, matcher, command, command_contains) = rule;
    for (p_event, p_matcher, p_command) in present {
        if p_event != event {
            continue;
        }
        if let Some(m) = matcher {
            if p_matcher.as_deref() != Some(m.as_str()) {
                continue;
            }
        }
        if let Some(c) = command {
            if p_command != c {
                continue;
            }
        }
        if let Some(sub) = command_contains {
            if !p_command.contains(sub.as_str()) {
                continue;
            }
        }
        return true;
    }
    false
}

/// _strip_removed_signatures port。
fn strip_removed_signatures(existing_hooks: &Value, removed: &[RemovedRule]) -> Value {
    let map = match existing_hooks.as_object() {
        Some(map) => map,
        None => return existing_hooks.clone(),
    };
    if removed.is_empty() {
        return existing_hooks.clone();
    }
    let mut cleaned = Map::new();
    for (event_name, entries) in map {
        let list = match entries.as_array() {
            Some(list) => list,
            None => {
                cleaned.insert(event_name.clone(), entries.clone());
                continue;
            }
        };
        let mut kept_entries = Vec::new();
        for entry in list {
            let entry_map = match entry.as_object() {
                Some(map) => map,
                None => {
                    kept_entries.push(entry.clone());
                    continue;
                }
            };
            let hooks_list = match entry_map.get("hooks").and_then(Value::as_array) {
                Some(list) => list,
                None => {
                    kept_entries.push(entry.clone());
                    continue;
                }
            };
            let matcher = entry_map.get("matcher").and_then(Value::as_str).map(str::to_string);
            let kept_hooks: Vec<Value> = hooks_list
                .iter()
                .filter(|hook| {
                    let command = hook.as_object().and_then(|h| h.get("command")).and_then(Value::as_str);
                    match command {
                        Some(command) => !removed.iter().any(|rule| {
                            let candidate = (event_name.clone(), matcher.clone(), command.to_string());
                            removed_rule_present(rule, &[candidate])
                        }),
                        None => true,
                    }
                })
                .cloned()
                .collect();
            if !kept_hooks.is_empty() {
                let mut new_entry = entry_map.clone();
                new_entry.insert("hooks".to_string(), Value::Array(kept_hooks));
                kept_entries.push(Value::Object(new_entry));
            }
        }
        if !kept_entries.is_empty() {
            cleaned.insert(event_name.clone(), Value::Array(kept_entries));
        }
    }
    Value::Object(cleaned)
}

/// _strip_managed_hooks port。
fn strip_managed_hooks(existing_hooks: &Value, managed_signatures: &[Signature]) -> Value {
    let map = match existing_hooks.as_object() {
        Some(map) => map,
        None => return Value::Object(Map::new()),
    };
    let mut cleaned = Map::new();
    for (event_name, entries) in map {
        let list = match entries.as_array() {
            Some(list) => list,
            None => {
                cleaned.insert(event_name.clone(), entries.clone());
                continue;
            }
        };
        let mut kept_entries = Vec::new();
        for entry in list {
            let entry_map = match entry.as_object() {
                Some(map) => map,
                None => {
                    kept_entries.push(entry.clone());
                    continue;
                }
            };
            let hooks_list = match entry_map.get("hooks").and_then(Value::as_array) {
                Some(list) => list,
                None => {
                    kept_entries.push(entry.clone());
                    continue;
                }
            };
            let matcher = entry_map.get("matcher").and_then(Value::as_str).map(str::to_string);
            let kept_hooks: Vec<Value> = hooks_list
                .iter()
                .filter(|hook| {
                    let command = hook.as_object().and_then(|h| h.get("command")).and_then(Value::as_str);
                    match command {
                        Some(command) => !managed_signatures
                            .contains(&(event_name.clone(), matcher.clone(), command.to_string())),
                        None => true,
                    }
                })
                .cloned()
                .collect();
            if !kept_hooks.is_empty() {
                let mut new_entry = entry_map.clone();
                new_entry.insert("hooks".to_string(), Value::Array(kept_hooks));
                kept_entries.push(Value::Object(new_entry));
            }
        }
        if !kept_entries.is_empty() {
            cleaned.insert(event_name.clone(), Value::Array(kept_entries));
        }
    }
    Value::Object(cleaned)
}

/// engine_v1._claude_hooks_expected_render port（合并语义 + 写盘格式 digest）。
fn claude_expected_render(settings: &Value, managed_hooks: &HookMap, removed: &[RemovedRule]) -> Value {
    let managed_signatures = collect_signatures(managed_hooks);
    let hooks_value = settings.get("hooks").cloned().unwrap_or(Value::Object(Map::new()));
    let existing_hooks = strip_removed_signatures(&hooks_value, removed);
    let mut merged_hooks = strip_managed_hooks(&existing_hooks, &managed_signatures)
        .as_object()
        .cloned()
        .unwrap_or_default();
    for (event_name, entries) in managed_hooks {
        let entry_list = merged_hooks
            .entry(event_name.clone())
            .or_insert_with(|| Value::Array(Vec::new()));
        if let (Value::Array(target), Value::Array(items)) = (entry_list, entries) {
            target.extend(items.iter().cloned());
        }
    }
    let mut merged = settings.as_object().cloned().unwrap_or_default();
    merged.insert("hooks".to_string(), Value::Object(merged_hooks));
    Value::Object(merged)
}

/// atomic_write_json_file 字节格式：json.dumps(indent=2, ensure_ascii=False) + "\n"。
fn rendered_json_digest(payload: &Value) -> String {
    let text = serde_json::to_string_pretty(payload).expect("render") + "\n";
    format!("sha256:{}", sha256_hex(text.as_bytes()))
}

fn load_settings(path: &Path) -> Result<Map<String, Value>, String> {
    crate::mcp::load_json_object(path).map_err(|exc| exc.0)
}

struct HooksObserved {
    observed: Map<String, Value>,
    state: &'static str,
    error: Option<String>,
}

fn claude_status(claude_def: &Path) -> Result<HooksObserved, ConfigInvalid> {
    let (settings_path, managed_hooks, removed) =
        build_managed_hooks_claude(claude_def).map_err(ConfigInvalid)?;
    let managed_signatures = collect_signatures(&managed_hooks);
    let target = settings_path.to_string_lossy().to_string();
    let target_exists = settings_path.exists();
    let mut observed = Map::new();
    observed.insert("target".to_string(), Value::String(target));
    observed.insert("target_exists".to_string(), Value::Bool(target_exists));
    observed.insert(
        "managed_hook_count".to_string(),
        Value::Number((managed_signatures.len() as i64).into()),
    );
    observed.insert("missing_signatures".to_string(), Value::Number(0.into()));
    observed.insert("removed_signatures_present".to_string(), Value::Number(0.into()));
    if !target_exists {
        return Ok(HooksObserved { observed, state: "not-materialized", error: None });
    }
    let settings = match load_settings(&settings_path) {
        Ok(settings) => settings,
        Err(exc) => return Ok(HooksObserved { observed, state: "drifted", error: Some(exc) }),
    };
    let hooks_value = settings.get("hooks").cloned().unwrap_or(Value::Object(Map::new()));
    let present = collect_present_signatures(&hooks_value);
    let missing = managed_signatures
        .iter()
        .filter(|signature| !present.contains(signature))
        .count();
    let removed_present = removed
        .iter()
        .filter(|rule| removed_rule_present(rule, &present))
        .count();
    observed.insert("missing_signatures".to_string(), Value::Number((missing as i64).into()));
    observed.insert(
        "removed_signatures_present".to_string(),
        Value::Number((removed_present as i64).into()),
    );
    if missing > 0 || removed_present > 0 {
        return Ok(HooksObserved { observed, state: "drifted", error: None });
    }
    Ok(HooksObserved { observed, state: "managed-ok", error: None })
}

fn codex_status(codex_def: &Path) -> Result<(HooksObserved, Value), ConfigInvalid> {
    let (hooks_path, managed_hooks) = build_managed_hooks_codex(codex_def).map_err(ConfigInvalid)?;
    let mut expected = Map::new();
    expected.insert("hooks".to_string(), Value::Object(managed_hooks.clone()));
    let expected = Value::Object(expected);
    let target = hooks_path.to_string_lossy().to_string();
    let target_exists = hooks_path.exists();
    let managed_hook_count: i64 = managed_hooks
        .values()
        .map(|entries| entries.as_array().map_or(0, |list| list.len() as i64))
        .sum();
    let mut observed = Map::new();
    observed.insert("target".to_string(), Value::String(target));
    observed.insert("target_exists".to_string(), Value::Bool(target_exists));
    observed.insert(
        "managed_hook_count".to_string(),
        Value::Number(managed_hook_count.into()),
    );
    if !target_exists {
        return Ok((HooksObserved { observed, state: "not-materialized", error: None }, expected));
    }
    let actual = match load_settings(&hooks_path) {
        Ok(value) => Value::Object(value),
        Err(exc) => {
            return Ok((HooksObserved { observed, state: "drifted", error: Some(exc) }, expected));
        }
    };
    if !json_equal(&actual, &expected) {
        return Ok((HooksObserved { observed, state: "drifted", error: None }, expected));
    }
    Ok((HooksObserved { observed, state: "managed-ok", error: None }, expected))
}

fn make_hooks_element(id: &str, merge_kind: &str, observed: Map<String, Value>, state: &str) -> Value {
    let mut spec = Map::new();
    spec.insert(
        "config".to_string(),
        Value::String(format!("config/{}-hooks.json", id.strip_prefix("hooks:").unwrap_or(id))),
    );
    spec.insert("merge".to_string(), Value::String(merge_kind.to_string()));
    let mut policy = Map::new();
    policy.insert("enabled".to_string(), Value::Bool(true));
    let mut element = Map::new();
    element.insert("id".to_string(), Value::String(id.to_string()));
    element.insert("domain".to_string(), Value::String("hooks".to_string()));
    element.insert("spec".to_string(), Value::Object(spec));
    element.insert("policy".to_string(), Value::Object(policy));
    element.insert("observed".to_string(), Value::Object(observed));
    element.insert("state".to_string(), Value::String(state.to_string()));
    element.insert("freshness".to_string(), crate::mcp::freshness_not_applicable());
    element.insert(
        "actions".to_string(),
        Value::Array(vec![serde_json::from_str(HOOKS_APPLY_ACTION).unwrap()]),
    );
    Value::Object(element)
}

/// build_hooks_status port。
pub fn build_hooks_status(
    _root_dir: &Path,
    _config_file: &Path,
    def_files: &[PathBuf],
) -> Result<(Value, Vec<Diagnostic>, Vec<Diagnostic>), ConfigInvalid> {
    let warnings = Vec::new();
    let mut errors = Vec::new();
    let mut elements = Vec::new();

    let claude = claude_status(&def_files[0])?;
    if let Some(error) = &claude.error {
        errors.push(diagnostic(
            "HOOKS_TARGET_PARSE_ERROR",
            Some("hooks:claude".to_string()),
            format!(".claude/settings.json 解析失败: {}", error),
        ));
    }
    elements.push(make_hooks_element("hooks:claude", "signature-set", claude.observed, claude.state));

    if def_files.len() > 1 {
        let (codex, _expected) = codex_status(&def_files[1])?;
        if let Some(error) = &codex.error {
            errors.push(diagnostic(
                "HOOKS_TARGET_PARSE_ERROR",
                Some("hooks:codex".to_string()),
                format!(".codex/hooks.json 解析失败: {}", error),
            ));
        }
        elements.push(make_hooks_element("hooks:codex", "whole-file", codex.observed, codex.state));
    }

    let mut data = Map::new();
    data.insert("domain".to_string(), Value::String("hooks".to_string()));
    data.insert("elements".to_string(), Value::Array(elements));
    Ok((Value::Object(data), warnings, errors))
}

/// build_hooks_plan port。
pub fn build_hooks_plan(
    _root_dir: &Path,
    _config_file: &Path,
    def_files: &[PathBuf],
) -> Result<(Value, Vec<Diagnostic>, Vec<Diagnostic>), ConfigInvalid> {
    let (status, warnings, errors) = build_hooks_status(_root_dir, _config_file, def_files)?;
    let mut actions = Vec::new();
    let elements = status
        .get("elements")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for element in &elements {
        let state = element.get("state").and_then(Value::as_str).unwrap_or("");
        if !matches!(state, "drifted" | "missing" | "not-materialized") {
            continue;
        }
        let element_id = element.get("id").and_then(Value::as_str).unwrap_or("");
        let target = element
            .get("observed")
            .and_then(|o| o.get("target"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let target_exists = element
            .get("observed")
            .and_then(|o| o.get("target_exists"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let before = if target_exists {
            crate::canon::sha256_file(Path::new(&target))
        } else {
            None
        };
        let (after, note) = if element_id == "hooks:claude" {
            let settings = if target_exists {
                crate::mcp::load_json_object(Path::new(&target))
                    .map(Value::Object)
                    .unwrap_or(Value::Object(Map::new()))
            } else {
                Value::Object(Map::new())
            };
            let (_path, managed_hooks, removed) =
                build_managed_hooks_claude(&def_files[0]).map_err(ConfigInvalid)?;
            (
                rendered_json_digest(&claude_expected_render(&settings, &managed_hooks, &removed)),
                "按签名集合合并托管 hooks（保留区块外用户配置）".to_string(),
            )
        } else {
            let (_observed, expected) = codex_status(&def_files[1])?;
            (
                rendered_json_digest(&expected),
                "整体重写 codex hooks.json".to_string(),
            )
        };
        actions.push(crate::canon::plan_action(
            "write-file",
            element_id,
            Value::String(target),
            "write-local",
            before,
            Some(after),
            None,
            Some(note),
        ));
    }
    let summary = crate::canon::plan_summary(&actions);
    let mut data = Map::new();
    data.insert("domain".to_string(), Value::String("hooks".to_string()));
    data.insert("actions".to_string(), Value::Array(actions));
    data.insert("summary".to_string(), summary);
    Ok((Value::Object(data), warnings, errors))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn signature_collect_and_strip_roundtrip() {
        let managed = json!({"SessionStart": [{"hooks": [{"type": "command", "command": "echo a"}]}]});
        let managed_map = managed.as_object().unwrap().clone();
        let signatures = collect_signatures(&managed_map);
        assert_eq!(signatures, vec![("SessionStart".to_string(), None, "echo a".to_string())]);

        // 用户已有别的 hook：剥离托管后保留，合并后双在。
        let existing = json!({"SessionStart": [
            {"hooks": [{"type": "command", "command": "echo user"}]},
            {"hooks": [{"type": "command", "command": "echo a"}]},
        ]});
        let stripped = strip_managed_hooks(&existing, &signatures);
        let rendered = claude_expected_render(&json!({"hooks": stripped}), &managed_map, &[]);
        let rendered_sigs = collect_present_signatures(rendered.get("hooks").unwrap());
        assert!(rendered_sigs.contains(&("SessionStart".to_string(), None, "echo user".to_string())));
        assert!(rendered_sigs.contains(&("SessionStart".to_string(), None, "echo a".to_string())));
    }

    #[test]
    fn removed_signatures_strip() {
        let removed = vec![(
            "PreToolUse".to_string(),
            None,
            None,
            Some("old-hook".to_string()),
        )];
        let existing = json!({"PreToolUse": [{"hooks": [
            {"type": "command", "command": "sh old-hook.sh"},
            {"type": "command", "command": "sh keep.sh"},
        ]}]});
        let stripped = strip_removed_signatures(&existing, &removed);
        let sigs = collect_present_signatures(&stripped);
        assert_eq!(sigs, vec![("PreToolUse".to_string(), None, "sh keep.sh".to_string())]);
    }

    #[test]
    fn rendered_digest_matches_python_atomic_write() {
        // python3: hashlib.sha256((json.dumps({"hooks":{}}, indent=2, ensure_ascii=False)+"\n").encode()).hexdigest()
        let payload = json!({"hooks": {}});
        assert_eq!(
            rendered_json_digest(&payload),
            "sha256:78922a784ee78e9e50587e93628cd3b9d4dfbe49087adc4514e6781cea38cbb9"
        );
    }

    #[test]
    fn hook_entry_validation_fail_close() {
        assert!(validate_hook_entry(&json!({"hooks": []}), "x").is_err());
        assert!(validate_hook_entry(&json!({"hooks": [{"type": "other", "command": "x"}]}), "x").is_err());
        assert!(validate_hook_entry(&json!({"hooks": [{"type": "command", "command": " x "}]}), "x").is_ok());
    }
}
