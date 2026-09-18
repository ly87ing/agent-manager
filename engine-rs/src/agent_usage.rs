//! 从助手自己写在本机的文件里读额度，不联网、不调 API、不需要额外登录。
//!
//! 每家助手把自己的额度状态写在不同地方，格式也不一样：
//! - Claude Code：`~/.claude.json` 的 `cachedUsageUtilization`，含 5 小时 / 7 天窗口的
//!   已用百分比与重置时刻，以及这份缓存自身的抓取时刻。
//! - Codex：`~/.codex/sessions/<年>/<月>/<日>/rollout-*.jsonl`，每条 token_count 事件带
//!   `rate_limits`；取最新会话文件里最后一条。
//!
//! 数字只和助手自己的记账一样新：它写的时候才更新，所以一律带上 `checked_at`，
//! 由调用方决定怎么提示新鲜度。别的助手本机没有这类记录，返回 None。
//!
//! 全程只读，不创建目录、不写文件。

use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use chrono::{DateTime, SecondsFormat, TimeZone, Utc};
use serde_json::{json, Map, Value};

/// 从会话文件尾部往回找额度时最多读这么多字节。最新一条 `rate_limits` 总在文件末尾附近，
/// 而单个会话文件可能上百 MB，整读会把只读的 status 拖慢。
const TAIL_SCAN_BYTES: u64 = 1 << 20;

/// 读某个助手的额度。没有本地记录时返回 None。
pub(crate) fn read(home_dir: &Path, agent_id: &str) -> Option<Value> {
    match agent_id {
        "claude-code" => claude(home_dir),
        "codex" => codex(home_dir),
        _ => None,
    }
}

fn usage(source: &str, checked_at: Option<String>, windows: Vec<Value>, extra: Map<String, Value>) -> Option<Value> {
    if windows.is_empty() {
        return None;
    }
    let mut out = Map::new();
    out.insert("source".into(), json!(source));
    out.insert("checked_at".into(), checked_at.map(Value::String).unwrap_or(Value::Null));
    out.insert("windows".into(), Value::Array(windows));
    for (key, value) in extra {
        out.insert(key, value);
    }
    Some(Value::Object(out))
}

fn window(minutes: i64, used_percent: f64, resets_at: Option<String>) -> Value {
    json!({
        "window_minutes": minutes,
        "used_percent": used_percent,
        "resets_at": resets_at.map(Value::String).unwrap_or(Value::Null),
    })
}

fn rfc3339_secs(dt: DateTime<Utc>) -> String {
    dt.to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// 规整成秒级 UTC：这些字段各家写法不一（带微秒、带偏移），统一后界面才好比对。
fn normalize_time(raw: &str) -> Option<String> {
    DateTime::parse_from_rfc3339(raw).ok().map(|dt| rfc3339_secs(dt.with_timezone(&Utc)))
}

fn from_unix_secs(secs: i64) -> Option<String> {
    Utc.timestamp_opt(secs, 0).single().map(rfc3339_secs)
}

// ---------- Claude Code ----------

/// Claude Code 自己缓存的额度：`utilization` 是已用百分比，窗口名固定为
/// five_hour / seven_day，其余窗口（按套餐才有）为 null，跳过。
fn claude(home_dir: &Path) -> Option<Value> {
    let path = home_dir.join(".claude.json");
    let root: Value = serde_json::from_str(&fs::read_to_string(&path).ok()?).ok()?;
    let cached = root.get("cachedUsageUtilization")?;
    let checked_at = cached
        .get("fetchedAtMs")
        .and_then(Value::as_i64)
        .and_then(|ms| Utc.timestamp_millis_opt(ms).single())
        .map(rfc3339_secs);

    let mut windows = Vec::new();
    for (key, minutes) in [("five_hour", 5 * 60), ("seven_day", 7 * 24 * 60)] {
        let Some(entry) = cached.get("utilization").and_then(|u| u.get(key)).filter(|v| !v.is_null()) else {
            continue;
        };
        let Some(used) = entry.get("utilization").and_then(Value::as_f64) else {
            continue;
        };
        let resets_at = entry.get("resets_at").and_then(Value::as_str).and_then(normalize_time);
        windows.push(window(minutes, used, resets_at));
    }

    usage(".claude.json", checked_at, windows, Map::new())
}

// ---------- Codex ----------

/// Codex 把额度写在会话记录里，取最新那次会话的最后一条。
fn codex(home_dir: &Path) -> Option<Value> {
    let path = newest_rollout(&home_dir.join(".codex").join("sessions"))?;
    let line = last_line_with(&path, "\"rate_limits\"")?;
    let event: Value = serde_json::from_str(&line).ok()?;
    let limits = find_key(&event, "rate_limits")?;

    let mut windows = Vec::new();
    for key in ["primary", "secondary"] {
        let Some(entry) = limits.get(key).filter(|v| !v.is_null()) else {
            continue;
        };
        let Some(used) = entry.get("used_percent").and_then(Value::as_f64) else {
            continue;
        };
        let minutes = entry.get("window_minutes").and_then(Value::as_i64).unwrap_or(0);
        let resets_at = entry.get("resets_at").and_then(Value::as_i64).and_then(from_unix_secs);
        windows.push(window(minutes, used, resets_at));
    }

    // 会话文件的修改时刻就是 Codex 最后一次记账的时刻
    let checked_at = fs::metadata(&path)
        .and_then(|meta| meta.modified())
        .ok()
        .map(|time| rfc3339_secs(DateTime::<Utc>::from(time)));

    let mut extra = Map::new();
    if let Some(plan) = limits.get("plan_type").and_then(Value::as_str) {
        extra.insert("plan".into(), json!(plan));
    }
    if let Some(balance) = limits.get("credits").and_then(|c| c.get("balance")).and_then(Value::as_str) {
        extra.insert("credits".into(), json!(balance));
    }

    usage(".codex/sessions", checked_at, windows, extra)
}

/// 会话目录按 年/月/日 分层，文件数不多，直接按修改时刻取最新。
fn newest_rollout(sessions_dir: &Path) -> Option<PathBuf> {
    let mut newest: Option<(std::time::SystemTime, PathBuf)> = None;
    let mut stack = vec![sessions_dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                stack.push(path);
                continue;
            }
            let is_rollout = path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("rollout-") && name.ends_with(".jsonl"));
            if !is_rollout {
                continue;
            }
            let Ok(modified) = meta.modified() else { continue };
            if newest.as_ref().is_none_or(|(best, _)| modified > *best) {
                newest = Some((modified, path));
            }
        }
    }
    newest.map(|(_, path)| path)
}

/// 从文件尾部读一段，返回其中最后一条包含 `needle` 的完整行。
fn last_line_with(path: &Path, needle: &str) -> Option<String> {
    let mut file = fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let start = len.saturating_sub(TAIL_SCAN_BYTES);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf);
    // 从中间切入时首行多半是残句，跳过；从头读则全部可用
    let mut lines = text.lines();
    if start > 0 {
        lines.next();
    }
    lines.filter(|line| line.contains(needle)).next_back().map(str::to_string)
}

/// `rate_limits` 在事件里的层级各版本不同，按键名递归找第一处。
fn find_key<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    match value {
        Value::Object(map) => {
            if let Some(found) = map.get(key) {
                return Some(found);
            }
            map.values().find_map(|child| find_key(child, key))
        }
        Value::Array(items) => items.iter().find_map(|item| find_key(item, key)),
        _ => None,
    }
}
