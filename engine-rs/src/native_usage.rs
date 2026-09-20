//! 用量统计：从助手自己写的会话记录里累加 token，按模型算花了多少。
//!
//! 数据源是两家的会话文件，不联网、不调 API：
//! - Claude：`~/.claude/projects/**/*.jsonl` 的 `assistant` 事件，`message.usage` 是**本次增量**；
//! - Codex：`~/.codex/sessions/**/rollout-*.jsonl` 的 `token_count` 事件。
//!   它的 `total_token_usage` 是**整个会话的累计值**，直接累加会严重虚高，
//!   要用 `last_token_usage`；模型名不在这个事件里，得从前面的 `turn_context` 记住。
//!
//! 增量扫描：每个文件记住处理到哪个字节。JSONL 只追加，偏移因此一直有效；
//! 文件被截断（size < 上次偏移）就从头重来。只处理到最后一个换行符为止——
//! 助手正在写的那一行是半截的，读了会解析失败或算错。
//!
//! 按 UTC 小时聚合存库，界面再按本地时区分组成天：存储里不掺时区，
//! 换个时区看同一份数据也不会错位。
//!
//! 费用是**估算**：按可编辑的单价表算，不等于账单。没填单价的模型只报 token 不报钱——
//! 编一个价格比不报更糟，用的人不会发现算错了。

use std::collections::BTreeMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use anyhow::Result;
use rusqlite::{Connection, TransactionBehavior};
use serde_json::{json, Map, Value};

use crate::native::{Context, DispatchError, Request};
use crate::store;

const SCAN_STATE_DDL: &str = "CREATE TABLE IF NOT EXISTS usage_scan_state (
    path TEXT PRIMARY KEY NOT NULL,
    offset INTEGER NOT NULL,
    size INTEGER NOT NULL,
    model TEXT,
    updated_at TEXT NOT NULL
)";

const HOURLY_DDL: &str = "CREATE TABLE IF NOT EXISTS usage_hourly (
    hour TEXT NOT NULL,
    agent TEXT NOT NULL,
    model TEXT NOT NULL,
    input INTEGER NOT NULL DEFAULT 0,
    output INTEGER NOT NULL DEFAULT 0,
    cache_write INTEGER NOT NULL DEFAULT 0,
    cache_read INTEGER NOT NULL DEFAULT 0,
    requests INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (hour, agent, model)
)";

fn ensure_tables(conn: &Connection) -> Result<()> {
    conn.execute_batch(SCAN_STATE_DDL)?;
    conn.execute_batch(HOURLY_DDL)?;
    Ok(())
}

/// 一段时间内某个模型的用量。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Tokens {
    pub(crate) input: i64,
    pub(crate) output: i64,
    /// 写进缓存的 token（Claude 的 cache_creation）。
    pub(crate) cache_write: i64,
    /// 命中缓存读到的 token，单价通常远低于普通输入。
    pub(crate) cache_read: i64,
    pub(crate) requests: i64,
}

impl Tokens {
    fn add(&mut self, other: &Tokens) {
        self.input += other.input;
        self.output += other.output;
        self.cache_write += other.cache_write;
        self.cache_read += other.cache_read;
        self.requests += other.requests;
    }
}

/// 聚合键：UTC 小时 + 助手 + 模型。
type Bucket = (String, String, String);

fn hour_of(timestamp: &str) -> Option<String> {
    let parsed = chrono::DateTime::parse_from_rfc3339(timestamp).ok()?;
    Some(
        parsed
            .with_timezone(&chrono::Utc)
            .format("%Y-%m-%dT%H:00:00Z")
            .to_string(),
    )
}

fn as_i64(value: Option<&Value>) -> i64 {
    value.and_then(Value::as_i64).unwrap_or(0)
}

