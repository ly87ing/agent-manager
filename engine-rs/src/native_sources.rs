//! sources 域：aliases 来源 JSON（aliases.shell）读写。
//!
//! 规则来源（rules.*）与 hooks 来源（hooks.*）已退役：生效规则与 hooks 以各助手的实际
//! 配置为准，分别由 rules / hooks 域直接编辑；旧 rule_sources 在写路径迁移成模板，旧
//! hook_sources 在 hooks 页以「旧版已保存的 hooks（未生效）」建议出现（见 native_hooks::legacy）。
//!
//! native-manager.db 的 alias_sources 表是来源的唯一可写事实源；编译期 include_str
//! 固化只读默认（config/shell-aliases.json），运行时不读源码树。读取（含缺失 DB/缺表）
//! 只返回默认且零写；真实保存使用 Immediate 事务 + revision CAS，审计仅记录
//! key/revision 等无正文元数据；保存定义绝不写助手文件或自动 apply。本域错误码统一
//! SOURCE_ERROR，且不使用共享 apply lock。

use std::collections::BTreeMap;
use std::collections::HashSet;

use rusqlite::{Connection, TransactionBehavior};
use serde_json::{json, Value};

use crate::native::{reject_secret_text, Context, DispatchError, DispatchResult, Request};
use crate::native_mcp::model::{now_iso, sha256_hex};
use crate::store;

pub const KEY_ALIAS_SHELL: &str = "aliases.shell";

/// aliases 来源固定 key（单键）。
pub const ALIAS_SOURCE_KEYS: [&str; 1] = [KEY_ALIAS_SHELL];

const RAW_DEFAULT_ALIAS_SHELL: &str = include_str!("../../config/shell-aliases.json");

const ALIAS_TABLE_DDL: &str = "CREATE TABLE IF NOT EXISTS alias_sources (
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

pub fn label_of(key: &str) -> &'static str {
    match key {
        KEY_ALIAS_SHELL => "Shell 别名",
        _ => "未知来源",
    }
}

/// 来源 revision：canonical compact JSON 的 64 位小写 sha256。
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

/// 对象级 canonical compact JSON（递归 key 字典序），供 aliases 与旧 hooks 来源共用。
pub(crate) fn canonical_json(obj: &serde_json::Map<String, Value>) -> String {
    serde_json::to_string(&canonicalize_json(&Value::Object(obj.clone()))).expect("compact JSON")
}

/// aliases.shell 默认来源：config/shell-aliases.json 原样 canonical 化
/// （保留 _comment 文档字段；渲染侧只消费 timezone/lang/aliases）。
fn default_alias_source(key: &str) -> Option<String> {
    if key != KEY_ALIAS_SHELL {
        return None;
    }
    let obj: serde_json::Map<String, Value> =
        serde_json::from_str(RAW_DEFAULT_ALIAS_SHELL).expect("内置 aliases 默认是合法 JSON 对象");
    Some(canonical_json(&obj))
}

/// 事务内直接写入一条来源（供配置备份恢复用）。
///
/// 与 `sources:set` 的区别只在于不做 revision 比对：恢复备份是整份替换，
/// 本机当前内容按定义就要被覆盖。结构校验仍由调用方在写入前完成。
pub(crate) fn write_source_in_tx(
    tx: &rusqlite::Transaction,
    key: &str,
    text: &str,
) -> anyhow::Result<()> {
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let revision = revision_of(text);
    if ALIAS_SOURCE_KEYS.contains(&key) {
        tx.execute_batch(ALIAS_TABLE_DDL)?;
        tx.execute(
            "INSERT INTO alias_sources (key, value_json, revision, updated_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, revision = excluded.revision, updated_at = excluded.updated_at",
            rusqlite::params![key, text, revision, now],
        )?;
        return Ok(());
    }
    anyhow::bail!("未知来源 key，拒绝写入")
}

/// alias_sources 精确 4 列 schema 校验；错误只含类别，不回显 DB 原值。
fn check_alias_table_schema(conn: &Connection) -> anyhow::Result<()> {
    let mut stmt = conn.prepare("PRAGMA table_info(alias_sources)")?;
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
            "alias_sources 表结构损坏：期望 {} 列，实际 {} 列",
            expected.len(),
            cols.len()
        );
    }
    for (idx, ((name, decl, notnull, pk), (_, want_pk))) in
        cols.iter().zip(expected.iter()).enumerate()
    {
        if *name != expected[idx].0 {
            anyhow::bail!("alias_sources 表结构损坏：列顺序/名称不符");
        }
        if !decl.eq_ignore_ascii_case("TEXT") {
            anyhow::bail!("alias_sources 表结构损坏：存在列类型不是 TEXT");
        }
        if !notnull {
            anyhow::bail!("alias_sources 表结构损坏：存在列缺少 NOT NULL");
        }
        if pk != want_pk {
            anyhow::bail!("alias_sources 表结构损坏：主键定义不符");
        }
    }
    Ok(())
}

