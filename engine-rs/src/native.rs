//! Native dispatcher: one entry point shared by the CLI and the Tauri bridge.
//!
//! Domain ownership: core (this module) implements policy/db/runs. Domain
//! modules are delivered by their owners as `crate::native_<domain>::handle`
//! with signature `fn handle(&Context, &Request) -> anyhow::Result<Value>`;
//! until registered they answer DOMAIN_NOT_IMPLEMENTED (never fake success).

use std::path::PathBuf;

use anyhow::Result;
use rusqlite::TransactionBehavior;
use serde_json::{json, Map, Value};

use crate::envelope::{diagnostic, Diagnostic, Envelope, Lock, Mode};
use crate::store;

/// Frozen fields (design review): data_dir = app data dir (native-manager.db
/// lives here); home_dir = user home for domain targets.
pub struct Context {
    pub data_dir: PathBuf,
    pub home_dir: PathBuf,
}

/// Frozen fields: command is "<domain>:<verb>" (a single '.' is also accepted),
/// args is the raw flag list (["--only", "rules:x", "--dry-run", ...]),
/// run_id is provided by the caller (GUI run id or CLI-generated).
pub struct Request {
    pub command: String,
    pub args: Vec<String>,
    pub run_id: String,
}

impl Request {
    pub fn flag(&self, name: &str) -> bool {
        self.args.iter().any(|arg| arg == name)
    }

    pub fn value(&self, name: &str) -> Option<&str> {
        let mut iter = self.args.iter();
        while let Some(arg) = iter.next() {
            if arg == name {
                return iter.next().map(String::as_str);
            }
        }
        None
    }

    pub fn values(&self, name: &str) -> Vec<&str> {
        let mut out = Vec::new();
        let mut iter = self.args.iter();
        while let Some(arg) = iter.next() {
            if arg == name {
                if let Some(value) = iter.next() {
                    out.push(value.as_str());
                }
            }
        }
        out
    }
}

const KNOWN_DOMAINS: &[&str] = &[
    "agents",
    "mcp",
    "skills",
    "rules",
    "hooks",
    "tools",
    "aliases",
    "secrets",
    "profile",
    "policy",
    "system",
    "runs",
    "snapshots",
    "sources",
    "db",
];

fn mode_of(request: &Request) -> Mode {
    Mode {
        dry_run: request.flag("--dry-run"),
        offline: request.flag("--offline"),
    }
}

fn lock_from_env() -> Lock {
    if std::env::var("AGENT_MANAGER_LOCK_HELD").ok().as_deref() == Some("true") {
        let pid = std::env::var("AGENT_MANAGER_LOCK_PID")
            .ok()
            .and_then(|raw| raw.parse::<i64>().ok())
            .unwrap_or_else(|| std::process::id() as i64);
        Lock {
            held: true,
            pid: Some(pid),
        }
    } else {
        Lock {
            held: false,
            pid: None,
        }
    }
}

fn log_file_from_env() -> Option<String> {
    std::env::var("AGENT_MANAGER_LOG_FILE")
        .ok()
        .filter(|value| !value.is_empty())
}

fn make_envelope(
    request: &Request,
    data: Value,
    warnings: Vec<Diagnostic>,
    errors: Vec<Diagnostic>,
) -> Envelope {
    Envelope::new(
        request.command.replacen(':', ".", 1),
        mode_of(request),
        lock_from_env(),
        request.run_id.clone(),
        log_file_from_env(),
        data,
        warnings,
        errors,
    )
}

fn error_envelope(request: &Request, code: &str, message: String) -> Envelope {
    make_envelope(
        request,
        Value::Object(Map::new()),
        vec![],
        vec![diagnostic(code, None, message)],
    )
}

/// The single native entry point. Pure with respect to process state: no exit
/// codes here (CLI maps errors to exit codes); status/plan never write the DB.
pub fn execute(ctx: &Context, request: &Request) -> Envelope {
    match dispatch(ctx, request) {
        Ok((data, warnings)) => make_envelope(request, data, warnings, vec![]),
        Err(DispatchError { code, message }) => error_envelope(request, code, message),
    }
}

struct DispatchError {
    code: &'static str,
    message: String,
}

impl DispatchError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl From<anyhow::Error> for DispatchError {
    fn from(err: anyhow::Error) -> Self {
        DispatchError::new("INTERNAL", format!("{err:#}"))
    }
}

