//! sources 域：rules 来源文本（rules.shared / rules.claude / rules.codex）读写。
//!
//! native-manager.db 的 rule_sources 表是来源的唯一可写事实源；编译期
//! include_str 固化 agent-rule/rules/src 的三份只读默认，运行时不读写
//! checkout 源文件，也不生成 rules/dist。读取（含缺失 DB/缺表）只返回默认
//! 且零写；真实保存使用 Immediate 事务 + revision CAS，审计仅记录
//! key/revision 等无正文元数据；保存定义绝不写助手文件或自动 apply。
//! 本域错误码统一 SOURCE_ERROR，且不使用共享 apply lock。

use std::collections::BTreeMap;
use std::collections::HashSet;

use rusqlite::{Connection, TransactionBehavior};
use serde_json::{json, Value};

use crate::envelope::diagnostic;
use crate::native::{reject_secret_text, Context, DispatchError, DispatchResult, Request};
use crate::native_mcp::model::{now_iso, sha256_hex};
use crate::store;

pub const KEY_SHARED: &str = "rules.shared";
pub const KEY_CLAUDE: &str = "rules.claude";
pub const KEY_CODEX: &str = "rules.codex";

pub const SOURCE_KEYS: [&str; 3] = [KEY_SHARED, KEY_CLAUDE, KEY_CODEX];

const DEFAULT_SHARED: &str = include_str!("../../agent-rule/rules/src/shared/core.md");
const DEFAULT_CLAUDE: &str = include_str!("../../agent-rule/rules/src/agents/claude/adapter.md");
const DEFAULT_CODEX: &str = include_str!("../../agent-rule/rules/src/agents/codex/adapter.md");

const TABLE_DDL: &str = "CREATE TABLE IF NOT EXISTS rule_sources (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    revision TEXT NOT NULL,
    updated_at TEXT NOT NULL
)";

/// 本域统一错误码（dispatch 契约：sources 一律 SOURCE_ERROR）。
const SOURCE_ERROR: &str = "SOURCE_ERROR";

fn source_error(message: impl Into<String>) -> DispatchError {
    DispatchError::new(SOURCE_ERROR, message)
}

fn internal(err: impl Into<anyhow::Error>) -> DispatchError {
    let err = err.into();
    DispatchError::new(SOURCE_ERROR, format!("{err:#}"))
}

pub fn default_value(key: &str) -> Option<&'static str> {
    match key {
        KEY_SHARED => Some(DEFAULT_SHARED),
        KEY_CLAUDE => Some(DEFAULT_CLAUDE),
        KEY_CODEX => Some(DEFAULT_CODEX),
        _ => None,
    }
}

pub fn label_of(key: &str) -> &'static str {
    match key {
        KEY_SHARED => "共享核心规则",
        KEY_CLAUDE => "Claude 专用适配",
        KEY_CODEX => "Codex 专用适配",
        _ => "未知来源",
    }
}

/// 来源 revision：当前有效文本的 sha256。
pub fn revision_of(text: &str) -> String {
    sha256_hex(text)
}

fn ensure_table(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(TABLE_DDL)?;
    Ok(())
}

/// 读取已保存的来源文本；缺表视为空（域名自管表，读路径不建表）。
fn load_stored_from(conn: &Connection) -> anyhow::Result<BTreeMap<String, String>> {
    let mut stmt = match conn.prepare("SELECT key, value FROM rule_sources") {
        Ok(stmt) => stmt,
        Err(exc) => {
            if exc.to_string().contains("no such table") {
                return Ok(BTreeMap::new());
            }
            return Err(exc.into());
        }
    };
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut out = BTreeMap::new();
    for row in rows {
        let (key, value) = row?;
        if default_value(&key).is_some() {
            out.insert(key, value);
        }
    }
    Ok(out)
}

/// 由连接读出三个来源的当前文本（已存值覆盖编译期默认）。
pub fn current_sources_from(conn: &Connection) -> anyhow::Result<BTreeMap<String, String>> {
    let stored = load_stored_from(conn)?;
    let mut out = BTreeMap::new();
    for key in SOURCE_KEYS {
        let value = stored
            .get(key)
            .cloned()
            .unwrap_or_else(|| default_value(key).expect("已知来源 key").to_string());
        out.insert(key.to_string(), value);
    }
    Ok(out)
}

