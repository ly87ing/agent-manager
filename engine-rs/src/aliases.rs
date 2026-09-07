//! aliases domain kernels — port of build_aliases_status / build_aliases_plan
//! from src/engine_v1.py. The managed marker block in ~/.zshrc / ~/.zprofile is
//! the only managed surface; content outside the markers is ignored.

use crate::canon::sha256_hex;
use crate::envelope::Diagnostic;
use crate::mcp::ConfigInvalid;
use serde_json::{Map, Value};
use std::path::PathBuf;

const ALIAS_APPLY_ACTION: &str =
    "{\"id\": \"apply\", \"risk\": \"write-local\", \"needs_lock\": true, \"offline_ok\": true}";

pub const ALIAS_MARKER_BEGIN: &str = "# >>> agent-manager shell aliases >>>";
pub const ALIAS_MARKER_END: &str = "# <<< agent-manager shell aliases <<<";

/// _render_alias_lines port（sync_shell_aliases.sh 渲染步骤复刻）：
/// sorted 按名、"alias <name>='<value>'"、{timezone}/{lang} 占位替换。
pub fn render_alias_lines(cfg: &Map<String, Value>) -> Vec<String> {
    let timezone = cfg
        .get("timezone")
        .and_then(Value::as_str)
        .unwrap_or("America/Los_Angeles");
    let lang = cfg.get("lang").and_then(Value::as_str).unwrap_or("en_US.UTF-8");
    let aliases = cfg.get("aliases").and_then(Value::as_object).cloned().unwrap_or_default();
    let mut pairs: Vec<(&String, &Value)> = aliases.iter().collect();
    pairs.sort_by(|a, b| a.0.cmp(b.0));
    pairs
        .into_iter()
        .map(|(name, value)| {
            let value = value.as_str().unwrap_or("");
            let value = value.replace("{timezone}", timezone).replace("{lang}", lang);
            format!("alias {}='{}'", name, value)
        })
        .collect()
}

/// _extract_alias_block port：(has_block, inner_text)；区块外内容忽略。
pub fn extract_alias_block(text: &str) -> (bool, String) {
    let mut inside = false;
    let mut found = false;
    let mut inner = Vec::new();
    for line in text.lines() {
        if line == ALIAS_MARKER_BEGIN {
            inside = true;
            found = true;
            continue;
        }
        if line == ALIAS_MARKER_END {
            inside = false;
            continue;
        }
        if inside {
            inner.push(line);
        }
    }
    (found, inner.join("\n"))
}

fn sha256_text(text: &str) -> String {
    format!("sha256:{}", sha256_hex(text.as_bytes()))
}

fn make_alias_element(rc_name: &str, cfg: &Map<String, Value>, observed: Map<String, Value>, state: &str) -> Value {
    let aliases = cfg.get("aliases").and_then(Value::as_object).cloned().unwrap_or_default();
    let mut names: Vec<String> = aliases.keys().cloned().collect();
    names.sort();
    let mut spec = Map::new();
    spec.insert(
        "aliases".to_string(),
        Value::Array(names.into_iter().map(Value::String).collect()),
    );
    spec.insert(
        "marker".to_string(),
        Value::String("agent-manager shell aliases".to_string()),
    );
    let mut policy = Map::new();
    policy.insert("enabled".to_string(), Value::Bool(true));
    let mut element = Map::new();
    element.insert("id".to_string(), Value::String(format!("aliases:{}", rc_name)));
    element.insert("domain".to_string(), Value::String("aliases".to_string()));
    element.insert("spec".to_string(), Value::Object(spec));
    element.insert("policy".to_string(), Value::Object(policy));
    element.insert("observed".to_string(), Value::Object(observed));
    element.insert("state".to_string(), Value::String(state.to_string()));
    element.insert("freshness".to_string(), crate::mcp::freshness_not_applicable());
    element.insert(
        "actions".to_string(),
        Value::Array(vec![serde_json::from_str(ALIAS_APPLY_ACTION).unwrap()]),
    );
    Value::Object(element)
}