impl From<rusqlite::Error> for DispatchError {
    fn from(err: rusqlite::Error) -> Self {
        DispatchError::new("INTERNAL", err.to_string())
    }
}

type DispatchResult = Result<(Value, Vec<Diagnostic>), DispatchError>;

fn dispatch(ctx: &Context, request: &Request) -> DispatchResult {
    let dotted = request.command.replacen(':', ".", 1);
    let mut parts = dotted.splitn(2, '.');
    let domain = parts.next().unwrap_or("");
    let verb = parts.next().unwrap_or("");
    if domain.is_empty() || verb.is_empty() {
        return Err(DispatchError::new(
            "USAGE_ERROR",
            format!("命令 '{}' 不是 <domain>:<verb> 形式", request.command),
        ));
    }
    if !KNOWN_DOMAINS.contains(&domain) {
        return Err(DispatchError::new(
            "UNKNOWN_DOMAIN",
            format!(
                "未知域 '{}'（已知域: {}）",
                domain,
                KNOWN_DOMAINS.join(", ")
            ),
        ));
    }
    match (domain, verb) {
        ("policy", "get") => policy_get(ctx),
        ("policy", "set") => policy_set(ctx, request),
        ("policy", "delete") => policy_delete(ctx, request),
        ("db", "status") => db_status(ctx),
        ("db", "migrate") => db_migrate(ctx, request),
        ("runs", "list") => runs_list(ctx),
        ("runs", "get") => runs_get(ctx, request),
        _ => Err(DispatchError::new(
            "DOMAIN_NOT_IMPLEMENTED",
            format!(
                "命令 '{dotted}' 属于契约命令面，但 native 核心尚未接线（域模块由对应 owner 提供）"
            ),
        )),
    }
}

// ---------------------------------------------------------------------------
// policy domain
// ---------------------------------------------------------------------------

fn validate_key(key: &str) -> Result<Vec<&str>, DispatchError> {
    if key.is_empty() {
        return Err(DispatchError::new("USAGE_ERROR", "非法 key 路径：不能为空"));
    }
    let segments: Vec<&str> = key.split('.').collect();
    let re = regex_lite_match();
    for segment in &segments {
        if segment.is_empty() || !re(segment) {
            return Err(DispatchError::new(
                "USAGE_ERROR",
                format!("非法 key 路径段（点号分隔，每段只允许字母/数字/_/-）: {key}"),
            ));
        }
    }
    Ok(segments)
}

/// 无 regex 依赖的段校验（等价于 ^[A-Za-z0-9_-]+$）。
fn regex_lite_match() -> impl Fn(&str) -> bool {
    |segment: &str| {
        segment
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    }
}

/// Deep-merge `patch` into `target` honoring `_replace: true` (replace the
/// subtree instead of merging) and `_delete: true` (remove the key), matching
/// the overlay semantics the GUI already sends.
pub fn deep_merge_overlay(target: &mut Value, patch: &Value) {
    match (target, patch) {
        (Value::Object(target_map), Value::Object(patch_map)) => {
            if patch_map.get("_replace") == Some(&Value::Bool(true)) {
                let mut replacement = patch_map.clone();
                replacement.remove("_replace");
                *target_map = replacement;
                return;
            }
            for (key, value) in patch_map {
                if key == "_replace" {
                    continue;
                }
                if value
                    == &Value::Object(Map::from_iter([("_delete".to_string(), Value::Bool(true))]))
                {
                    target_map.remove(key);
                    continue;
                }
                deep_merge_overlay(target_map.entry(key.clone()).or_insert(Value::Null), value);
            }
        }
        (target, patch) => *target = patch.clone(),
    }
}

fn set_dot_path(root: &mut Value, segments: &[&str], value: Value) -> Result<(), DispatchError> {
    let mut cursor = root;
    for segment in &segments[..segments.len() - 1] {
        if !cursor.is_object() {
            return Err(DispatchError::new(
                "POLICY_SET_INVALID",
                format!("配置项路径 '{segment}' 的父级不是对象，拒绝写入子字段"),
            ));
        }
        cursor = cursor
            .as_object_mut()
            .expect("checked above")
            .entry(segment.to_string())
            .or_insert_with(|| Value::Object(Map::new()));
    }
    if !cursor.is_object() {
        return Err(DispatchError::new(
            "POLICY_SET_INVALID",
            "配置项不是对象，拒绝写入子字段",
        ));
    }
    let leaf = segments[segments.len() - 1];
    let map = cursor.as_object_mut().expect("checked above");
    match map.get_mut(leaf) {
        Some(existing) => deep_merge_overlay(existing, &value),
        None => {
            // 新键直接落值（_replace/_delete 包裹的新子树按原样存储语义不展开）
            map.insert(leaf.to_string(), value);
        }
    }
    Ok(())
}

