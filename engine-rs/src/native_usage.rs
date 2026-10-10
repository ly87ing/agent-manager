//! 用量统计：从助手自己写的会话记录里累加 token，按模型算花了多少。
//!
//! 数据源是两家的会话文件，不联网、不调 API：
//! - Claude：`~/.claude/projects/**/*.jsonl` 的 `assistant` 事件，`message.usage` 是**本次增量**；
//! - Codex：`~/.codex/sessions/**/rollout-*.jsonl` 的 `token_count` 事件。
//!   它的 `total_token_usage` 是**整个会话的累计值**，直接累加会严重虚高，
//!   要用 `last_token_usage`；模型名不在这个事件里，得从前面的 `turn_context` 记住。
//!
//! 增量扫描：每个文件记住处理到哪个字节。JSONL 只追加，偏移因此一直有效。只处理到最后
//! 一个换行符为止——助手正在写的那一行是半截的，读了会解析失败或算错。
//!
//! 不重复计数（均用真实 claude 2.1.281 / codex 0.156.1 核对过会话文件的写法）：
//! - Claude 的 `--fork-session` 会新建一个会话文件，把之前的回复（同一个 message.id、同样的
//!   用量）原样抄进去；因此按 message.id 在所有文件之间去重，去重键存库（8 字节摘要）。
//! - Codex 复制型分叉与子代理的会话文件开头抄了父会话的记录（含 `token_count`），抄完写一条
//!   归属本会话的 `thread_settings_applied`；这段前缀不算，只取它的累计值作为本会话的起点。
//!   前缀里抄来的标记保留父会话的归属，不当成结束（见 `codex_prefix_end`）。
//! - Codex 同一累计值重发的 `token_count`（如限额更新时）只算一次：累计值变了才是新的用量。
//! - Claude 文件被截断时从头重读（有 message.id 去重兜底）；Codex 的不重读，从当前最后一个
//!   完整行之后接着读。
//!
//! 独立环境（实例）的会话在各自配置目录下，同样统计，助手一栏记作 `claude:<实例>` /
//! `codex:<实例>`。
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

/// Claude 回复的去重键（message.id 的 8 字节摘要），跨文件去重用。
const SEEN_DDL: &str = "CREATE TABLE IF NOT EXISTS usage_seen_messages (
    key INTEGER PRIMARY KEY NOT NULL
)";

fn table_exists(conn: &Connection, name: &str) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
        [name],
        |row| row.get::<_, i64>(0),
    )? > 0)
}

/// 建表并补齐旧库缺的列。返回去重表是不是这次才建的（需要用已扫过的内容回填）。
fn ensure_tables(conn: &Connection) -> Result<bool> {
    conn.execute_batch(SCAN_STATE_DDL)?;
    conn.execute_batch(HOURLY_DDL)?;
    let columns: Vec<String> = conn
        .prepare("SELECT name FROM pragma_table_info('usage_scan_state')")?
        .query_map([], |row| row.get(0))?
        .collect::<std::result::Result<_, _>>()?;
    if !columns.iter().any(|name| name == "last_total") {
        conn.execute_batch("ALTER TABLE usage_scan_state ADD COLUMN last_total TEXT")?;
    }
    if !columns.iter().any(|name| name == "fork_pending") {
        conn.execute_batch(
            "ALTER TABLE usage_scan_state ADD COLUMN fork_pending INTEGER NOT NULL DEFAULT 0",
        )?;
    }
    if !columns.iter().any(|name| name == "fork_owner") {
        conn.execute_batch("ALTER TABLE usage_scan_state ADD COLUMN fork_owner TEXT")?;
    }
    let fresh = !table_exists(conn, "usage_seen_messages")?;
    conn.execute_batch(SEEN_DDL)?;
    Ok(fresh)
}

/// message.id → 去重键。
fn seen_key(id: &str) -> i64 {
    let hex = crate::native_mcp::model::sha256_hex_bytes(id.as_bytes());
    u64::from_str_radix(&hex[..16], 16).unwrap_or(0) as i64
}

