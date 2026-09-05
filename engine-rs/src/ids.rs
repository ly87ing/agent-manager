//! Agent id canonicalization (src/agent_ids.py port: alias table + mapping merge).

use serde_json::{Map, Value};

const ALIASES: &[(&str, &str)] = &[
    ("Claude Code", "claude-code"),
    ("Codex", "codex"),
    ("Kimi Code", "kimi-code"),
];

pub fn canonicalize_agent_id(agent_id: &str) -> String {
    for (alias, canonical) in ALIASES {
        if agent_id == *alias {
            return canonical.to_string();
        }
    }
    agent_id.to_string()
}

/// canonicalize_agent_list: None stays None; every entry is canonicalized.
pub fn canonicalize_agent_list(value: &Value) -> Option<Vec<String>> {
    if value.is_null() {
        return None;
    }
    let items = value.as_array()?;
    Some(
        items
            .iter()
            .filter_map(|item| item.as_str().map(canonicalize_agent_id))
            .collect(),
    )
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

/// canonicalize_agent_mapping: alias keys canonicalized; duplicate canonical ids
/// deep-merge when both values are objects, conflict otherwise (fail-close).
pub fn canonicalize_agent_mapping(value: &Value) -> Result<Map<String, Value>, String> {
    let mut normalized = Map::new();
    let empty = Map::new();
    let mapping = match value {
        Value::Object(map) => map,
        Value::Null => &empty,
        _ => return Err("agent mapping 必须是对象".to_string()),
    };
    for (raw_key, raw_value) in mapping {
        let agent_id = canonicalize_agent_id(raw_key);
        match normalized.get(&agent_id) {
            Some(existing) => {
                if existing.is_object() && raw_value.is_object() {
                    let mut merged = existing.as_object().unwrap().clone();
                    deep_merge(&mut merged, raw_value.as_object().unwrap());
                    normalized.insert(agent_id, Value::Object(merged));
                } else if *existing != *raw_value {
                    return Err(format!("Conflicting values for agent id '{}'", agent_id));
                }
            }
            None => {
                normalized.insert(agent_id, raw_value.clone());
            }
        }
    }
    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn aliases_canonicalize() {
        assert_eq!(canonicalize_agent_id("Claude Code"), "claude-code");
        assert_eq!(canonicalize_agent_id("kimi-code"), "kimi-code");
    }

    #[test]
    fn mapping_merges_duplicate_objects() {
        let value = json!({"Claude Code": {"a": 1}, "claude-code": {"b": 2}});
        let merged = canonicalize_agent_mapping(&value).unwrap();
        assert_eq!(merged["claude-code"], json!({"a": 1, "b": 2}));
    }

    #[test]
    fn mapping_conflicts_fail_closed() {
        let value = json!({"Claude Code": 1, "claude-code": 2});
        assert!(canonicalize_agent_mapping(&value).is_err());
    }
}
