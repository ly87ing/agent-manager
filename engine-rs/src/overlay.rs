//! Definition overlay layer (src/policy_layers.py port): policy layers carry
//! definition fragments — mcp.definitions.<name> / skills.repositories.<name> —
//! deep-merged over the committed definition files; `_delete: true` tombstones.
//! Messages are byte-identical to the Python implementation (parity contract).

use serde_json::{Map, Value};

struct OverlayRule {
    overlay_key: &'static str,
    required_key: &'static str,
    known_keys: &'static [&'static str],
}

fn rule_for(domain: &str) -> OverlayRule {
    match domain {
        "agents" => OverlayRule {
            overlay_key: "agent_definitions",
            required_key: "bin",
            known_keys: &["display_name", "bin", "app_path", "preferred_provider", "require_managed_provider", "providers", "_delete"],
        },
        "mcp" => OverlayRule {
            overlay_key: "definitions",
            required_key: "command",
            known_keys: &[
                "command",
                "args",
                "env",
                "agents",
                "extra_args_by_agent",
                "startup_timeout_sec",
                "project_serena_folder_location",
                "url",
                "_delete",
            ],
        },
        "skills" => OverlayRule {
            overlay_key: "repositories",
            required_key: "url",
            known_keys: &["url", "skill_paths", "skill_count", "agents", "new_skill_default", "_delete"],
        },
        _ => panic!("unknown overlay domain: {}", domain),
    }
}

fn deep_merge(dst: &mut Map<String, Value>, src: &Map<String, Value>) {
    for (key, value) in src {
        let mergeable = matches!(dst.get(key), Some(Value::Object(_))) && value.is_object();
        if mergeable {
            if let (Some(Value::Object(dst_child)), Value::Object(src_child)) = (dst.get_mut(key), value) {
                deep_merge(dst_child, src_child);
            }
        } else {
            dst.insert(key.clone(), value.clone());
        }
    }
}

/// overlay_definitions port：合并后策略 config.<domain>.<overlay_key>，缺省 {}。
pub fn overlay_definitions(config: &Map<String, Value>, domain: &str) -> Map<String, Value> {
    let rule = rule_for(domain);
    if domain == "agents" {
        return config.get(rule.overlay_key).and_then(Value::as_object).cloned().unwrap_or_default();
    }
    config
        .get(domain)
        .and_then(Value::as_object)
        .and_then(|section| section.get(rule.overlay_key))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default()
}

/// apply_definition_overlays port：返回 (merged, warnings)；fail-close 于非法形状。
pub fn apply_definition_overlays(
    base_defs: &Map<String, Value>,
    overlay_defs: &Map<String, Value>,
    domain: &str,
) -> Result<(Map<String, Value>, Vec<String>), String> {
    let rule = rule_for(domain);
    let mut warnings = Vec::new();
    let mut merged = base_defs.clone();
    for (name, fragment) in overlay_defs {
        let fragment = match fragment.as_object() {
            Some(map) => map,
            None => return Err(format!("{} 覆盖层片段 {} 必须是对象", domain, name)),
        };
        if fragment.get("_delete").and_then(Value::as_bool) == Some(true) {
            merged.remove(name);
            continue;
        }
        let unknown: Vec<&String> = fragment
            .keys()
            .filter(|key| key.as_str() != "_replace" && !rule.known_keys.contains(&key.as_str()))
            .collect();
        if !unknown.is_empty() {
            warnings.push(format!(
                "{} 覆盖层片段 {} 含未知键（忽略生效语义之外的影响）: {}",
                domain,
                name,
                unknown.iter().map(|k| k.as_str()).collect::<Vec<_>>().join(", ")
            ));
        }
        if fragment.get("_replace").and_then(Value::as_bool) == Some(true) {
            merged.remove(name);
        }
        let fragment: Map<String, Value> = fragment
            .iter()
            .filter(|(key, _)| key.as_str() != "_delete" && key.as_str() != "_replace")
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        match merged.get_mut(name) {
            Some(Value::Object(existing)) => deep_merge(existing, &fragment),
            _ => {
                merged.insert(name.clone(), Value::Object(fragment));
            }
        }
    }
    for (name, definition) in &merged {
        let definition = match definition.as_object() {
            Some(map) => map,
            None => return Err(format!("{} 定义 {} 必须是对象", domain, name)),
        };
        if domain == "agents" && !overlay_defs.contains_key(name) {
            continue;
        }
        let valid = definition
            .get(rule.required_key)
            .and_then(Value::as_str)
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false);
        if !valid {
            return Err(format!(
                "{} 定义 {} 缺少合法必填键 {}（非空字符串；新增/覆盖后生效集合必须完整）",
                domain, name, rule.required_key
            ));
        }
    }
    Ok((merged, warnings))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn overlay_add_edit_tombstone() {
        let base = json!({
            "alpha": {"command": "alpha-cmd", "args": ["--x"]},
            "gamma": {"command": "gamma-cmd"},
        });
        let overlay = json!({
            "alpha": {"args": ["--y"]},
            "gamma": {"_delete": true},
            "beta": {"command": "beta-cmd"},
        });
        let (merged, warnings) = apply_definition_overlays(
            base.as_object().unwrap(),
            overlay.as_object().unwrap(),
            "mcp",
        )
        .unwrap();
        assert_eq!(merged["alpha"]["args"], json!(["--y"]));
        assert_eq!(merged["alpha"]["command"], json!("alpha-cmd"));
        assert!(!merged.contains_key("gamma"));
        assert_eq!(merged["beta"]["command"], json!("beta-cmd"));
        assert!(warnings.is_empty());
    }

    #[test]
    fn overlay_fail_close_on_missing_required_key() {
        let overlay = json!({"new-server": {"args": []}});
        let result = apply_definition_overlays(
            &Map::new(),
            overlay.as_object().unwrap(),
            "mcp",
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("command"));
    }

    #[test]
    fn overlay_unknown_keys_warn_not_reject() {
        let overlay = json!({"s": {"command": "x", "bogus": 1}});
        let (_merged, warnings) = apply_definition_overlays(
            &Map::new(),
            overlay.as_object().unwrap(),
            "mcp",
        )
        .unwrap();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("bogus"));
    }
}
