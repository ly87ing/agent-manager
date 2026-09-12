//! sources 域：rules 来源文本（rules.shared / rules.claude / rules.codex）与
//! hooks 来源 JSON（hooks.claude / hooks.codex）读写。
//!
//! native-manager.db 的 rule_sources / hook_sources 表是来源的唯一可写事实源；
//! 编译期 include_str 固化只读默认（hooks 默认来自 config/claude-hooks.json 与
//! config/codex-hooks.json，去掉 settings_path/hooks_path 后固化），运行时不读
//! 源码树。读取（含缺失 DB/缺表）只返回默认且零写；真实保存使用 Immediate
//! 事务 + revision CAS，审计仅记录 key/revision 等无正文元数据；保存定义绝不
//! 写助手文件或自动 apply。本域错误码统一 SOURCE_ERROR，且不使用共享 apply lock。

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

pub const KEY_HOOKS_CLAUDE: &str = "hooks.claude";
pub const KEY_HOOKS_CODEX: &str = "hooks.codex";

/// hooks 来源固定 key，稳定顺序：claude 在前，codex 在后。
pub const HOOK_SOURCE_KEYS: [&str; 2] = [KEY_HOOKS_CLAUDE, KEY_HOOKS_CODEX];

const DEFAULT_SHARED: &str = include_str!("../../agent-rule/rules/src/shared/core.md");
const DEFAULT_CLAUDE: &str = include_str!("../../agent-rule/rules/src/agents/claude/adapter.md");
const DEFAULT_CODEX: &str = include_str!("../../agent-rule/rules/src/agents/codex/adapter.md");

const RAW_DEFAULT_HOOKS_CLAUDE: &str = include_str!("../../config/claude-hooks.json");
const RAW_DEFAULT_HOOKS_CODEX: &str = include_str!("../../config/codex-hooks.json");

const TABLE_DDL: &str = "CREATE TABLE IF NOT EXISTS rule_sources (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    revision TEXT NOT NULL,
    updated_at TEXT NOT NULL
)";

const HOOK_TABLE_DDL: &str = "CREATE TABLE IF NOT EXISTS hook_sources (
    key TEXT PRIMARY KEY NOT NULL,
    value_json TEXT NOT NULL,
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
        KEY_HOOKS_CLAUDE => "Claude Hooks",
        KEY_HOOKS_CODEX => "Codex Hooks",
        _ => "未知来源",
    }
}

/// 来源 revision：rules 为当前有效文本的 sha256；hooks 为 canonical
/// compact JSON 的 64 位小写 sha256。
pub fn revision_of(text: &str) -> String {
    sha256_hex(text)
}

/// 递归 canonicalization：每个 JSON object 按 key 字典序重建（显式排序，
/// 不依赖 preserve_order 下的输入键序），array 保持元素顺序但递归处理
/// 元素，scalar 原样。
fn canonicalize_json(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<(&String, &Value)> = map.iter().collect();
            entries.sort_by(|a, b| a.0.cmp(b.0));
            let mut out = serde_json::Map::new();
            for (key, item) in entries {
                out.insert(key.clone(), canonicalize_json(item));
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(canonicalize_json).collect()),
        other => other.clone(),
    }
}

/// hooks 默认来源：去掉 settings_path/hooks_path 后的 canonical compact JSON。
fn default_hook_source(key: &str) -> Option<String> {
    let raw = match key {
        KEY_HOOKS_CLAUDE => RAW_DEFAULT_HOOKS_CLAUDE,
        KEY_HOOKS_CODEX => RAW_DEFAULT_HOOKS_CODEX,
        _ => return None,
    };
    let mut obj: serde_json::Map<String, Value> =
        serde_json::from_str(raw).expect("内置 hooks 默认是合法 JSON 对象");
    obj.remove("settings_path");
    obj.remove("hooks_path");
    Some(canonical_hook_json(&obj))
}