/// 只读读取三个来源的当前文本；无 DB 时返回纯默认，零写。
pub fn current_sources_readonly(ctx: &Context) -> anyhow::Result<BTreeMap<String, String>> {
    match store::open_readonly(ctx)? {
        Some(conn) => current_sources_from(&conn),
        None => {
            let mut out = BTreeMap::new();
            for key in SOURCE_KEYS {
                out.insert(
                    key.to_string(),
                    default_value(key).expect("已知来源 key").to_string(),
                );
            }
            Ok(out)
        }
    }
}

fn source_item(key: &str, value: &str) -> Value {
    json!({
        "key": key,
        "label": label_of(key),
        "value": value,
        "revision": revision_of(value),
        "format": "text",
    })
}

struct SourcesArgs {
    key: Option<String>,
    value: Option<String>,
    dry_run: bool,
}

/// 严格参数解析：只接受 --key/--value/--dry-run，拒绝位置参数、未知或重复
/// flag、缺值与以 `--` 开头的值。解析错误一律使用不含参数内容的固定消息，
/// 避免误传的规则正文经错误包络与诊断日志泄漏。
fn parse_args(req: &Request) -> Result<SourcesArgs, DispatchError> {
    let mut seen: HashSet<&str> = HashSet::new();
    let mut key: Option<String> = None;
    let mut value: Option<String> = None;
    let mut dry_run = false;

    let mut i = 0;
    while i < req.args.len() {
        let token = &req.args[i];
        if !token.starts_with("--") {
            return Err(source_error("sources 参数错误：不接受位置参数"));
        }
        let flag = token.as_str();
        if !seen.insert(flag) {
            return Err(source_error("sources 参数错误：重复提供同一参数"));
        }
        match flag {
            "--dry-run" => {
                dry_run = true;
                i += 1;
            }
            "--key" | "--value" => {
                i += 1;
                let val = req
                    .args
                    .get(i)
                    .ok_or_else(|| source_error(format!("参数 '{flag}' 缺少参数值")))?;
                if val.starts_with("--") {
                    return Err(source_error(format!("参数 '{flag}' 缺少参数值")));
                }
                if flag == "--key" {
                    key = Some(val.clone());
                } else {
                    value = Some(val.clone());
                }
                i += 1;
            }
            _ => {
                return Err(source_error(
                    "sources 参数错误：未知选项（只接受 --key/--value/--dry-run）",
                ))
            }
        }
    }
    Ok(SourcesArgs {
        key,
        value,
        dry_run,
    })
}

pub(crate) fn get(ctx: &Context, req: &Request) -> DispatchResult {
    let args = parse_args(req)?;
    if args.value.is_some() {
        return Err(source_error("sources:get 不接受 --value"));
    }
    if args.dry_run {
        return Err(source_error("sources:get 不接受 --dry-run"));
    }
    let key = args.key.ok_or_else(|| {
        source_error("sources:get 需要 --key <rules|rules.shared|rules.claude|rules.codex>")
    })?;
    let keys: Vec<&str> = match key.as_str() {
        "rules" => SOURCE_KEYS.to_vec(),
        exact if SOURCE_KEYS.contains(&exact) => vec![exact],
        other => {
            return Err(source_error(format!(
                "未知来源 key '{other}'（支持 rules / {}）",
                SOURCE_KEYS.join(", ")
            )))
        }
    };
    let sources = current_sources_readonly(ctx).map_err(internal)?;
    let items: Vec<Value> = keys
        .iter()
        .map(|key| source_item(key, &sources[*key]))
        .collect();
    Ok((json!({ "sources": items }), vec![]))
}