/// 解析一行 Claude 会话事件。只认带用量的 assistant 事件。
///
/// 返回去重键：同一条消息在文件里出现两次（重试、续写）不能算两遍。
fn parse_claude_line(line: &str) -> Option<(String, Bucket, Tokens)> {
    let event: Value = serde_json::from_str(line).ok()?;
    if event.get("type").and_then(Value::as_str) != Some("assistant") {
        return None;
    }
    let message = event.get("message")?;
    let usage = message.get("usage")?;
    let model = message.get("model").and_then(Value::as_str)?;
    let hour = hour_of(event.get("timestamp").and_then(Value::as_str)?)?;
    // message.id 稳定标识一次回复；没有就退回 requestId
    let dedup = message
        .get("id")
        .and_then(Value::as_str)
        .or_else(|| event.get("requestId").and_then(Value::as_str))?
        .to_string();
    let tokens = Tokens {
        input: as_i64(usage.get("input_tokens")),
        output: as_i64(usage.get("output_tokens")),
        cache_write: as_i64(usage.get("cache_creation_input_tokens")),
        cache_read: as_i64(usage.get("cache_read_input_tokens")),
        requests: 1,
    };
    Some((dedup, (hour, "claude".into(), model.to_string()), tokens))
}

/// 解析一行 Codex 会话事件。
///
/// `turn_context` 带模型名，要记住给后面的 `token_count` 用；
/// `token_count` 的 `last_token_usage` 才是本次增量，`total_token_usage` 是会话累计。
fn parse_codex_line(line: &str, model: &mut Option<String>) -> Option<(Bucket, Tokens)> {
    let event: Value = serde_json::from_str(line).ok()?;
    let payload = event.get("payload")?;
    if let Some(found) = payload.get("model").and_then(Value::as_str) {
        *model = Some(found.to_string());
    }
    if payload.get("type").and_then(Value::as_str) != Some("token_count") {
        return None;
    }
    let last = payload.get("info")?.get("last_token_usage")?;
    let hour = hour_of(event.get("timestamp").and_then(Value::as_str)?)?;
    let input_total = as_i64(last.get("input_tokens"));
    let cached = as_i64(last.get("cached_input_tokens"));
    let tokens = Tokens {
        // cached 是 input 里命中缓存的那部分，分开记才好按各自单价算
        input: (input_total - cached).max(0),
        output: as_i64(last.get("output_tokens")),
        cache_write: as_i64(last.get("cache_write_input_tokens")),
        cache_read: cached,
        requests: 1,
    };
    if tokens.input == 0 && tokens.output == 0 && tokens.cache_read == 0 {
        return None;
    }
    Some((
        (
            hour,
            "codex".into(),
            model.clone().unwrap_or_else(|| "未知模型".into()),
        ),
        tokens,
    ))
}

/// 会话文件清单：Claude 的 projects 与 Codex 的 sessions，各自递归找 .jsonl。
fn session_files(home: &Path) -> Vec<(PathBuf, &'static str)> {
    let mut out = Vec::new();
    let mut walk = |root: PathBuf, agent: &'static str, prefix: Option<&str>| {
        let mut stack = vec![root];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let Ok(meta) = entry.metadata() else { continue };
                if meta.is_dir() {
                    stack.push(path);
                    continue;
                }
                let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                    continue;
                };
                if !name.ends_with(".jsonl") {
                    continue;
                }
                if prefix.is_some_and(|p| !name.starts_with(p)) {
                    continue;
                }
                out.push((path, agent));
            }
        }
    };
    walk(home.join(".claude/projects"), "claude", None);
    walk(home.join(".codex/sessions"), "codex", Some("rollout-"));
    out
}

struct ScanState {
    offset: u64,
    model: Option<String>,
}