fn canonical_hook_json(obj: &serde_json::Map<String, Value>) -> String {
    serde_json::to_string(&canonicalize_json(&Value::Object(obj.clone()))).expect("compact JSON")
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

/// hook_sources 精确 4 列 schema 校验；错误只含类别，不回显 DB 原值。
fn check_hook_table_schema(conn: &Connection) -> anyhow::Result<()> {
    let mut stmt = conn.prepare("PRAGMA table_info(hook_sources)")?;
    let cols: Vec<(String, String, bool, i64)> = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)? != 0,
                row.get::<_, i64>(5)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let expected: [(&str, i64); 4] = [
        ("key", 1),
        ("value_json", 0),
        ("revision", 0),
        ("updated_at", 0),
    ];
    if cols.len() != expected.len() {
        anyhow::bail!(
            "hook_sources 表结构损坏：期望 {} 列，实际 {} 列",
            expected.len(),
            cols.len()
        );
    }
    for (idx, ((name, decl, notnull, pk), (_, want_pk))) in
        cols.iter().zip(expected.iter()).enumerate()
    {
        if *name != expected[idx].0 {
            anyhow::bail!("hook_sources 表结构损坏：列顺序/名称不符");
        }
        if !decl.eq_ignore_ascii_case("TEXT") {
            anyhow::bail!("hook_sources 表结构损坏：存在列类型不是 TEXT");
        }
        if !notnull {
            anyhow::bail!("hook_sources 表结构损坏：存在列缺少 NOT NULL");
        }
        if pk != want_pk {
            anyhow::bail!("hook_sources 表结构损坏：主键定义不符");
        }
    }
    Ok(())
}

/// 校验单行：固定 key、value_json 可解析为对象且通过来源结构校验、且
/// 必须等于自身递归 canonical 字符串；revision 为 64 位小写 sha256 且等于
/// value_json 的 sha256；updated_at 非空。错误不回显行内容。
fn validate_hook_row(
    key: &str,
    value_json: &str,
    revision: &str,
    updated_at: &str,
) -> anyhow::Result<()> {
    if !HOOK_SOURCE_KEYS.contains(&key) {
        anyhow::bail!("hook_sources 存在未知 key 行");
    }
    let parsed: Value = serde_json::from_str(value_json)
        .map_err(|_| anyhow::anyhow!("hook_sources 行 value_json 不是合法 JSON"))?;
    let obj = parsed
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("hook_sources 行 value_json 不是 JSON 对象"))?;
    crate::native_hooks::model::validate_source(key, obj)
        .map_err(|err| anyhow::anyhow!("hook_sources 行结构非法: {err:#}"))?;
    if canonical_hook_json(obj) != value_json {
        anyhow::bail!("hook_sources 行 value_json 不是 canonical 形式");
    }
    if !crate::native_mcp::model::is_sha256_hex(revision) {
        anyhow::bail!("hook_sources 行 revision 不是 64 位小写 sha256");
    }
    if revision_of(value_json) != revision {
        anyhow::bail!("hook_sources 行 revision 与 value_json 不一致");
    }
    if updated_at.is_empty() {
        anyhow::bail!("hook_sources 行 updated_at 不能为空");
    }
    Ok(())
}

/// 读取已保存的 hooks 来源（canonical compact JSON）；缺表视为空；
/// 表结构或行损坏严格 fail-closed（绝不 panic、不回显 DB 原值）。
fn load_hook_stored_from(conn: &Connection) -> anyhow::Result<BTreeMap<String, String>> {
    let mut stmt =
        match conn.prepare("SELECT key, value_json, revision, updated_at FROM hook_sources") {
            Ok(stmt) => stmt,
            Err(exc) => {
                if exc.to_string().contains("no such table") {
                    return Ok(BTreeMap::new());
                }
                return Err(exc.into());
            }
        };
    check_hook_table_schema(conn)?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
        ))
    })?;
    let mut out = BTreeMap::new();
    for row in rows {
        let (key, value_json, revision, updated_at) = row?;
        validate_hook_row(&key, &value_json, &revision, &updated_at)?;
        out.insert(key, value_json);
    }
    Ok(out)
}