/// 校验单行：固定 key、value_json 可解析为对象且通过 aliases 来源结构校验、
/// 且必须等于自身递归 canonical 字符串；revision 为 64 位小写 sha256 且等于
/// value_json 的 sha256；updated_at 非空。错误不回显行内容。
fn validate_alias_row(
    key: &str,
    value_json: &str,
    revision: &str,
    updated_at: &str,
) -> anyhow::Result<()> {
    if !ALIAS_SOURCE_KEYS.contains(&key) {
        anyhow::bail!("alias_sources 存在未知 key 行");
    }
    let parsed: Value = serde_json::from_str(value_json)
        .map_err(|_| anyhow::anyhow!("alias_sources 行 value_json 不是合法 JSON"))?;
    let obj = parsed
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("alias_sources 行 value_json 不是 JSON 对象"))?;
    crate::native_aliases::model::validate_source(obj)
        .map_err(|err| anyhow::anyhow!("alias_sources 行结构非法: {err:#}"))?;
    if canonical_json(obj) != value_json {
        anyhow::bail!("alias_sources 行 value_json 不是 canonical 形式");
    }
    if !crate::native_mcp::model::is_sha256_hex(revision) {
        anyhow::bail!("alias_sources 行 revision 不是 64 位小写 sha256");
    }
    if revision_of(value_json) != revision {
        anyhow::bail!("alias_sources 行 revision 与 value_json 不一致");
    }
    if updated_at.is_empty() {
        anyhow::bail!("alias_sources 行 updated_at 不能为空");
    }
    Ok(())
}

/// 读取已保存的 aliases 来源（canonical compact JSON）；缺表视为空；
/// 表结构或行损坏严格 fail-closed（绝不 panic、不回显 DB 原值）。
fn load_alias_stored_from(conn: &Connection) -> anyhow::Result<BTreeMap<String, String>> {
    let mut stmt =
        match conn.prepare("SELECT key, value_json, revision, updated_at FROM alias_sources") {
            Ok(stmt) => stmt,
            Err(exc) => {
                if exc.to_string().contains("no such table") {
                    return Ok(BTreeMap::new());
                }
                return Err(exc.into());
            }
        };
    check_alias_table_schema(conn)?;
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
        validate_alias_row(&key, &value_json, &revision, &updated_at)?;
        out.insert(key, value_json);
    }
    Ok(out)
}

/// 由连接读出 aliases.shell 当前 canonical compact JSON（已存值覆盖默认）。
pub fn current_alias_source_from(conn: &Connection) -> anyhow::Result<String> {
    let stored = load_alias_stored_from(conn)?;
    Ok(stored
        .get(KEY_ALIAS_SHELL)
        .cloned()
        .unwrap_or_else(|| default_alias_source(KEY_ALIAS_SHELL).expect("已知 aliases 来源 key")))
}

/// 只读读取 aliases.shell 当前内容；无 DB 时返回纯默认，零写。
pub fn current_alias_source_readonly(ctx: &Context) -> anyhow::Result<String> {
    match store::open_readonly(ctx)? {
        Some(conn) => current_alias_source_from(&conn),
        None => Ok(default_alias_source(KEY_ALIAS_SHELL).expect("已知 aliases 来源 key")),
    }
}

fn current_alias_revision_in(conn: &Connection, key: &str) -> anyhow::Result<String> {
    let stored = load_alias_stored_from(conn)?;
    let current = stored
        .get(key)
        .cloned()
        .unwrap_or_else(|| default_alias_source(key).expect("已知 aliases 来源 key"));
    Ok(revision_of(&current))
}

fn alias_source_item(key: &str, canonical: &str) -> anyhow::Result<Value> {
    let value: Value = serde_json::from_str(canonical)
        .map_err(|_| anyhow::anyhow!("alias_sources 行 value_json 不是合法 JSON"))?;
    Ok(json!({
        "key": key,
        "label": label_of(key),
        "value": value,
        "revision": revision_of(canonical),
        "format": "json",
    }))
}

struct SourcesArgs {
    key: Option<String>,
    value: Option<String>,
}

/// 严格参数解析：只接受 --key/--value，拒绝位置参数、未知或重复 flag、缺值
/// 与以 `--` 开头的值（已退役的 --dry-run 按未知选项一律拒绝）。解析错误一律
/// 使用不含参数内容的固定消息，避免误传的规则正文经错误包络泄漏。
fn parse_args(req: &Request) -> Result<SourcesArgs, DispatchError> {
    let mut seen: HashSet<&str> = HashSet::new();
    let mut key: Option<String> = None;
    let mut value: Option<String> = None;

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
                    "sources 参数错误：未知选项（只接受 --key/--value）",
                ))
            }
        }
    }
    Ok(SourcesArgs { key, value })
}