fn delete_dot_path(root: &mut Value, segments: &[&str]) -> Result<(), DispatchError> {
    let mut cursor = root;
    for segment in &segments[..segments.len() - 1] {
        let next = cursor.as_object_mut().and_then(|map| map.get_mut(*segment));
        match next {
            Some(value) => cursor = value,
            None => {
                return Err(DispatchError::new(
                    "POLICY_KEY_NOT_FOUND",
                    format!("键不存在: {}", segments.join(".")),
                ))
            }
        }
    }
    let map = cursor.as_object_mut().ok_or_else(|| {
        DispatchError::new(
            "POLICY_KEY_NOT_FOUND",
            format!("键不存在: {}", segments.join(".")),
        )
    })?;
    if map.remove(segments[segments.len() - 1]).is_none() {
        return Err(DispatchError::new(
            "POLICY_KEY_NOT_FOUND",
            format!("键不存在: {}", segments.join(".")),
        ));
    }
    Ok(())
}

fn reject_secret_text(text: &str) -> Option<&'static str> {
    use std::sync::OnceLock;
    static RULES: OnceLock<Vec<(&'static str, regex::Regex)>> = OnceLock::new();
    let rules = RULES.get_or_init(|| {
        vec![
            (
                "openai_like_key",
                regex::Regex::new(r"\bsk-(?:proj-|ant-)?[A-Za-z0-9_-]{20,}\b").unwrap(),
            ),
            (
                "github_token",
                regex::Regex::new(r"\bgh[pousr]_[A-Za-z0-9]{20,}\b").unwrap(),
            ),
            (
                "google_api_key",
                regex::Regex::new(r"\bAIza[0-9A-Za-z_-]{20,}\b").unwrap(),
            ),
            (
                "aws_access_key",
                regex::Regex::new(r"\bAKIA[0-9A-Z]{16}\b").unwrap(),
            ),
            (
                "private_key_block",
                regex::Regex::new(r"-----BEGIN (?:RSA |EC |DSA |OPENSSH |PGP )?PRIVATE KEY-----")
                    .unwrap(),
            ),
            (
                "augment_session_token",
                regex::Regex::new(r#""accessToken"\s*:\s*"ace_[A-Za-z0-9_-]{12,}""#).unwrap(),
            ),
        ]
    });
    rules
        .iter()
        .find_map(|(kind, rule)| rule.is_match(text).then_some(*kind))
}

fn reject_secrets(key: &str, value: &Value) -> Result<(), DispatchError> {
    if let Some(kind) = reject_secret_text(key) {
        return Err(DispatchError::new(
            "SECRET_INPUT_REJECTED",
            format!("拒绝写入疑似凭据: {kind}"),
        ));
    }
    match value {
        Value::String(text) => reject_secret_text(text).map_or(Ok(()), |kind| {
            Err(DispatchError::new(
                "SECRET_INPUT_REJECTED",
                format!("拒绝写入疑似凭据: {kind}"),
            ))
        }),
        Value::Object(map) => {
            for (child, item) in map {
                reject_secrets(child, item)?;
            }
            Ok(())
        }
        Value::Array(items) => {
            for item in items {
                reject_secrets(key, item)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn policy_data(
    layer: Option<&str>,
    key: Option<&str>,
    value: Option<Value>,
    path: Option<String>,
    effective: Option<Value>,
    attribution: Option<Value>,
) -> Value {
    json!({
        "layers": Value::Null,
        "effective": effective,
        "attribution": attribution,
        "layer": layer,
        "key": key,
        "value": value,
        "path": path,
    })
}

fn policy_get(ctx: &Context) -> DispatchResult {
    let db = store::db_path(ctx);
    let exists = db.is_file();
    let effective = store::effective_policy(ctx).map_err(DispatchError::from)?;
    let attribution: Value = effective
        .as_object()
        .map(|map| {
            map.keys()
                .map(|key| {
                    (
                        key.clone(),
                        Value::String(if exists {
                            "sqlite".into()
                        } else {
                            "builtin".into()
                        }),
                    )
                })
                .collect::<Map<String, Value>>()
        })
        .map(Value::Object)
        .unwrap_or(Value::Null);
    let mut layers = Map::new();
    layers.insert("baseline".into(), Value::Null);
    layers.insert("synced".into(), Value::Null);
    layers.insert("local".into(), Value::Null);
    layers.insert(
        "sqlite".into(),
        if exists {
            effective.clone()
        } else {
            Value::Null
        },
    );
    let mut data = policy_data(
        None,
        None,
        None,
        Some(db.display().to_string()),
        Some(effective),
        Some(attribution),
    );
    data.as_object_mut()
        .expect("object")
        .insert("layers".into(), Value::Object(layers));
    Ok((data, vec![]))
}

fn policy_set(ctx: &Context, request: &Request) -> DispatchResult {
    if request.value("--layer") != Some("local") {
        return Err(DispatchError::new(
            "USAGE_ERROR",
            "policy:set 仅支持 --layer local",
        ));
    }
    let key = request.value("--key").ok_or_else(|| {
        DispatchError::new(
            "USAGE_ERROR",
            "policy:set 需要 --key <dot.path> --value <json>",
        )
    })?;
    let raw_value = request.value("--value").ok_or_else(|| {
        DispatchError::new(
            "USAGE_ERROR",
            "policy:set 需要 --key <dot.path> --value <json>",
        )
    })?;
    let value: Value = serde_json::from_str(raw_value).map_err(|err| {
        DispatchError::new("USAGE_ERROR", format!("--value 不是合法 JSON: {err}"))
    })?;
    let segments = validate_key(key)?;
    reject_secrets(key, &value)?;
    if request.flag("--dry-run") {
        let data = policy_data(Some("sqlite"), Some(key), Some(value), None, None, None);
        return Ok((
            data,
            vec![diagnostic(
                "DRY_RUN",
                None,
                format!("dry-run：未写入 {key}"),
            )],
        ));
    }
    // 首次明确保存即初始化库并固化内置默认；同连接同事务完成读-改-写。
    let mut conn = store::open_write(ctx).map_err(DispatchError::from)?;
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(DispatchError::from)?;
    let mut policy = store::policy_from_connection(&tx).map_err(DispatchError::from)?;
    set_dot_path(&mut policy, &segments, value.clone())?;
    store::write_policy(&tx, &policy).map_err(DispatchError::from)?;
    store::record_operation(
        &tx,
        Some(&request.run_id),
        "policy:set",
        Some(key),
        Some(&json!({ "key": key, "layer": "local" })),
    )
    .map_err(DispatchError::from)?;
    tx.commit().map_err(DispatchError::from)?;
    let data = policy_data(
        Some("sqlite"),
        Some(key),
        Some(value),
        Some(store::db_path(ctx).display().to_string()),
        None,
        None,
    );
    Ok((data, vec![]))
}

fn policy_delete(ctx: &Context, request: &Request) -> DispatchResult {
    let key = request
        .value("--key")
        .ok_or_else(|| DispatchError::new("USAGE_ERROR", "policy:delete 需要 --key <dot.path>"))?;
    let segments = validate_key(key)?;
    if request.flag("--dry-run") {
        let data = policy_data(Some("sqlite"), Some(key), None, None, None, None);
        return Ok((
            data,
            vec![diagnostic(
                "DRY_RUN",
                None,
                format!("dry-run：未删除 {key}"),
            )],
        ));
    }
    let mut conn = match store::open_readonly(ctx).map_err(DispatchError::from)? {
        // 无库即无键：删除不建库
        None => {
            return Err(DispatchError::new(
                "POLICY_KEY_NOT_FOUND",
                format!("键不存在: {key}"),
            ))
        }
        Some(_) => store::open_write(ctx).map_err(DispatchError::from)?,
    };
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(DispatchError::from)?;
    let mut policy = store::policy_from_connection(&tx).map_err(DispatchError::from)?;
    delete_dot_path(&mut policy, &segments)?;
    store::write_policy(&tx, &policy).map_err(DispatchError::from)?;
    store::record_operation(&tx, Some(&request.run_id), "policy:delete", Some(key), None)
        .map_err(DispatchError::from)?;
    tx.commit().map_err(DispatchError::from)?;
    let data = policy_data(
        Some("sqlite"),
        Some(key),
        None,
        Some(store::db_path(ctx).display().to_string()),
        None,
        None,
    );
    Ok((data, vec![]))
}

// ---------------------------------------------------------------------------
// db domain
// ---------------------------------------------------------------------------

fn db_status(ctx: &Context) -> DispatchResult {
    let path = store::db_path(ctx);
    let exists = path.is_file();
    let mut meta = Map::new();
    let mut ready = false;
    match store::open_readonly(ctx) {
        Ok(Some(conn)) => {
            ready = true;
            if let Ok(mut stmt) = conn.prepare("SELECT key, value FROM schema_meta") {
                let rows = stmt.query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                });
                if let Ok(rows) = rows {
                    for row in rows.flatten() {
                        meta.insert(row.0, Value::String(row.1));
                    }
                }
            }
            let count = |sql: &str| {
                conn.query_row(sql, [], |row| row.get::<_, i64>(0))
                    .unwrap_or(0)
            };
            meta.insert(
                "counts".into(),
                json!({
                    "policies": count("SELECT COUNT(*) FROM policy_entries"),
                    "operations": count("SELECT COUNT(*) FROM operation_logs"),
                }),
            );
        }
        Ok(None) => {}
        Err(err) => {
            meta.insert("error".into(), Value::String(format!("{err:#}")));
        }
    }
    let data = json!({
        "engine": "native",
        "active": exists,
        "ready": ready,
        "data_dir": ctx.data_dir.display().to_string(),
        "db_path": path.display().to_string(),
        "exists": exists,
        "meta": Value::Object(meta),
    });
    Ok((data, vec![]))
}

fn db_migrate(ctx: &Context, request: &Request) -> DispatchResult {
    let path = store::db_path(ctx);
    let exists = path.is_file();
    if request.flag("--dry-run") {
        let (mut data, _) = db_status(ctx)?;
        let map = data.as_object_mut().expect("object");
        map.insert("dry_run".into(), Value::Bool(true));
        map.insert("would_migrate".into(), Value::Bool(!exists));
        return Ok((
            data,
            vec![diagnostic(
                "DRY_RUN",
                None,
                "dry-run：未执行初始化，native-manager.db 状态未改变".to_string(),
            )],
        ));
    }
    store::open_write(ctx).map_err(DispatchError::from)?;
    db_status(ctx)
}

// ---------------------------------------------------------------------------
// runs domain (native audit trail in operation_logs)
// ---------------------------------------------------------------------------

fn runs_list(ctx: &Context) -> DispatchResult {
    let mut runs: Vec<Value> = Vec::new();
    if let Some(conn) = store::open_readonly(ctx).map_err(DispatchError::from)? {
        let mut stmt = conn.prepare(
            "SELECT run_id, COUNT(*) AS ops, MAX(created_at) AS last_at
             FROM operation_logs WHERE run_id IS NOT NULL
             GROUP BY run_id ORDER BY last_at DESC",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(|err| DispatchError::new("INTERNAL", err.to_string()))?;
        for row in rows {
            let (run_id, ops, last_at) = row?;
            runs.push(json!({ "run_id": run_id, "operations": ops, "last_at": last_at }));
        }
    }
    Ok((json!({ "runs": runs }), vec![]))
}

fn runs_get(ctx: &Context, request: &Request) -> DispatchResult {
    let run_id = request
        .value("--run")
        .ok_or_else(|| DispatchError::new("USAGE_ERROR", "runs:get 需要 --run <run_id>"))?;
    let mut operations: Vec<Value> = Vec::new();
    if let Some(conn) = store::open_readonly(ctx).map_err(DispatchError::from)? {
        let mut stmt = conn.prepare(
            "SELECT id, operation_type, target, detail_json, created_at
             FROM operation_logs WHERE run_id = ?1 ORDER BY created_at ASC",
        )?;
        let rows = stmt
            .query_map([run_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })
            .map_err(|err| DispatchError::new("INTERNAL", err.to_string()))?;
        for row in rows {
            let (id, operation_type, target, detail_json, created_at) = row?;
            operations.push(json!({
                "id": id,
                "operation_type": operation_type,
                "target": target,
                "detail": detail_json.and_then(|raw| serde_json::from_str::<Value>(&raw).ok()),
                "created_at": created_at,
            }));
        }
    }
    if operations.is_empty() {
        return Err(DispatchError::new(
            "RUN_NOT_FOUND",
            format!("未找到运行记录: {run_id}"),
        ));
    }
    Ok((
        json!({ "run_id": run_id, "operations": operations }),
        vec![],
    ))
}
