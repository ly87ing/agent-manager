//! 从助手自己写在本机的文件里读额度，不联网、不调 API、不需要额外登录。
//!
//! 每家助手把自己的额度状态写在不同地方，格式也不一样：
//! - Claude Code：两个来源取较新的那个。
//!   1. `<数据目录>/usage/<编码后的配置目录>.json`：状态栏脚本顺手存下的实时额度。
//!      Claude Code 每次渲染状态栏都把 `rate_limits` 推给脚本，但自己不写文件，
//!      外部进程读不到；脚本存一份，这里就能拿到与它同屏显示的那个数。存在本工具
//!      自己的数据目录里，不往助手的配置目录放文件；编码规则见 [`usage_key`]。
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
/// `data_dir` 是本工具的数据目录，状态栏落盘的额度在它下面。
pub(crate) fn read(
    home_dir: &Path,
    data_dir: &Path,
    bin: &str,
    config_path: Option<&str>,
) -> Option<Value> {
    let config = config_path.map(|raw| crate::native_mcp::targets::expand_home(raw, home_dir));
    match binary_kind(bin)? {
        Kind::Claude => {
            let path = config.unwrap_or_else(|| home_dir.join(".claude.json"));
            // 主助手的 .claude.json 在 home 根，配置目录却是 ~/.claude，落盘在后者
            claude(&path, &snapshot_path(data_dir, &home_dir.join(".claude")))
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
pub(crate) fn read_profile(kind: &str, config_dir: &Path, data_dir: &Path) -> Option<Value> {
    match kind {
        // 实例的配置目录就是 CLAUDE_CONFIG_DIR，落盘按它编码出文件名
        "claude" => claude(
            &config_dir.join(".claude.json"),
            &snapshot_path(data_dir, config_dir),
        ),
        "codex" => codex(&config_dir.join("sessions")),
        _ => None,
    }
}

/// 状态栏脚本存额度的位置：`<数据目录>/usage/<编码后的配置目录>.json`。
///
/// 脚本部署在 `<数据目录>/statusline.sh`，按自身所在目录定位，两边因此是同一处。
fn snapshot_path(data_dir: &Path, config_dir: &Path) -> PathBuf {
    data_dir
        .join("usage")
        .join(format!("{}.json", usage_key(&config_dir.to_string_lossy())))
}

/// 配置目录编进文件名：先 `%` -> `%25`，再 `/` -> `%2F`。与状态栏脚本逐字一致，
/// 不同目录编出来不会相同。
///
/// Windows：状态栏脚本在 Git Bash 里拿到的目录可能是 `/c/Users/u/.claude` 或 `C:\Users\u\.claude`，
/// 两边先统一成 `c:/users/u/.claude`（`\` 换 `/`、不分大小写），`:` 也转义成 `%3A`。
fn usage_key(config_dir: &str) -> String {
    let dir = if cfg!(windows) {
        config_dir.replace('\\', "/").to_lowercase()
    } else {
        config_dir.to_string()
    };
    let key = dir.replace('%', "%25").replace('/', "%2F");
    if cfg!(windows) {
        key.replace(':', "%3A")
    } else {
        key
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

fn usage(
    source: &str,
    checked_at: Option<String>,
    windows: Vec<Value>,
    extra: Map<String, Value>,
) -> Option<Value> {
    if windows.is_empty() {
        return None;
    }
    let mut out = Map::new();
    out.insert("source".into(), json!(source));
    out.insert(
        "checked_at".into(),
        checked_at.map(Value::String).unwrap_or(Value::Null),
    );
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
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|dt| rfc3339_secs(dt.with_timezone(&Utc)))
}

fn from_unix_secs(secs: i64) -> Option<String> {
    Utc.timestamp_opt(secs, 0).single().map(rfc3339_secs)
}

// ---------- Claude Code ----------

/// 两个来源取较新的那个：状态栏落盘的实时快照，或 Claude Code 自己的额度缓存。
///
/// 没装落盘时只有后者，行为与从前一致；装了就以它为准——除非缓存反而更新
/// （比如刚在 Claude Code 里查过额度，而状态栏很久没刷）。
fn claude(config_path: &Path, snapshot_path: &Path) -> Option<Value> {
    let snapshot = statusline_snapshot(snapshot_path);
    let cached = claude_cache(config_path);
    match (snapshot, cached) {
        (Some(a), Some(b)) => Some(if checked_at_of(&a) >= checked_at_of(&b) {
            a
        } else {
            b
        }),
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
fn statusline_snapshot(path: &Path) -> Option<Value> {
    let root: Value = serde_json::from_str(&fs::read_to_string(path).ok()?).ok()?;
    let checked_at = root
        .get("at")
        .and_then(Value::as_i64)
        .and_then(from_unix_secs);

    let mut windows = Vec::new();
    for (key, minutes) in [("five_hour", 5 * 60), ("seven_day", 7 * 24 * 60)] {
        let Some(entry) = root.get(key).filter(|v| !v.is_null()) else {
            continue;
        };
        let Some(used) = entry.get("used_percentage").and_then(Value::as_f64) else {
            continue;
        };
        let resets_at = entry
            .get("resets_at")
            .and_then(Value::as_i64)
            .and_then(from_unix_secs);
        windows.push(window(minutes, used, resets_at));
    }

    usage(&display(path), checked_at, windows, Map::new())
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
        let Some(entry) = cached
            .get("utilization")
            .and_then(|u| u.get(key))
            .filter(|v| !v.is_null())
        else {
            continue;
        };
        let Some(used) = entry.get("utilization").and_then(Value::as_f64) else {
            continue;
        };
        let resets_at = entry
            .get("resets_at")
            .and_then(Value::as_str)
            .and_then(normalize_time);
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
        let minutes = entry
            .get("window_minutes")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        let resets_at = entry
            .get("resets_at")
            .and_then(Value::as_i64)
            .and_then(from_unix_secs);
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
    if let Some(balance) = limits
        .get("credits")
        .and_then(|c| c.get("balance"))
        .and_then(Value::as_str)
    {
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
            let Ok(modified) = meta.modified() else {
                continue;
            };
            // 修改时间一样（时间戳精度粗的文件系统上紧挨着写的两个会话）时按文件名比：
            // 文件名里带会话开始时间（rollout-2026-09-17T09-00-00-…），大的更新；不看遍历顺序
            let newer = newest.as_ref().is_none_or(|(best, best_path)| {
                modified > *best || (modified == *best && path.file_name() > best_path.file_name())
            });
            if newer {
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
    lines
        .filter(|line| line.contains(needle))
        .next_back()
        .map(str::to_string)
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

#[cfg(test)]
mod newest_tests {
    use super::*;

    #[test]
    fn 修改时间相同时按文件名取新会话_不看遍历顺序() {
        let dir = std::env::temp_dir().join(format!("am-newest-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let day = dir.join("2026/09/17");
        fs::create_dir_all(&day).unwrap();
        let same =
            std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_789_000_000);
        for name in [
            "rollout-2026-09-17T09-00-00-new.jsonl",
            "rollout-2026-09-17T01-00-00-old.jsonl",
        ] {
            let path = day.join(name);
            fs::write(&path, "{}\n").unwrap();
            fs::File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(same)
                .unwrap();
        }
        let picked = newest_rollout(&dir).unwrap();
        assert!(
            picked.ends_with("rollout-2026-09-17T09-00-00-new.jsonl"),
            "{picked:?}"
        );
        let _ = fs::remove_dir_all(&dir);
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::io::Write;
    use std::process::{Command, Stdio};

    const SCRIPT: &str = include_str!("../assets/statusline.sh");

    /// 按 Claude Code 的方式跑一次部署好的状态栏脚本：绝对路径调用，stdin 推 rate_limits。
    fn run_statusline(script: &Path, home: &Path, config_dir: Option<&Path>, five: u32) {
        let mut cmd = Command::new("/bin/bash");
        cmd.arg(script)
            .env("HOME", home)
            .env("AM_STATUSLINE_NOW", "1789871000")
            .env_remove("CLAUDE_CONFIG_DIR")
            .stdin(Stdio::piped())
            .stdout(Stdio::null());
        if let Some(dir) = config_dir {
            cmd.env("CLAUDE_CONFIG_DIR", dir);
        }
        let mut child = cmd.spawn().unwrap();
        let input = format!(
            r#"{{"model":{{"display_name":"Claude Test"}},"rate_limits":{{"five_hour":{{"used_percentage":{five},"resets_at":1789900000}},"seven_day":{{"used_percentage":41,"resets_at":1790400000}}}}}}"#
        );
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
        assert!(child.wait().unwrap().success());
    }

    /// 开了 CLAUDE_CODE_SUBPROCESS_ENV_SCRUB 时状态栏子进程拿不到 CLAUDE_CONFIG_DIR：
    /// 从 transcript_path（<配置目录>/projects/…）反推，额度仍记在该实例名下。
    #[test]
    fn statusline_falls_back_to_transcript_dir_when_config_dir_is_scrubbed() {
        let base = std::env::temp_dir().join(format!("am-usage-scrub-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let home = base.join("home");
        let data = base.join("data");
        let instance = home.join(".claude-relay");
        fs::create_dir_all(&data).unwrap();
        fs::create_dir_all(instance.join("projects/p")).unwrap();
        let script = data.join("statusline.sh");
        fs::write(&script, SCRIPT).unwrap();

        let mut child = Command::new("/bin/bash")
            .arg(&script)
            .env("HOME", &home)
            .env("AM_STATUSLINE_NOW", "1789871000")
            .env_remove("CLAUDE_CONFIG_DIR")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        let input = format!(
            r#"{{"transcript_path":"{}/projects/p/s.jsonl","rate_limits":{{"five_hour":{{"used_percentage":17,"resets_at":1789900000}}}}}}"#,
            instance.display()
        );
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
        assert!(child.wait().unwrap().success());

        let via_profile = read_profile("claude", &instance, &data).unwrap();
        assert_eq!(via_profile["windows"][0]["used_percent"], json!(17.0));
        assert!(entries(&instance.join("projects/p")).is_empty());
        let _ = fs::remove_dir_all(&base);
    }

    fn entries(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn 状态栏脚本与读取端编码一致_额度落在数据目录_不同配置目录互不覆盖() {
        let base = std::env::temp_dir().join(format!(
            "am-usage-key-{}-{:x}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let data = base.join("data");
        let home = base.join("home");
        // 两个目录按朴素的「/ 换成 %2F」会撞成同一个文件名，% 必须先转义
        let a = home.join("a").join("b");
        let b = home.join("a%2Fb");
        let default_dir = home.join(".claude");
        for dir in [&data, &a, &b, &default_dir] {
            fs::create_dir_all(dir).unwrap();
        }
        let script = data.join("statusline.sh");
        fs::write(&script, SCRIPT).unwrap();

        run_statusline(&script, &home, Some(&a), 11);
        run_statusline(&script, &home, Some(&b), 22);
        run_statusline(&script, &home, None, 33);
        // CLAUDE_CONFIG_DIR 写成 "~/..." 时（引号里 shell 不展开）按 HOME 展开后编码
        let tilde = std::path::PathBuf::from("~/a%2Fb");
        run_statusline(&script, &home, Some(&tilde), 44);

        // 助手配置目录里不产生任何文件
        for dir in [&a, &b, &default_dir] {
            assert!(entries(dir).is_empty(), "{} 被写入了文件", dir.display());
        }
        assert_eq!(entries(&data.join("usage")).len(), 3);

        let used = |dir: &Path| {
            let path = snapshot_path(&data, dir);
            assert!(path.starts_with(data.join("usage")));
            statusline_snapshot(&path).unwrap()["windows"][0]["used_percent"].clone()
        };
        assert_eq!(used(&a), json!(11.0));
        assert_eq!(used(&b), json!(44.0), "~ 展开后与同一目录共用一份");
        assert_eq!(used(&default_dir), json!(33.0));

        // 读取端经 read / read_profile 也读到同一份
        let via_read = read(&home, &data, "claude", None).unwrap();
        assert_eq!(
            via_read["source"],
            json!(snapshot_path(&data, &default_dir).display().to_string())
        );
        assert_eq!(via_read["checked_at"], json!("2026-09-20T02:23:20Z"));
        let via_profile = read_profile("claude", &a, &data).unwrap();
        assert_eq!(via_profile["windows"][0]["used_percent"], json!(11.0));

        let _ = fs::remove_dir_all(&base);
    }
}