/// 记下一条 Claude 回复；之前（任何文件里）见过就返回 false。
fn mark_seen(conn: &Connection, id: &str) -> Result<bool> {
    Ok(conn.execute(
        "INSERT OR IGNORE INTO usage_seen_messages (key) VALUES (?1)",
        [seen_key(id)],
    )? == 1)
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
fn parse_claude_line(line: &str, agent: &str) -> Option<(String, Bucket, Tokens)> {
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
    Some((dedup, (hour, agent.to_string(), model.to_string()), tokens))
}

/// 一条 `token_count` 的本次增量：`last_token_usage` 才是增量，`total_token_usage` 是会话累计。
/// 全零（如上下文已满时的合成事件）返回 None。
fn codex_tokens(
    event: &Value,
    info: &Value,
    model: Option<&str>,
    agent: &str,
) -> Option<(Bucket, Tokens)> {
    let last = info.get("last_token_usage")?;
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
            agent.to_string(),
            model.unwrap_or("未知模型").to_string(),
        ),
        tokens,
    ))
}

/// 会话元信息表明这是复制型分叉或子代理的会话：开头抄了父会话的记录。
fn codex_is_fork(payload: &Value) -> bool {
    let non_empty =
        |value: Option<&Value>| value.and_then(Value::as_str).is_some_and(|v| !v.is_empty());
    non_empty(payload.get("forked_from_id"))
        || non_empty(
            payload
                .get("source")
                .and_then(|source| source.get("subagent"))
                .and_then(|subagent| subagent.get("thread_spawn"))
                .and_then(|spawn| spawn.get("parent_thread_id")),
        )
}

/// 一个 Codex 会话文件跨多次扫描要记住的东西。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct CodexFile {
    /// `turn_context` 带的模型名，给后面的 `token_count` 用。
    model: Option<String>,
    /// 上一条计入（或作为起点）的累计值；同一累计值重发不再算。
    last_total: Option<String>,
    /// 分叉文件还没读到结束抄来前缀的 `thread_settings_applied`：之前的 `token_count` 是抄来的。
    fork_pending: bool,
    /// 分叉文件自己的会话 id（`session_meta.id`），用来认出哪条标记是它自己的。
    fork_owner: Option<String>,
}

/// 文件多久没变才算写完。分叉文件读到末尾还没见到自己的标记时，写完了说明是不写标记的旧版本，
/// 还在变就可能正写到一半（抄来的前缀与标记是同一次追加写入的），等下次再读。
const CODEX_SETTLE_SECS: u64 = 30;

/// 分叉文件的抄来前缀在哪条 `thread_settings_applied` 处结束：返回那一行在 `text` 里的起始字节。
///
/// 新版本（`rust-v0.156.1` 已是）的标记带 `thread_id`（归属会话），抄来的父会话标记保留原
/// 归属——分叉的分叉、父会话恢复过都会让前缀里夹着别人的标记——只有归属本会话的那条才结束
/// 前缀。更早的标记不带归属：本段里没有本会话的标记时，第一条不带归属的就是结束。不知道本会话
/// id 时第一条标记就是结束。
fn codex_prefix_end(text: &str, owner: Option<&str>) -> Option<usize> {
    let mut first_any = None;
    let mut first_anonymous = None;
    let mut position = 0;
    for line in text.split_inclusive('\n') {
        let start = position;
        position += line.len();
        if !line.contains("thread_settings_applied") {
            continue;
        }
        let Ok(event) = serde_json::from_str::<Value>(line.trim_end()) else {
            continue;
        };
        let Some(payload) = event.get("payload") else {
            continue;
        };
        if payload.get("type").and_then(Value::as_str) != Some("thread_settings_applied") {
            continue;
        }
        first_any.get_or_insert(start);
        match payload.get("thread_id").and_then(Value::as_str) {
            Some(id) if owner == Some(id) => return Some(start),
            Some(_) => {}
            None => {
                first_anonymous.get_or_insert(start);
            }
        }
    }
    if owner.is_none() {
        first_any
    } else {
        first_anonymous
    }
}