/// 由连接读出两个 hooks 来源的当前 canonical compact JSON（已存值覆盖默认）。
pub fn current_hook_sources_from(conn: &Connection) -> anyhow::Result<BTreeMap<String, String>> {
    let stored = load_hook_stored_from(conn)?;
    let mut out = BTreeMap::new();
    for key in HOOK_SOURCE_KEYS {
        let value = stored
            .get(key)
            .cloned()
            .unwrap_or_else(|| default_hook_source(key).expect("已知 hooks 来源 key"));
        out.insert(key.to_string(), value);
    }
    Ok(out)
}

/// 只读读取 hooks 来源；无 DB 时返回纯默认，零写。
pub fn current_hook_sources_readonly(ctx: &Context) -> anyhow::Result<BTreeMap<String, String>> {
    match store::open_readonly(ctx)? {
        Some(conn) => current_hook_sources_from(&conn),
        None => {
            let mut out = BTreeMap::new();
            for key in HOOK_SOURCE_KEYS {
                out.insert(
                    key.to_string(),
                    default_hook_source(key).expect("已知 hooks 来源 key"),
                );
            }
            Ok(out)
        }
    }
}

fn hook_source_item(key: &str, canonical: &str) -> anyhow::Result<Value> {
    let value: Value = serde_json::from_str(canonical)
        .map_err(|_| anyhow::anyhow!("hook_sources 行 value_json 不是合法 JSON"))?;
    Ok(json!({
        "key": key,
        "label": label_of(key),
        "value": value,
        "revision": revision_of(canonical),
        "format": "json",
    }))
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
        source_error(
            "sources:get 需要 --key <rules|hooks|rules.shared|rules.claude|rules.codex|hooks.claude|hooks.codex>",
        )
    })?;
    match key.as_str() {
        "rules" => {
            let sources = current_sources_readonly(ctx).map_err(internal)?;
            let items: Vec<Value> = SOURCE_KEYS
                .iter()
                .map(|key| source_item(key, &sources[*key]))
                .collect();
            Ok((json!({ "sources": items }), vec![]))
        }
        exact if SOURCE_KEYS.contains(&exact) => {
            let sources = current_sources_readonly(ctx).map_err(internal)?;
            Ok((
                json!({ "sources": [source_item(exact, &sources[exact])] }),
                vec![],
            ))
        }
        "hooks" => {
            let sources = current_hook_sources_readonly(ctx).map_err(internal)?;
            let mut items = Vec::new();
            for key in HOOK_SOURCE_KEYS {
                items.push(hook_source_item(key, &sources[key]).map_err(internal)?);
            }
            Ok((json!({ "sources": items }), vec![]))
        }
        exact if HOOK_SOURCE_KEYS.contains(&exact) => {
            let sources = current_hook_sources_readonly(ctx).map_err(internal)?;
            Ok((
                json!({ "sources": [hook_source_item(exact, &sources[exact]).map_err(internal)?] }),
                vec![],
            ))
        }
        _ => Err(source_error(
            "未知来源 key（支持 rules / hooks 前缀与精确 key）",
        )),
    }
}

/// 解析并校验 {revision, value} wrapper：仅这两个字段；revision 必须是
/// 字符串，value 类型按 key 域另行约束。错误不回显正文或疑似凭据内容。
fn parse_wrapper(raw: &str) -> Result<(String, Value), DispatchError> {
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
        .cloned()
        .ok_or_else(|| source_error("--value.value 缺失"))?;
    if revision.contains('\0') {
        return Err(source_error("来源内容不允许包含 NUL 字符"));
    }
    Ok((revision.to_string(), value))
}

/// rules 文本值的既有校验：字符串、无 NUL、不命中凭据检测。
fn validate_rules_value(value: &Value) -> Result<String, DispatchError> {
    let text = value
        .as_str()
        .ok_or_else(|| source_error("--value.value 缺失或不是字符串"))?;
    if text.contains('\0') {
        return Err(source_error("来源内容不允许包含 NUL 字符"));
    }
    if let Some(kind) = reject_secret_text(text) {
        return Err(source_error(format!("拒绝写入疑似凭据: {kind}")));
    }
    Ok(text.to_string())
}