fn load_scan_state(conn: &Connection, path: &str, size: u64) -> ScanState {
    let row: Option<(i64, i64, Option<String>)> = conn
        .query_row(
            "SELECT offset, size, model FROM usage_scan_state WHERE path = ?1",
            [path],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .ok();
    match row {
        // 文件变小说明被重写过，之前的偏移作废，从头再来
        Some((offset, _, model)) if (offset as u64) <= size => ScanState {
            offset: offset as u64,
            model,
        },
        _ => ScanState {
            offset: 0,
            model: None,
        },
    }
}

/// 从上次的偏移往后读，只处理到最后一个换行符为止。
///
/// 助手可能正在往这个文件里写，最后一行是半截的；把偏移停在完整行的边界上，
/// 下次再从那里接着读，这一行就不会被算成两半。
fn read_new_lines(path: &Path, from: u64) -> Option<(String, u64)> {
    let mut file = std::fs::File::open(path).ok()?;
    file.seek(SeekFrom::Start(from)).ok()?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).ok()?;
    let last_newline = buf.iter().rposition(|byte| *byte == b'\n')?;
    let complete = &buf[..=last_newline];
    Some((
        String::from_utf8_lossy(complete).into_owned(),
        from + complete.len() as u64,
    ))
}

/// 扫描全部会话文件，把新增部分累加进 `usage_hourly`。
pub(crate) fn scan(ctx: &Context, req: &Request) -> Result<Value, DispatchError> {
    let files = session_files(&ctx.home_dir);
    let mut conn = store::open_write(ctx).map_err(|err| DispatchError::from(err))?;
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(DispatchError::from)?;
    ensure_tables(&tx).map_err(DispatchError::from)?;

    let mut totals: BTreeMap<Bucket, Tokens> = BTreeMap::new();
    let mut scanned_files = 0i64;
    let mut scanned_bytes = 0i64;

    for (path, agent) in &files {
        let Ok(meta) = std::fs::metadata(path) else {
            continue;
        };
        let size = meta.len();
        let key = path.display().to_string();
        let state = load_scan_state(&tx, &key, size);
        if state.offset >= size {
            continue;
        }
        let Some((chunk, new_offset)) = read_new_lines(path, state.offset) else {
            continue;
        };
        scanned_files += 1;
        scanned_bytes += (new_offset - state.offset) as i64;

        let mut model = state.model.clone();
        // 同一段字节里的重复消息只算一次；跨段由字节偏移保证不重复读
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        for line in chunk.lines() {
            if line.trim().is_empty() {
                continue;
            }
            if *agent == "claude" {
                if let Some((dedup, bucket, tokens)) = parse_claude_line(line) {
                    if seen.insert(dedup) {
                        totals.entry(bucket).or_default().add(&tokens);
                    }
                }
            } else if let Some((bucket, tokens)) = parse_codex_line(line, &mut model) {
                totals.entry(bucket).or_default().add(&tokens);
            }
        }

        tx.execute(
            "INSERT INTO usage_scan_state (path, offset, size, model, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(path) DO UPDATE SET offset=excluded.offset, size=excluded.size,
                 model=excluded.model, updated_at=excluded.updated_at",
            rusqlite::params![
                &key,
                new_offset as i64,
                size as i64,
                model,
                chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
            ],
        )
        .map_err(DispatchError::from)?;
    }

    let buckets = totals.len() as i64;
    for ((hour, agent, model), tokens) in &totals {
        tx.execute(
            "INSERT INTO usage_hourly (hour, agent, model, input, output, cache_write, cache_read, requests)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(hour, agent, model) DO UPDATE SET
                 input = input + excluded.input,
                 output = output + excluded.output,
                 cache_write = cache_write + excluded.cache_write,
                 cache_read = cache_read + excluded.cache_read,
                 requests = requests + excluded.requests",
            rusqlite::params![
                hour,
                agent,
                model,
                tokens.input,
                tokens.output,
                tokens.cache_write,
                tokens.cache_read,
                tokens.requests
            ],
        )
        .map_err(DispatchError::from)?;
    }

    store::record_operation(
        &tx,
        Some(&req.run_id),
        "usage:scan",
        None,
        Some(&json!({
            "files": scanned_files,
            "bytes": scanned_bytes,
            "buckets": buckets,
        })),
    )
    .map_err(DispatchError::from)?;
    tx.commit().map_err(DispatchError::from)?;

    Ok(json!({
        "domain": "usage",
        "action": "scan",
        "files": scanned_files,
        "bytes": scanned_bytes,
        "buckets": buckets,
    }))
}