/// 处理一段 Codex 会话记录，返回这段里新增的用量；第二项是只消费到哪个字节（分叉文件的前缀
/// 还没结束、文件又还在变时，下次从前缀开头重读），`None` 表示整段都已处理。`settled` 表示
/// 文件已有一阵没变。
fn codex_chunk_usage(
    chunk: &str,
    state: &mut CodexFile,
    agent: &str,
    settled: bool,
) -> (Vec<(Bucket, Tokens)>, Option<usize>) {
    let mut out = Vec::new();
    // 分叉文件里、结束标记之前的 token_count：先扣住，读到结束标记就作废
    let mut held: Vec<(Bucket, Tokens, Option<String>)> = Vec::new();
    let mut held_total: Option<String> = None;
    let mut pending_from = state.fork_pending.then_some(0);
    let mut prefix_end = if state.fork_pending {
        codex_prefix_end(chunk, state.fork_owner.as_deref())
    } else {
        None
    };
    let mut position = 0;
    for raw in chunk.split_inclusive('\n') {
        let start = position;
        position += raw.len();
        let Ok(event) = serde_json::from_str::<Value>(raw.trim_end()) else {
            continue;
        };
        let Some(payload) = event.get("payload") else {
            continue;
        };
        if event.get("type").and_then(Value::as_str) == Some("session_meta") {
            if codex_is_fork(payload) {
                state.fork_pending = true;
                state.fork_owner = payload
                    .get("id")
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty())
                    .map(str::to_string);
                pending_from = Some(position);
                prefix_end = codex_prefix_end(&chunk[position..], state.fork_owner.as_deref())
                    .map(|end| end + position);
            }
            continue;
        }
        if let Some(found) = payload.get("model").and_then(Value::as_str) {
            state.model = Some(found.to_string());
        }
        match payload.get("type").and_then(Value::as_str) {
            // 抄来的父会话记录到此为止：它们的用量已记在父会话名下；其余标记是抄来的
            Some("thread_settings_applied") if state.fork_pending && prefix_end == Some(start) => {
                if held_total.is_some() {
                    state.last_total = held_total.take();
                }
                held.clear();
                state.fork_pending = false;
                pending_from = None;
            }
            Some("token_count") => {
                let Some(info) = payload.get("info").filter(|info| !info.is_null()) else {
                    continue;
                };
                let total = info
                    .get("total_token_usage")
                    .filter(|total| !total.is_null())
                    .map(Value::to_string);
                if state.fork_pending {
                    if total.is_some() {
                        held_total = total.clone();
                    }
                    if let Some((bucket, tokens)) =
                        codex_tokens(&event, info, state.model.as_deref(), agent)
                    {
                        held.push((bucket, tokens, total));
                    }
                    continue;
                }
                if total.is_some() && total == state.last_total {
                    continue;
                }
                if total.is_some() {
                    state.last_total = total;
                }
                if let Some(usage) = codex_tokens(&event, info, state.model.as_deref(), agent) {
                    out.push(usage);
                }
            }
            _ => {}
        }
    }
    if state.fork_pending && !held.is_empty() {
        if !settled {
            // 可能正写到一半：前缀先不消费，下次从前缀开头重读
            return (out, pending_from);
        }
        // 文件已不再变仍没有结束标记：不写标记的旧版本，分不清哪些是抄来的，按普通文件算
        state.fork_pending = false;
        for (bucket, tokens, total) in held {
            if total.is_some() && total == state.last_total {
                continue;
            }
            if total.is_some() {
                state.last_total = total;
            }
            out.push((bucket, tokens));
        }
    }
    (out, None)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SessionKind {
    Claude,
    Codex,
}