/// 递归检查 JSON 字符串叶是否含 NUL。
fn json_contains_nul(value: &Value) -> bool {
    match value {
        Value::String(s) => s.contains('\0'),
        Value::Array(items) => items.iter().any(json_contains_nul),
        Value::Object(map) => map
            .iter()
            .any(|(key, item)| key.contains('\0') || json_contains_nul(item)),
        _ => false,
    }
}

/// hooks 对象值校验：必须是对象、无 NUL、结构合法、不命中凭据检测；
/// 返回 canonical compact JSON。错误不回显 source JSON/command/marker。
fn validate_hooks_value(key: &str, value: &Value) -> Result<String, DispatchError> {
    let obj = value
        .as_object()
        .ok_or_else(|| source_error("--value.value 必须是 JSON 对象"))?;
    if json_contains_nul(value) {
        return Err(source_error("来源内容不允许包含 NUL 字符"));
    }
    crate::native_hooks::model::validate_source(key, obj)
        .map_err(|err| source_error(format!("hooks 来源结构非法: {err:#}")))?;
    let canonical = canonical_hook_json(obj);
    if let Some(kind) = reject_secret_text(&canonical) {
        return Err(source_error(format!("拒绝写入疑似凭据: {kind}")));
    }
    Ok(canonical)
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
    let key = args
        .key
        .ok_or_else(|| source_error("sources:set 需要 --key <rules.*|hooks.claude|hooks.codex>"))?;
    let is_rules_key = SOURCE_KEYS.contains(&key.as_str());
    let is_hooks_key = HOOK_SOURCE_KEYS.contains(&key.as_str());
    if !is_rules_key && !is_hooks_key {
        return Err(source_error(
            "sources:set 只接受精确 key（rules.* 或 hooks.* 固定 key）",
        ));
    }
    let raw_value = args
        .value
        .ok_or_else(|| source_error("sources:set 需要 --value <{\"revision\":..,\"value\":..}>"))?;
    let (revision, value) = parse_wrapper(&raw_value)?;

    // 预检（只读）：revision CAS 基于当前有效来源。
    let current_revision = if is_rules_key {
        current_sources_readonly(ctx)
            .map_err(internal)?
            .get(&key)
            .map(|v| revision_of(v))
    } else {
        current_hook_sources_readonly(ctx)
            .map_err(internal)?
            .get(&key)
            .map(|v| revision_of(v))
    };
    let Some(current_revision) = current_revision else {
        return Err(source_error("来源 key 缺失"));
    };
    if revision != current_revision {
        return Err(source_error("来源 revision 已过期，请重新读取后再保存"));
    }

    if is_rules_key {
        let new_value = validate_rules_value(&value)?;
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
            return Err(source_error("来源 revision 已过期，请重新读取后再保存"));
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
        return Ok((
            json!({ "sources": [source_item(&key, &new_value)] }),
            vec![],
        ));
    }

    let new_value = validate_hooks_value(&key, &value)?;
    let new_revision = revision_of(&new_value);
    if args.dry_run {
        return Ok((
            json!({ "sources": [hook_source_item(&key, &new_value).map_err(internal)?] }),
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
    tx.execute_batch(HOOK_TABLE_DDL).map_err(internal)?;
    // 事务内复查 CAS。
    let in_tx_revision = current_hook_revision_in(&tx, &key).map_err(internal)?;
    if revision != in_tx_revision {
        return Err(source_error("来源 revision 已过期，请重新读取后再保存"));
    }
    tx.execute(
        "INSERT INTO hook_sources (key, value_json, revision, updated_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, revision = excluded.revision, updated_at = excluded.updated_at",
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
        json!({ "sources": [hook_source_item(&key, &new_value).map_err(internal)?] }),
        vec![],
    ))
}

fn current_hook_revision_in(conn: &Connection, key: &str) -> anyhow::Result<String> {
    let stored = load_hook_stored_from(conn)?;
    let current = stored
        .get(key)
        .cloned()
        .unwrap_or_else(|| default_hook_source(key).expect("已知 hooks 来源 key"));
    Ok(revision_of(&current))
}
