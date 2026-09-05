//! Upstream probe cache reader (src/upstream_cache.py port, read-only side)
//! plus the local (zero-network) helpers status uses: env-file loading and the
//! npx cache scan.

use serde_json::{Map, Value};
use std::path::PathBuf;

pub const DEFAULT_TTL_SEC: i64 = 24 * 3600;

pub fn cache_path() -> PathBuf {
    if let Ok(configured) = std::env::var("AGENT_MANAGER_UPSTREAM_CACHE") {
        if !configured.is_empty() {
            return PathBuf::from(configured);
        }
    }
    let tmp = std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".to_string());
    let user = std::env::var("USER").unwrap_or_else(|_| "unknown".to_string());
    PathBuf::from(tmp).join(format!("agent_manager_upstream_{}.json", user))
}

pub fn ttl_sec() -> i64 {
    if let Ok(raw) = std::env::var("AGENT_MANAGER_UPSTREAM_TTL_SEC") {
        if let Ok(parsed) = raw.trim().parse::<i64>() {
            return parsed.max(0);
        }
    }
    DEFAULT_TTL_SEC
}

/// load_cache: missing/corrupt file yields an empty entries map.
pub fn load_cache() -> Map<String, Value> {
    let path = cache_path();
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(_) => return Map::new(),
    };
    let parsed: Value = match serde_json::from_str(&text) {
        Ok(value) => value,
        Err(_) => return Map::new(),
    };
    parsed
        .get("entries")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default()
}

pub fn parse_iso(value: &str) -> Option<i64> {
    let naive =
        chrono::NaiveDateTime::parse_from_str(value.trim(), "%Y-%m-%dT%H:%M:%SZ").ok()?;
    Some(naive.and_utc().timestamp())
}

pub fn cache_age_sec(entry: &Value, now_epoch: i64) -> Option<i64> {
    let checked = entry.get("checked_at")?.as_str().and_then(parse_iso)?;
    Some((now_epoch - checked).max(0))
}

pub fn is_stale(entry: &Value, now_epoch: i64) -> bool {
    match cache_age_sec(entry, now_epoch) {
        Some(age) => age > ttl_sec(),
        None => true,
    }
}

/// load_env_files (agent-mcp/sync_mcp.py port): KEY=VALUE lines, '#' comments,
/// surrounding quotes stripped. Values never leave this module's callers.
pub fn load_env_files(paths: &[PathBuf]) -> Map<String, Value> {
    let mut vars = Map::new();
    for path in paths {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(_) => continue,
        };
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((key, value)) = line.split_once('=') {
                let value = value
                    .trim()
                    .trim_matches('"')
                    .trim_matches('\'')
                    .to_string();
                vars.insert(key.trim().to_string(), Value::String(value));
            }
        }
    }
    vars
}

/// Best-effort read of the npx cache (~/.npm/_npx); None when absent.
pub fn npx_cached_version(package: &str) -> Option<String> {
    let home = std::env::var("HOME").ok()?;
    let base = PathBuf::from(home).join(".npm").join("_npx");
    let mut entries: Vec<_> = std::fs::read_dir(base).ok()?.filter_map(|e| e.ok()).collect();
    entries.sort_by_key(|e| e.file_name());
    let mut rel = PathBuf::from("node_modules");
    for segment in package.split('/') {
        rel = rel.join(segment);
    }
    rel = rel.join("package.json");
    for entry in entries {
        let candidate = entry.path().join(&rel);
        if let Ok(text) = std::fs::read_to_string(candidate) {
            if let Ok(Value::Object(map)) = serde_json::from_str::<Value>(&text) {
                if let Some(version) = map.get("version").and_then(Value::as_str) {
                    if !version.trim().is_empty() {
                        return Some(version.trim().to_string());
                    }
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_iso_and_age() {
        let epoch = parse_iso("2026-09-05T10:30:00Z").unwrap();
        assert!(epoch > 0);
        let entry = serde_json::json!({"checked_at": "2026-09-05T10:30:00Z"});
        assert_eq!(cache_age_sec(&entry, epoch + 90), Some(90));
        assert_eq!(cache_age_sec(&serde_json::json!({}), epoch), None);
    }

    #[test]
    fn env_files_strip_quotes_and_skip_comments() {
        let dir = std::env::temp_dir().join(format!("engine_rs_env_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("mcp.env");
        std::fs::write(&file, "# comment\nA=1\nB=\"two\"\nC='three'\n").unwrap();
        let vars = load_env_files(&[file]);
        assert_eq!(vars["A"], Value::String("1".to_string()));
        assert_eq!(vars["B"], Value::String("two".to_string()));
        assert_eq!(vars["C"], Value::String("three".to_string()));
        std::fs::remove_dir_all(&dir).ok();
    }
}