/// 单价表：美元 / 百万 token。没填的模型只报 token 不报钱。
fn pricing(ctx: &Context) -> Map<String, Value> {
    store::effective_policy(ctx)
        .ok()
        .and_then(|policy| {
            policy
                .get("usage")
                .and_then(|u| u.get("pricing"))
                .and_then(Value::as_object)
                .cloned()
        })
        .unwrap_or_default()
}

/// 只读：把已聚合的用量吐出来，附上单价表。不扫描、不写库。
pub(crate) fn status(ctx: &Context, _req: &Request) -> Result<Value, DispatchError> {
    let Some(conn) = store::open_readonly(ctx).map_err(DispatchError::from)? else {
        return Ok(json!({
            "domain": "usage",
            "rows": [],
            "pricing": pricing(ctx),
            "last_scan_at": Value::Null,
        }));
    };
    let table_exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='usage_hourly'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    if table_exists == 0 {
        return Ok(json!({
            "domain": "usage",
            "rows": [],
            "pricing": pricing(ctx),
            "last_scan_at": Value::Null,
        }));
    }

    let mut stmt = conn
        .prepare(
            "SELECT hour, agent, model, input, output, cache_write, cache_read, requests
             FROM usage_hourly ORDER BY hour DESC, agent, model",
        )
        .map_err(DispatchError::from)?;
    let rows = stmt
        .query_map([], |row| {
            Ok(json!({
                "hour": row.get::<_, String>(0)?,
                "agent": row.get::<_, String>(1)?,
                "model": row.get::<_, String>(2)?,
                "input": row.get::<_, i64>(3)?,
                "output": row.get::<_, i64>(4)?,
                "cache_write": row.get::<_, i64>(5)?,
                "cache_read": row.get::<_, i64>(6)?,
                "requests": row.get::<_, i64>(7)?,
            }))
        })
        .map_err(DispatchError::from)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(DispatchError::from)?;

    let last_scan_at: Option<String> = conn
        .query_row("SELECT MAX(updated_at) FROM usage_scan_state", [], |row| {
            row.get(0)
        })
        .ok()
        .flatten();

    Ok(json!({
        "domain": "usage",
        "rows": rows,
        "pricing": pricing(ctx),
        "last_scan_at": last_scan_at.map(Value::String).unwrap_or(Value::Null),
    }))
}

/// 清空统计：重新从头扫一遍前先把旧数据与偏移抹掉。
pub(crate) fn clear(ctx: &Context, req: &Request) -> Result<Value, DispatchError> {
    let mut conn = store::open_write(ctx).map_err(DispatchError::from)?;
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(DispatchError::from)?;
    ensure_tables(&tx).map_err(DispatchError::from)?;
    let rows = tx
        .execute("DELETE FROM usage_hourly", [])
        .map_err(DispatchError::from)?;
    tx.execute("DELETE FROM usage_scan_state", [])
        .map_err(DispatchError::from)?;
    store::record_operation(
        &tx,
        Some(&req.run_id),
        "usage:clear",
        None,
        Some(&json!({"rows": rows})),
    )
    .map_err(DispatchError::from)?;
    tx.commit().map_err(DispatchError::from)?;
    Ok(json!({"domain": "usage", "action": "clear", "deleted": rows}))
}