/// 已退役的规则来源键：rules 前缀与 rules.* 精确键。
fn is_retired_rules_key(key: &str) -> bool {
    key == "rules" || key.starts_with("rules.")
}

const RULES_RETIRED: &str =
    "规则来源已退役：生效规则以各助手的规则文件为准，请在规则页直接编辑或使用规则模板";

/// 已退役的 hooks 来源键：hooks 前缀与 hooks.* 精确键。
fn is_retired_hooks_key(key: &str) -> bool {
    key == "hooks" || key.starts_with("hooks.")
}

const HOOKS_RETIRED: &str =
    "Hooks 来源已退役：生效 hooks 以各助手的实际配置为准，请在 Hooks 页直接编辑";

pub(crate) fn get(ctx: &Context, req: &Request) -> DispatchResult {
    let args = parse_args(req)?;
    if args.value.is_some() {
        return Err(source_error("sources:get 不接受 --value"));
    }
    let key = args
        .key
        .ok_or_else(|| source_error("sources:get 需要 --key <aliases|aliases.shell>"))?;
    match key.as_str() {
        retired if is_retired_rules_key(retired) => Err(source_error(RULES_RETIRED)),
        retired if is_retired_hooks_key(retired) => Err(source_error(HOOKS_RETIRED)),
        "aliases" => {
            let canonical = current_alias_source_readonly(ctx).map_err(internal)?;
            Ok((
                json!({ "sources": [alias_source_item(KEY_ALIAS_SHELL, &canonical).map_err(internal)?] }),
                vec![],
            ))
        }
        exact if ALIAS_SOURCE_KEYS.contains(&exact) => {
            let canonical = current_alias_source_readonly(ctx).map_err(internal)?;
            Ok((
                json!({ "sources": [alias_source_item(exact, &canonical).map_err(internal)?] }),
                vec![],
            ))
        }
        _ => Err(source_error("未知来源 key（支持 aliases 前缀与精确 key）")),
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

/// aliases 对象值校验：必须是对象、无 NUL、alias 名/值合法（防空格/引号
/// 注入）、不命中凭据检测；返回 canonical compact JSON。错误不回显正文。
fn validate_alias_value(key: &str, value: &Value) -> Result<String, DispatchError> {
    debug_assert_eq!(key, KEY_ALIAS_SHELL);
    let obj = value
        .as_object()
        .ok_or_else(|| source_error("--value.value 必须是 JSON 对象"))?;
    if json_contains_nul(value) {
        return Err(source_error("来源内容不允许包含 NUL 字符"));
    }
    crate::native_aliases::model::validate_source(obj)
        .map_err(|err| source_error(format!("aliases 来源结构非法: {err:#}")))?;
    let canonical = canonical_json(obj);
    if let Some(kind) = reject_secret_text(&canonical) {
        return Err(source_error(format!("拒绝写入疑似凭据: {kind}")));
    }
    Ok(canonical)
}

pub(crate) fn set(ctx: &Context, req: &Request) -> DispatchResult {
    let args = parse_args(req)?;
    let key = args
        .key
        .ok_or_else(|| source_error("sources:set 需要 --key aliases.shell"))?;
    if is_retired_rules_key(&key) {
        return Err(source_error(RULES_RETIRED));
    }
    if is_retired_hooks_key(&key) {
        return Err(source_error(HOOKS_RETIRED));
    }
    if !ALIAS_SOURCE_KEYS.contains(&key.as_str()) {
        return Err(source_error(
            "sources:set 只接受精确 key（aliases.shell 固定 key）",
        ));
    }
    let raw_value = args
        .value
        .ok_or_else(|| source_error("sources:set 需要 --value <{\"revision\":..,\"value\":..}>"))?;
    let (revision, value) = parse_wrapper(&raw_value)?;

    // 预检（只读）：revision CAS 基于当前有效来源。
    let current_revision = revision_of(&current_alias_source_readonly(ctx).map_err(internal)?);
    if revision != current_revision {
        return Err(source_error("来源 revision 已过期，请重新读取后再保存"));
    }

    let new_value = validate_alias_value(&key, &value)?;
    let new_revision = revision_of(&new_value);
    let mut conn = store::open_write(ctx).map_err(internal)?;
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(internal)?;
    tx.execute_batch(ALIAS_TABLE_DDL).map_err(internal)?;
    // 事务内复查 CAS。
    let in_tx_revision = current_alias_revision_in(&tx, &key).map_err(internal)?;
    if revision != in_tx_revision {
        return Err(source_error("来源 revision 已过期，请重新读取后再保存"));
    }
    tx.execute(
        "INSERT INTO alias_sources (key, value_json, revision, updated_at) VALUES (?1, ?2, ?3, ?4)
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
        json!({ "sources": [alias_source_item(&key, &new_value).map_err(internal)?] }),
        vec![],
    ))
}
