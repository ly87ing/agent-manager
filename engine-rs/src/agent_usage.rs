//! 从助手自己写在本机的文件里读额度，不联网、不调 API、不需要额外登录。
//!
//! 每家助手把自己的额度状态写在不同地方，格式也不一样：
//! - Claude Code：两个来源取较新的那个。
//!   1. `<配置目录>/.agent-manager-usage.json`：状态栏脚本顺手存下的实时额度。
//!      Claude Code 每次渲染状态栏都把 `rate_limits` 推给脚本，但自己不写文件，
//!      外部进程读不到；脚本存一份，这里就能拿到与它同屏显示的那个数。
//!   2. `~/.claude.json` 的 `cachedUsageUtilization`：Claude Code 主动查额度时才刷新的
//!      缓存，常规对话不更新，可能停在很多天前。没装落盘时只有它。
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
///
/// `bin` 决定按哪家的格式读，`config_path` 是这个助手定义里写的配置文件位置
/// （claude 的 `.claude.json` / codex 的 `config.toml`），没写就回落到默认位置。
pub(crate) fn read(home_dir: &Path, bin: &str, config_path: Option<&str>) -> Option<Value> {
    let config = config_path.map(|raw| crate::native_mcp::targets::expand_home(raw, home_dir));
    match binary_kind(bin)? {
        Kind::Claude => {
            let path = config.unwrap_or_else(|| home_dir.join(".claude.json"));
            // 主助手的 .claude.json 在 home 根，配置目录却是 ~/.claude，落盘在后者
            claude(&path, &home_dir.join(".claude"))
        }
        Kind::Codex => {
            // 配置文件是 <CODEX_HOME>/config.toml，会话就在它旁边
            let root = config
                .as_deref()
                .and_then(Path::parent)
                .map(Path::to_path_buf)
                .unwrap_or_else(|| home_dir.join(".codex"));
            codex(&root.join("sessions"))
        }
    }
}

/// 实例（profile）的额度：配置目录由实例自己定义，直接按种类去那里读。
///
/// 实例存在的意义就是多开不同账号，额度自然各算各的——用默认位置那份会张冠李戴。
pub(crate) fn read_profile(kind: &str, config_dir: &Path) -> Option<Value> {
    match kind {
        // 实例的配置目录就是 CLAUDE_CONFIG_DIR，落盘与 .claude.json 同级
        "claude" => claude(&config_dir.join(".claude.json"), config_dir),
        "codex" => codex(&config_dir.join("sessions")),
        _ => None,
    }
}

enum Kind {
    Claude,
    Codex,
}

/// 只认可执行程序名，路径和 .exe 后缀都剥掉：自定义助手可能把 bin 写成绝对路径。
fn binary_kind(bin: &str) -> Option<Kind> {
    let name = Path::new(bin).file_stem()?.to_str()?;
    match name {
        "claude" => Some(Kind::Claude),
        "codex" => Some(Kind::Codex),
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

/// 数字是从哪个文件读来的，直接给完整路径：实例各有各的配置目录，只说「.claude.json」
/// 分不清是哪一个。
fn display(path: &Path) -> String {
    path.display().to_string()
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

/// 两个来源取较新的那个：状态栏落盘的实时快照，或 Claude Code 自己的额度缓存。
///
/// 没装落盘时只有后者，行为与从前一致；装了就以它为准——除非缓存反而更新
/// （比如刚在 Claude Code 里查过额度，而状态栏很久没刷）。
fn claude(config_path: &Path, snapshot_dir: &Path) -> Option<Value> {
    let snapshot = statusline_snapshot(snapshot_dir);
    let cached = claude_cache(config_path);
    match (snapshot, cached) {
        (Some(a), Some(b)) => Some(if checked_at_of(&a) >= checked_at_of(&b) { a } else { b }),
        (Some(only), None) | (None, Some(only)) => Some(only),
        (None, None) => None,
    }
}

/// 记账时刻，用于在两个来源间挑较新的那份；缺时刻的按最旧处理。
fn checked_at_of(usage: &Value) -> String {
    usage
        .get("checked_at")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

/// 状态栏脚本存下的实时额度。字段沿用 Claude Code 推给状态栏的原名
/// （`used_percentage` 与 Unix 秒的 `resets_at`），与 `.claude.json` 里那套不同。
fn statusline_snapshot(dir: &Path) -> Option<Value> {
    let path = dir.join(".agent-manager-usage.json");
    let root: Value = serde_json::from_str(&fs::read_to_string(&path).ok()?).ok()?;
    let checked_at = root.get("at").and_then(Value::as_i64).and_then(from_unix_secs);

    let mut windows = Vec::new();
    for (key, minutes) in [("five_hour", 5 * 60), ("seven_day", 7 * 24 * 60)] {
        let Some(entry) = root.get(key).filter(|v| !v.is_null()) else {
            continue;
        };
        let Some(used) = entry.get("used_percentage").and_then(Value::as_f64) else {
            continue;
        };
        let resets_at = entry.get("resets_at").and_then(Value::as_i64).and_then(from_unix_secs);
        windows.push(window(minutes, used, resets_at));
    }

    usage(&display(&path), checked_at, windows, Map::new())
}

/// Claude Code 自己缓存的额度：`utilization` 是已用百分比，窗口名固定为
/// five_hour / seven_day，其余窗口（按套餐才有）为 null，跳过。
fn claude_cache(path: &Path) -> Option<Value> {
    let root: Value = serde_json::from_str(&fs::read_to_string(path).ok()?).ok()?;
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

    usage(&display(path), checked_at, windows, Map::new())
}

// ---------- Codex ----------

/// Codex 把额度写在会话记录里，取最新那次会话的最后一条。
fn codex(sessions_dir: &Path) -> Option<Value> {
    let path = newest_rollout(sessions_dir)?;
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

    // 记账时刻取这条事件自己带的 timestamp：文件修改时刻会被后续别的事件顶新，
    // 那样显示出来的「几时更新」比这份额度的实际采样时刻新，等于谎报新鲜度。
    let checked_at = event
        .get("timestamp")
        .and_then(Value::as_str)
        .and_then(normalize_time)
        .or_else(|| {
            fs::metadata(&path)
                .and_then(|meta| meta.modified())
                .ok()
                .map(|time| rfc3339_secs(DateTime::<Utc>::from(time)))
        });

    let mut extra = Map::new();
    if let Some(plan) = limits.get("plan_type").and_then(Value::as_str) {
        extra.insert("plan".into(), json!(plan));
    }
    if let Some(balance) = limits.get("credits").and_then(|c| c.get("balance")).and_then(Value::as_str) {
        extra.insert("credits".into(), json!(balance));
    }

    usage(&display(sessions_dir), checked_at, windows, extra)
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