pub(crate) fn handle(ctx: &Context, req: &Request) -> Result<Value, DispatchError> {
    let verb = req.command.replace('.', ":");
    match verb.split_once(':').map(|(_, v)| v) {
        Some("status") => status(ctx, req),
        Some("scan") => scan(ctx, req),
        Some("clear") => clear(ctx, req),
        _ => Err(DispatchError::new("USAGE_ERROR", "未知的 usage 动作")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_只认带用量的回复_并给出去重键() {
        let line = json!({
            "type": "assistant",
            "timestamp": "2026-09-20T02:51:16.947Z",
            "requestId": "req_1",
            "message": {
                "id": "msg_1",
                "model": "claude-opus-5",
                "usage": {
                    "input_tokens": 2,
                    "output_tokens": 3018,
                    "cache_creation_input_tokens": 6803,
                    "cache_read_input_tokens": 444718
                }
            }
        })
        .to_string();
        let (dedup, bucket, tokens) = parse_claude_line(&line).unwrap();
        assert_eq!(dedup, "msg_1");
        assert_eq!(bucket.0, "2026-09-20T02:00:00Z", "按 UTC 小时归桶");
        assert_eq!(bucket.2, "claude-opus-5");
        assert_eq!(tokens.input, 2);
        assert_eq!(tokens.output, 3018);
        assert_eq!(tokens.cache_write, 6803);
        assert_eq!(tokens.cache_read, 444718);
        assert_eq!(tokens.requests, 1);

        // 用户消息没有用量，不该进统计
        let user = json!({"type": "user", "timestamp": "2026-09-20T02:51:16Z"}).to_string();
        assert!(parse_claude_line(&user).is_none());
    }

    #[test]
    fn codex_用本次增量而不是会话累计() {
        let mut model = None;
        // 先出现 turn_context，模型名要被记住
        let ctx_line = json!({
            "timestamp": "2026-09-17T09:00:00Z",
            "type": "turn_context",
            "payload": {"model": "gpt-5.6-sol"}
        })
        .to_string();
        assert!(parse_codex_line(&ctx_line, &mut model).is_none());
        assert_eq!(model.as_deref(), Some("gpt-5.6-sol"));

        let line = json!({
            "timestamp": "2026-09-17T09:29:42.648Z",
            "type": "event_msg",
            "payload": {
                "type": "token_count",
                "info": {
                    // 累计值：直接累加它会把整个会话重复计进去
                    "total_token_usage": {"input_tokens": 116769, "cached_input_tokens": 78592, "output_tokens": 1057},
                    "last_token_usage": {"input_tokens": 37694, "cached_input_tokens": 29184, "cache_write_input_tokens": 0, "output_tokens": 461}
                }
            }
        })
        .to_string();
        let (bucket, tokens) = parse_codex_line(&line, &mut model).unwrap();
        assert_eq!(bucket.2, "gpt-5.6-sol", "模型名来自前面的 turn_context");
        // cached 是 input 里命中缓存的部分，要拆开算
        assert_eq!(tokens.input, 37694 - 29184);
        assert_eq!(tokens.cache_read, 29184);
        assert_eq!(tokens.output, 461);
        assert_ne!(tokens.input, 116769, "不能拿会话累计当增量");
    }

    #[test]
    fn 半截的最后一行留到下次_不会算成两半() {
        let dir = std::env::temp_dir().join(format!("am-usage-tail-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("s.jsonl");
        std::fs::write(&path, "{\"a\":1}\n{\"b\":2}\n{\"half\":").unwrap();

        let (chunk, offset) = read_new_lines(&path, 0).unwrap();
        assert_eq!(chunk.lines().count(), 2, "半截那行不算");
        assert_eq!(offset, 16, "偏移停在完整行的边界");

        // 后半行写完后接着读，这一行才完整出现一次
        std::fs::write(&path, "{\"a\":1}\n{\"b\":2}\n{\"half\":3}\n").unwrap();
        let (chunk, _) = read_new_lines(&path, offset).unwrap();
        assert_eq!(chunk.trim(), "{\"half\":3}");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