/// build_aliases_status port。
pub fn build_aliases_status(
    _root_dir: &std::path::Path,
    _config_file: &std::path::Path,
    def_files: &[PathBuf],
) -> Result<(Value, Vec<Diagnostic>, Vec<Diagnostic>), ConfigInvalid> {
    let warnings = Vec::new();
    let errors = Vec::new();
    let cfg = crate::mcp::load_json_object(&def_files[0])?;
    let alias_lines = render_alias_lines(&cfg);
    let expected_inner = alias_lines.join("\n");

    let home = std::env::var("HOME").unwrap_or_default();
    let mut elements = Vec::new();
    for (rc_name, rc_path) in [
        ("zshrc", format!("{}/.zshrc", home)),
        ("zprofile", format!("{}/.zprofile", home)),
    ] {
        let target_exists = std::path::Path::new(&rc_path).exists();
        let (has_block, matches) = if target_exists {
            let text = std::fs::read_to_string(&rc_path).unwrap_or_default();
            let (found, inner) = extract_alias_block(&text);
            (found, found && inner == expected_inner)
        } else {
            (false, false)
        };
        let state = if !target_exists {
            if alias_lines.is_empty() { "managed-ok" } else { "not-materialized" }
        } else if matches {
            "managed-ok"
        } else if has_block {
            "drifted"
        } else if alias_lines.is_empty() {
            "managed-ok"
        } else {
            "missing"
        };
        let mut observed = Map::new();
        observed.insert("target".to_string(), Value::String(rc_path));
        observed.insert("target_exists".to_string(), Value::Bool(target_exists));
        observed.insert("block_present".to_string(), Value::Bool(has_block));
        observed.insert(
            "alias_count".to_string(),
            Value::Number((alias_lines.len() as i64).into()),
        );
        elements.push(make_alias_element(rc_name, &cfg, observed, state));
    }
    let mut data = Map::new();
    data.insert("domain".to_string(), Value::String("aliases".to_string()));
    data.insert("elements".to_string(), Value::Array(elements));
    Ok((Value::Object(data), warnings, errors))
}

/// build_aliases_plan port（区块级 digest；marker 区块外内容不动）。
pub fn build_aliases_plan(
    root_dir: &std::path::Path,
    config_file: &std::path::Path,
    def_files: &[PathBuf],
    only: &[String],
) -> Result<(Value, Vec<Diagnostic>, Vec<Diagnostic>), ConfigInvalid> {
    let (status, warnings, errors) = build_aliases_status(root_dir, config_file, def_files)?;
    let cfg = crate::mcp::load_json_object(&def_files[0])?;
    let alias_lines = render_alias_lines(&cfg);
    let expected_inner = alias_lines.join("\n");
    let mut actions = Vec::new();
    let elements = status
        .get("elements")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if !only.is_empty() {
        for target in only {
            if !elements.iter().any(|e| e.get("id").and_then(Value::as_str) == Some(target.as_str())) {
                return Err(ConfigInvalid(format!("元素 '{}' 在域 'aliases' 中不存在", target)));
            }
        }
    }
    for element in &elements {
        let element_id = element.get("id").and_then(Value::as_str).unwrap_or("");
        if !only.is_empty() && !only.contains(&element_id.to_string()) {
            continue;
        }
        let state = element.get("state").and_then(Value::as_str).unwrap_or("");
        if !matches!(state, "drifted" | "missing" | "not-materialized") {
            continue;
        }
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
        let block_present = element
            .get("observed")
            .and_then(|o| o.get("block_present"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let before = if target_exists && block_present {
            let text = std::fs::read_to_string(&target).unwrap_or_default();
            let (_found, inner) = extract_alias_block(&text);
            Some(sha256_text(&inner))
        } else {
            None
        };
        actions.push(crate::canon::plan_action(
            "write-file",
            element_id,
            Value::String(target),
            "write-local",
            before,
            if alias_lines.is_empty() {
                None
            } else {
                Some(sha256_text(&expected_inner))
            },
            None,
            Some("受管区块 digest；marker 区块外内容不动".to_string()),
        ));
    }
    let summary = crate::canon::plan_summary(&actions);
    let mut data = Map::new();
    data.insert("domain".to_string(), Value::String("aliases".to_string()));
    data.insert("actions".to_string(), Value::Array(actions));
    data.insert("summary".to_string(), summary);
    Ok((Value::Object(data), warnings, errors))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn render_sorted_lines_with_placeholders() {
        let cfg = json!({
            "timezone": "UTC",
            "lang": "C",
            "aliases": {"ll": "ls -l", "ts": "TZ={timezone} LANG={lang} x"},
        });
        let lines = render_alias_lines(cfg.as_object().unwrap());
        assert_eq!(lines, vec!["alias ll='ls -l'", "alias ts='TZ=UTC LANG=C x'"]);
        let empty = json!({});
        assert!(render_alias_lines(empty.as_object().unwrap()).is_empty());
    }

    #[test]
    fn block_extraction_ignores_outer_content() {
        let text = "# user line\n# >>> agent-manager shell aliases >>>\nalias a='1'\n# <<< agent-manager shell aliases <<<\n# after\n";
        let (found, inner) = extract_alias_block(text);
        assert!(found);
        assert_eq!(inner, "alias a='1'");
        let (found, inner) = extract_alias_block("# nothing\n");
        assert!(!found);
        assert_eq!(inner, "");
        // 旧区块 + 新期望不一致 → 调用方判 drifted。
        let text = "# >>> agent-manager shell aliases >>>\nalias old='1'\n# <<< agent-manager shell aliases <<<\n";
        let (found, inner) = extract_alias_block(text);
        assert!(found);
        assert_eq!(inner, "alias old='1'");
    }
}