/// 各账号的配置目录：主配置与各独立环境。返回（目录，助手一栏的记法，格式）。
/// 实例目录取自实例定义；同一目录只出现一次。
pub(crate) fn session_roots(ctx: &Context) -> Vec<(PathBuf, String, SessionKind)> {
    let home = &ctx.home_dir;
    let mut roots: Vec<(PathBuf, String, SessionKind)> = vec![
        (
            home.join(".claude"),
            "claude".to_string(),
            SessionKind::Claude,
        ),
        (home.join(".codex"), "codex".to_string(), SessionKind::Codex),
    ];
    if let Ok(policy) = store::effective_policy(ctx) {
        for def in crate::native_profiles::model::load_definitions(&policy)
            .into_iter()
            .flatten()
        {
            let Ok(dir) = crate::native_profiles::paths::config_dir(ctx, &def) else {
                continue;
            };
            let kind = match def.kind {
                crate::native_profiles::model::ProfileKind::Claude => SessionKind::Claude,
                crate::native_profiles::model::ProfileKind::Codex => SessionKind::Codex,
            };
            if roots.iter().any(|(existing, _, _)| existing == &dir) {
                continue;
            }
            let label = format!("{}:{}", def.kind.as_str(), def.id);
            roots.push((dir, label, kind));
        }
    }
    roots
}

/// 会话文件清单：各账号目录下的 Claude projects、Codex sessions，各自递归找 .jsonl。
/// 返回（文件，助手一栏的记法，格式）。
fn session_files(ctx: &Context) -> Vec<(PathBuf, String, SessionKind)> {
    let mut out = Vec::new();
    for (root, label, kind) in session_roots(ctx) {
        let (dir, prefix) = match kind {
            SessionKind::Claude => (root.join("projects"), None),
            SessionKind::Codex => (root.join("sessions"), Some("rollout-")),
        };
        let mut stack = vec![dir];
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
                if !name.ends_with(".jsonl") || prefix.is_some_and(|p| !name.starts_with(p)) {
                    continue;
                }
                out.push((path, label.clone(), kind));
            }
        }
    }
    out
}

struct ScanState {
    offset: u64,
    codex: CodexFile,
    /// Codex 文件被截断、偏移已挪到末尾：即使没有新内容也要把新偏移记下。
    moved: bool,
}

/// 截断后接着读的位置：当前最后一个完整行之后。Codex 可能正写到一半，停在半截行里会让那一行
/// 下次读成残片、被跳过；往前 1 MiB 内找不到换行就停在末尾。
fn last_line_boundary(path: &Path, size: u64) -> u64 {
    const WINDOW: u64 = 1024 * 1024;
    let from = size.saturating_sub(WINDOW);
    let Ok(mut file) = std::fs::File::open(path) else {
        return size;
    };
    let mut buf = Vec::new();
    if file.seek(SeekFrom::Start(from)).is_err()
        || file
            .by_ref()
            .take(size - from)
            .read_to_end(&mut buf)
            .is_err()
    {
        return size;
    }
    match buf.iter().rposition(|byte| *byte == b'\n') {
        Some(index) => from + index as u64 + 1,
        None if from == 0 => 0,
        None => size,
    }
}

/// 上次扫到哪里。文件变小说明被重写过：Claude 从头重读（有 message.id 去重兜底），Codex
/// 不重读（分不清哪些已经算过），从当前最后一个完整行之后接着往后读。
fn load_scan_state(conn: &Connection, path: &str, size: u64, kind: SessionKind) -> ScanState {
    type Row = (i64, Option<String>, Option<String>, i64, Option<String>);
    let row: Option<Row> = conn
        .query_row(
            "SELECT offset, model, last_total, fork_pending, fork_owner FROM usage_scan_state WHERE path = ?1",
            [path],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        )
        .ok();
    match row {
        Some((offset, model, last_total, fork_pending, fork_owner)) => {
            let codex = CodexFile {
                model,
                last_total,
                fork_pending: fork_pending != 0,
                fork_owner,
            };
            let offset = offset as u64;
            if offset <= size {
                ScanState {
                    offset,
                    codex,
                    moved: false,
                }
            } else if kind == SessionKind::Codex {
                ScanState {
                    offset: last_line_boundary(Path::new(path), size),
                    codex,
                    moved: true,
                }
            } else {
                ScanState {
                    offset: 0,
                    codex: CodexFile::default(),
                    moved: true,
                }
            }
        }
        None => ScanState {
            offset: 0,
            codex: CodexFile::default(),
            moved: false,
        },
    }
}