/// 解析并校验 {revision, value} wrapper：仅这两个字段、均为字符串、
/// 不含 NUL、不命中凭据检测。错误不回显规则正文或疑似凭据内容。
fn parse_wrapper(raw: &str) -> Result<(String, String), DispatchError> {
    let wrapper: Value = serde_json::from_str(raw)
        .map_err(|err| source_error(format!("--value 不是合法 JSON: {err}")))?;
    let obj = wrapper
        .as_object()
        .ok_or_else(|| source_error("--value 必须是含 revision 与 value 的 JSON 对象"))?;
    for field in obj.keys() {
        if field != "revision" && field != "value" {
            return Err(source_error(
                "--value 对象只允许 revision 与 value 字段，存在额外字段",
            ));
        }
    }
    let revision = obj
        .get("revision")
        .and_then(Value::as_str)
        .ok_or_else(|| source_error("--value.revision 缺失或不是字符串"))?;
    let value = obj
        .get("value")
        .and_then(Value::as_str)
        .ok_or_else(|| source_error("--value.value 缺失或不是字符串"))?;
    if revision.contains('\0') || value.contains('\0') {
        return Err(source_error("来源内容不允许包含 NUL 字符"));
    }
    if let Some(kind) = reject_secret_text(value) {
        return Err(source_error(format!("拒绝写入疑似凭据: {kind}")));
    }
    Ok((revision.to_string(), value.to_string()))
}

fn current_revision_in(conn: &Connection, key: &str) -> anyhow::Result<String> {
    let stored = load_stored_from(conn)?;
    let current = stored
        .get(key)
        .cloned()
        .unwrap_or_else(|| default_value(key).expect("已知来源 key").to_string());
    Ok(revision_of(&current))
}

pub(crate) fn set(ctx: &Context, req: &Request) -> DispatchResult {
    let args = parse_args(req)?;
    let key = args.key.ok_or_else(|| {
        source_error("sources:set 需要 --key <rules.shared|rules.claude|rules.codex>")
    })?;
    if !SOURCE_KEYS.contains(&key.as_str()) {
        return Err(source_error(format!(
            "sources:set 只接受精确 key（{}），得到 '{key}'",
            SOURCE_KEYS.join(", ")
        )));
    }
    let raw_value = args
        .value
        .ok_or_else(|| source_error("sources:set 需要 --value <{\"revision\":..,\"value\":..}>"))?;
    let (revision, new_value) = parse_wrapper(&raw_value)?;

    // 预检（只读）：revision CAS 基于当前有效文本。
    let current_revision = current_sources_readonly(ctx)
        .map_err(internal)?
        .get(&key)
        .cloned()
        .map(|v| revision_of(&v));
    let Some(current_revision) = current_revision else {
        return Err(source_error("来源 key 缺失"));
    };
    if revision != current_revision {
        return Err(source_error(format!(
            "来源 '{key}' 的 revision 已过期，请重新读取后再保存"
        )));
    }

    let new_revision = revision_of(&new_value);
    if args.dry_run {
        return Ok((
            json!({ "sources": [source_item(&key, &new_value)] }),
            vec![diagnostic(
                "DRY_RUN",
                None,
                format!("dry-run：未写入 {key}"),
            )],
        ));
    }

    let mut conn = store::open_write(ctx).map_err(internal)?;
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(internal)?;
    ensure_table(&tx).map_err(internal)?;
    // 事务内复查 CAS：预检与写入之间来源可能被其他写入方改动。
    let in_tx_revision = current_revision_in(&tx, &key).map_err(internal)?;
    if revision != in_tx_revision {
        return Err(source_error(format!(
            "来源 '{key}' 的 revision 已过期，请重新读取后再保存"
        )));
    }
    tx.execute(
        "INSERT INTO rule_sources (key, value, revision, updated_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, revision = excluded.revision, updated_at = excluded.updated_at",
        rusqlite::params![key, new_value, new_revision, now_iso()],
    )
    .map_err(internal)?;
    store::record_operation(
        &tx,
        Some(&req.run_id),
        "sources:set",
        Some(&key),
        Some(&json!({ "key": key, "revision": new_revision })),
    )
    .map_err(internal)?;
    tx.commit().map_err(internal)?;

    Ok((
        json!({ "sources": [source_item(&key, &new_value)] }),
        vec![],
    ))
}