/// 去重表刚建：把各 Claude 文件里已经扫过的那部分回复先记进去（不计数），之后分叉出来的
/// 会话文件里抄来的旧回复才认得出来。
fn backfill_seen(conn: &Connection, files: &[(PathBuf, String, SessionKind)]) -> Result<()> {
    for (path, label, kind) in files {
        if *kind != SessionKind::Claude {
            continue;
        }
        let key = path.display().to_string();
        let offset: Option<i64> = conn
            .query_row(
                "SELECT offset FROM usage_scan_state WHERE path = ?1",
                [&key],
                |row| row.get(0),
            )
            .ok();
        let Some(offset) = offset.filter(|offset| *offset > 0) else {
            continue;
        };
        let Ok(mut file) = std::fs::File::open(path) else {
            continue;
        };
        let mut buf = Vec::new();
        if file
            .by_ref()
            .take(offset as u64)
            .read_to_end(&mut buf)
            .is_err()
        {
            continue;
        }
        for line in String::from_utf8_lossy(&buf).lines() {
            if let Some((dedup, _, _)) = parse_claude_line(line, label) {
                mark_seen(conn, &dedup)?;
            }
        }
    }
    Ok(())
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
    let files = session_files(ctx);
    let mut conn = store::open_write(ctx).map_err(|err| DispatchError::from(err))?;
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(DispatchError::from)?;
    let fresh_seen = ensure_tables(&tx).map_err(DispatchError::from)?;
    if fresh_seen {
        backfill_seen(&tx, &files).map_err(DispatchError::from)?;
    }

    let mut totals: BTreeMap<Bucket, Tokens> = BTreeMap::new();
    let mut scanned_files = 0i64;
    let mut scanned_bytes = 0i64;

    for (path, agent, kind) in &files {
        let Ok(meta) = std::fs::metadata(path) else {
            continue;
        };
        let size = meta.len();
        let settled = meta
            .modified()
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age.as_secs() >= CODEX_SETTLE_SECS);
        let key = path.display().to_string();
        let mut state = load_scan_state(&tx, &key, size, *kind);
        let chunk = if state.offset >= size {
            None
        } else {
            read_new_lines(path, state.offset)
        };
        let mut new_offset = match &chunk {
            Some((_, new_offset)) => *new_offset,
            // 没有新的完整行；Codex 截断后跳到末尾的偏移仍要记下
            None if state.moved => state.offset,
            None => continue,
        };
        if let Some((chunk, _)) = &chunk {
            scanned_files += 1;
            scanned_bytes += (new_offset - state.offset) as i64;
            match kind {
                SessionKind::Claude => {
                    for line in chunk.lines() {
                        if let Some((dedup, bucket, tokens)) = parse_claude_line(line, agent) {
                            // 同一条回复在任何文件里只算一次（分叉会话会把旧回复抄进新文件）
                            if mark_seen(&tx, &dedup).map_err(DispatchError::from)? {
                                totals.entry(bucket).or_default().add(&tokens);
                            }
                        }
                    }
                }
                SessionKind::Codex => {
                    let (usage, consumed) =
                        codex_chunk_usage(chunk, &mut state.codex, agent, settled);
                    for (bucket, tokens) in usage {
                        totals.entry(bucket).or_default().add(&tokens);
                    }
                    if let Some(consumed) = consumed {
                        new_offset = state.offset + consumed as u64;
                    }
                }
            }
        }
        tx.execute(
            "INSERT INTO usage_scan_state (path, offset, size, model, updated_at, last_total, fork_pending, fork_owner)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(path) DO UPDATE SET offset=excluded.offset, size=excluded.size,
                 model=excluded.model, updated_at=excluded.updated_at,
                 last_total=excluded.last_total, fork_pending=excluded.fork_pending,
                 fork_owner=excluded.fork_owner",
            rusqlite::params![
                &key,
                new_offset as i64,
                size as i64,
                state.codex.model,
                chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                state.codex.last_total,
                state.codex.fork_pending as i64,
                state.codex.fork_owner,
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
    tx.execute("DELETE FROM usage_seen_messages", [])
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
        let (dedup, bucket, tokens) = parse_claude_line(&line, "claude").unwrap();
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
        assert!(parse_claude_line(&user, "claude").is_none());
    }

    fn codex_line(ts: &str, payload: Value) -> String {
        json!({"timestamp": ts, "type": "event_msg", "payload": payload}).to_string()
    }

    fn token_count(total: i64, last_input: i64, last_output: i64) -> Value {
        json!({"type": "token_count", "info": {
            "total_token_usage": {"input_tokens": total, "output_tokens": 0, "total_tokens": total},
            "last_token_usage": {"input_tokens": last_input, "cached_input_tokens": 0, "output_tokens": last_output}
        }})
    }

    #[test]
    fn codex_用本次增量而不是会话累计() {
        let mut state = CodexFile::default();
        // 先出现 turn_context，模型名要被记住
        let chunk = [
            json!({"timestamp": "2026-09-17T09:00:00Z", "type": "turn_context", "payload": {"model": "gpt-5.6-sol"}}).to_string(),
            codex_line("2026-09-17T09:29:42.648Z", json!({
                "type": "token_count",
                "info": {
                    // 累计值：直接累加它会把整个会话重复计进去
                    "total_token_usage": {"input_tokens": 116769, "cached_input_tokens": 78592, "output_tokens": 1057},
                    "last_token_usage": {"input_tokens": 37694, "cached_input_tokens": 29184, "cache_write_input_tokens": 0, "output_tokens": 461}
                }
            })),
        ]
        .join("\n");
        let usage = codex_chunk_usage(&chunk, &mut state, "codex", true).0;
        assert_eq!(state.model.as_deref(), Some("gpt-5.6-sol"));
        assert_eq!(usage.len(), 1);
        let (bucket, tokens) = &usage[0];
        assert_eq!(bucket.1, "codex");
        assert_eq!(bucket.2, "gpt-5.6-sol", "模型名来自前面的 turn_context");
        // cached 是 input 里命中缓存的部分，要拆开算
        assert_eq!(tokens.input, 37694 - 29184);
        assert_eq!(tokens.cache_read, 29184);
        assert_eq!(tokens.output, 461);
        assert_ne!(tokens.input, 116769, "不能拿会话累计当增量");
    }

    /// 同一累计值重发（如限额更新时）只算一次；跨两次扫描同样认得出来。
    #[test]
    fn codex_同一累计值重发只算一次() {
        let mut state = CodexFile::default();
        let first = [
            codex_line("2026-09-17T09:00:00Z", token_count(100, 100, 10)),
            codex_line("2026-09-17T09:00:01Z", token_count(100, 100, 10)),
        ]
        .join("\n");
        assert_eq!(
            codex_chunk_usage(&first, &mut state, "codex", true).0.len(),
            1
        );
        let later = [
            codex_line("2026-09-17T09:01:00Z", token_count(100, 100, 10)),
            codex_line("2026-09-17T09:02:00Z", token_count(160, 60, 5)),
        ]
        .join("\n");
        let usage = codex_chunk_usage(&later, &mut state, "codex", true).0;
        assert_eq!(usage.len(), 1);
        assert_eq!(usage[0].1.input, 60);
    }

    /// 复制型分叉 / 子代理：开头抄来的父会话 token_count 到 thread_settings_applied 为止都不算，
    /// 之后本会话自己的照常算；标记跨两次扫描也有效。0.156.1 的 exec fork 先写标记、没有抄来的
    /// token_count，同样只算自己的。
    #[test]
    fn codex_分叉文件开头抄来的用量不算() {
        let meta = json!({"timestamp": "2026-09-17T10:00:00Z", "type": "session_meta",
            "payload": {"id": "child", "forked_from_id": "parent"}})
        .to_string();
        let mut state = CodexFile::default();
        let copied = [
            meta.clone(),
            codex_line("2026-09-17T10:00:00Z", token_count(1000, 1000, 50)),
            codex_line("2026-09-17T10:00:00Z", token_count(2000, 1000, 50)),
        ]
        .join("\n");
        // 标记还没写到：先扣住，不算也不当成旧版本
        assert!(codex_chunk_usage(&meta, &mut state, "codex", true)
            .0
            .is_empty());
        assert!(state.fork_pending);
        let rest = [
            codex_line("2026-09-17T10:00:00Z", token_count(1000, 1000, 50)),
            codex_line("2026-09-17T10:00:00Z", token_count(2000, 1000, 50)),
            codex_line(
                "2026-09-17T10:00:00Z",
                json!({"type": "thread_settings_applied"}),
            ),
            // 子会话延续父会话的累计值：第一条是限额更新重发，不算
            codex_line("2026-09-17T10:00:01Z", token_count(2000, 1000, 50)),
            codex_line("2026-09-17T10:00:02Z", token_count(3050, 1000, 50)),
        ]
        .join("\n");
        let usage = codex_chunk_usage(&rest, &mut state, "codex", true).0;
        assert_eq!(usage.len(), 1, "只算分叉后自己的那一次");
        assert!(!state.fork_pending);

        // 一次读到整段也一样
        let mut once = CodexFile::default();
        let whole = format!(
            "{copied}\n{}\n{}",
            codex_line(
                "2026-09-17T10:00:00Z",
                json!({"type": "thread_settings_applied"})
            ),
            codex_line("2026-09-17T10:00:02Z", token_count(3050, 1000, 50))
        );
        assert_eq!(
            codex_chunk_usage(&whole, &mut once, "codex", true).0.len(),
            1
        );

        // 真实 0.156.1 exec fork：标记在前，第一条 token_count 就是自己的
        let mut referenced = CodexFile::default();
        let real = [
            meta.clone(),
            codex_line(
                "2026-09-17T10:00:00Z",
                json!({"type": "thread_settings_applied"}),
            ),
            codex_line(
                "2026-09-17T10:00:00Z",
                json!({"type": "thread_settings_applied"}),
            ),
            codex_line("2026-09-17T10:00:02Z", token_count(3150, 1050, 0)),
        ]
        .join("\n");
        assert_eq!(
            codex_chunk_usage(&real, &mut referenced, "codex", true)
                .0
                .len(),
            1
        );

        // 子代理的会话元信息同样按分叉处理
        let spawned = json!({"timestamp": "t", "type": "session_meta", "payload": {"id": "sub",
            "source": {"subagent": {"thread_spawn": {"parent_thread_id": "parent", "depth": 1}}}}});
        assert!(codex_is_fork(&spawned["payload"]));
        assert!(!codex_is_fork(
            &json!({"id": "plain", "forked_from_id": null})
        ));
    }

    /// 不写 thread_settings_applied 的旧版本分叉文件：分不清哪些是抄来的，按普通文件算，
    /// 不会整份不计。
    #[test]
    fn codex_旧版本分叉文件没有标记时按普通文件算() {
        let mut state = CodexFile::default();
        let chunk = [
            json!({"timestamp": "t", "type": "session_meta", "payload": {"forked_from_id": "parent"}}).to_string(),
            codex_line("2026-09-17T10:00:00Z", token_count(100, 100, 1)),
            codex_line("2026-09-17T10:00:01Z", token_count(200, 100, 1)),
        ]
        .join("\n");
        assert_eq!(
            codex_chunk_usage(&chunk, &mut state, "codex", true).0.len(),
            2
        );
        assert!(!state.fork_pending);
    }

    fn marker(owner: Option<&str>) -> String {
        let mut payload = json!({"type": "thread_settings_applied"});
        if let Some(owner) = owner {
            payload["thread_id"] = json!(owner);
        }
        codex_line("2026-09-17T10:00:00Z", payload)
    }

    /// 分叉的分叉、或父会话恢复过：抄来的前缀里夹着父会话自己的标记（保留原归属），只有归属
    /// 本会话的标记才结束前缀；父会话自己那段用量不再算一遍。
    #[test]
    fn codex_前缀里抄来的标记不当成结束_只认本会话的() {
        let meta = json!({"timestamp": "t", "type": "session_meta",
            "payload": {"id": "grandchild", "forked_from_id": "child"}})
        .to_string();
        let chunk = [
            meta,
            // 祖父会话的用量
            codex_line("2026-09-17T10:00:00Z", token_count(1000, 1000, 50)),
            // 子会话自己的标记与用量，都是抄来的
            marker(Some("child")),
            codex_line("2026-09-17T10:00:00Z", token_count(2050, 1000, 50)),
            // 本会话的标记
            marker(Some("grandchild")),
            codex_line("2026-09-17T10:00:01Z", token_count(3100, 1000, 50)),
        ]
        .join("\n");
        let mut state = CodexFile::default();
        let (usage, consumed) = codex_chunk_usage(&chunk, &mut state, "codex", true);
        assert_eq!(usage.len(), 1, "只算本会话自己的那一次");
        assert_eq!(consumed, None);
        assert!(!state.fork_pending);

        // 不带归属的旧标记：本段里找不到本会话的标记时，第一条就是结束
        let old = [
            json!({"timestamp": "t", "type": "session_meta", "payload": {"id": "c", "forked_from_id": "p"}}).to_string(),
            codex_line("2026-09-17T10:00:00Z", token_count(1000, 1000, 50)),
            marker(None),
            codex_line("2026-09-17T10:00:01Z", token_count(2050, 1000, 50)),
        ]
        .join("\n");
        let mut state = CodexFile::default();
        assert_eq!(
            codex_chunk_usage(&old, &mut state, "codex", true).0.len(),
            1
        );
    }

    /// 读到前缀中途、文件还在变：前缀先不消费也不算，下次从前缀开头重读；本会话的标记写到后
    /// 只算自己的。文件已不再变仍没有标记，才按不写标记的旧版本整份算。
    #[test]
    fn codex_分叉前缀读到一半时等下次_不提前按旧版本算() {
        let meta = json!({"timestamp": "t", "type": "session_meta",
            "payload": {"id": "child", "forked_from_id": "parent"}})
        .to_string();
        let copied = [
            codex_line("2026-09-17T10:00:00Z", token_count(1000, 1000, 50)),
            marker(Some("parent")),
            codex_line("2026-09-17T10:00:00Z", token_count(2050, 1000, 50)),
        ]
        .join("\n");
        let partial = format!("{meta}\n{copied}\n");
        let mut state = CodexFile::default();
        let (usage, consumed) = codex_chunk_usage(&partial, &mut state, "codex", false);
        assert!(usage.is_empty());
        assert_eq!(
            consumed,
            Some(meta.len() + 1),
            "停在会话元信息之后、前缀开头"
        );
        assert!(state.fork_pending);
        assert_eq!(state.fork_owner.as_deref(), Some("child"));

        // 下次从前缀开头读到完整的一次追加
        let rest = format!(
            "{copied}\n{}\n{}\n",
            marker(Some("child")),
            codex_line("2026-09-17T10:00:02Z", token_count(3100, 1000, 50))
        );
        let (usage, consumed) = codex_chunk_usage(&rest, &mut state, "codex", false);
        assert_eq!(usage.len(), 1);
        assert_eq!(consumed, None);
        assert!(!state.fork_pending);

        // 同样的前缀，文件已不再变：按旧版本整份算
        let mut settled = CodexFile::default();
        assert_eq!(
            codex_chunk_usage(&partial, &mut settled, "codex", true)
                .0
                .len(),
            2
        );
    }

    #[test]
    fn codex_截断后停在最后一个完整行之后() {
        let dir = std::env::temp_dir().join(format!("am-usage-boundary-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("r.jsonl");
        std::fs::write(&path, "{\"a\":1}\n{\"half\":").unwrap();
        assert_eq!(last_line_boundary(&path, 16), 8);
        std::fs::write(&path, "{\"half\":").unwrap();
        assert_eq!(last_line_boundary(&path, 8), 0, "整份只有半行：从头读");
        let _ = std::fs::remove_dir_all(&dir);
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
