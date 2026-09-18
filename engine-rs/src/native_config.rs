//! 配置备份：把本机的定义与来源导出成一份可移植 JSON，或从这份 JSON 恢复。
//!
//! 导出只读零写；导入是整体替换，走与逐条保存相同的校验闸门（疑似凭据拒绝、
//! MCP 定义校验、来源结构校验），并在一个事务里完成——要么整份生效，要么原样不动。
//!
//! 不进备份的东西：分配记录与写入指纹（那是本机落地状态，换机器后必须重新应用）、
//! 操作记录与诊断日志、探测缓存、本机偏好。凭据本来就不入库，备份里自然只有 `${VAR}` 引用。

use anyhow::{bail, Result};
use rusqlite::TransactionBehavior;
use serde_json::{json, Map, Value};

use crate::native::{Context, Request};
use crate::native_sources;
use crate::store;

/// 备份格式标识：导入时严格比对，不认识的格式直接拒绝而不是尽力解析。
const FORMAT: &str = "agent-manager/config-backup/v1";

/// 参与备份的来源键，顺序固定，便于逐份比对。
const SOURCE_KEYS: &[&str] = &[
    native_sources::KEY_SHARED,
    native_sources::KEY_CLAUDE,
    native_sources::KEY_CODEX,
    native_sources::KEY_HOOKS_CLAUDE,
    native_sources::KEY_HOOKS_CODEX,
    native_sources::KEY_ALIAS_SHELL,
];

fn collect_sources(ctx: &Context) -> Result<Map<String, Value>> {
    let rules = native_sources::current_sources_readonly(ctx)?;
    let hooks = native_sources::current_hook_sources_readonly(ctx)?;
    let alias = native_sources::current_alias_source_readonly(ctx)?;
    let mut out = Map::new();
    for key in SOURCE_KEYS {
        let text = match *key {
            native_sources::KEY_ALIAS_SHELL => alias.clone(),
            k if k.starts_with("hooks.") => hooks.get(k).cloned().unwrap_or_default(),
            k => rules.get(k).cloned().unwrap_or_default(),
        };
        out.insert((*key).to_string(), Value::String(text));
    }
    Ok(out)
}

/// 只读导出：定义树 + 来源正文。零写，不创建数据目录或数据库。
pub(crate) fn export(ctx: &Context, req: &Request) -> Result<Value> {
    for arg in &req.args {
        bail!("config:export 不接受参数，得到 '{}'", arg);
    }
    let policy = store::effective_policy(ctx)?;
    let sources = collect_sources(ctx)?;
    Ok(json!({
        "format": FORMAT,
        "exported_at": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        "policy": policy,
        "sources": Value::Object(sources),
    }))
}

/// 校验备份内容：格式标识、结构，以及与逐条保存同一套的凭据与定义闸门。
fn validate_payload(payload: &Value) -> Result<(Value, Map<String, Value>)> {
    let obj = payload
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("备份内容必须是 JSON 对象"))?;
    match obj.get("format").and_then(Value::as_str) {
        Some(FORMAT) => {}
        _ => bail!("备份格式不认识，拒绝导入（期望 {FORMAT}）"),
    }
    let policy = obj
        .get("policy")
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("备份缺少 policy"))?;
    if !policy.is_object() {
        bail!("备份里的 policy 必须是对象");
    }
    // 与逐条保存同一道闸门：明文凭据一律拒绝入库
    crate::native::reject_secrets_value(&policy)?;
    // MCP 定义按既有规则逐条校验，坏定义不进库
    if let Some(defs) = policy
        .get("mcp")
        .and_then(|v| v.get("definitions"))
        .and_then(Value::as_object)
    {
        for (name, def) in defs {
            if def.get("_delete") == Some(&Value::Bool(true)) {
                continue;
            }
            crate::native_mcp::policy_view::validate_definition(def, name)?;
        }
    }

    let sources = obj
        .get("sources")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    for (key, value) in &sources {
        if !SOURCE_KEYS.contains(&key.as_str()) {
            bail!("备份里有不认识的来源键，拒绝导入");
        }
        let text = value
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("来源 {key} 必须是字符串"))?;
        crate::native::reject_secrets_value(&Value::String(text.to_string()))?;
        // JSON 形态的来源按各自域的结构校验，坏结构不写进去
        if key.starts_with("hooks.") {
            let parsed: Value = serde_json::from_str(text)
                .map_err(|_| anyhow::anyhow!("来源 {key} 不是合法 JSON"))?;
            let map = parsed
                .as_object()
                .ok_or_else(|| anyhow::anyhow!("来源 {key} 不是 JSON 对象"))?;
            crate::native_hooks::model::validate_source(key, map)?;
        } else if key == native_sources::KEY_ALIAS_SHELL {
            let parsed: Value = serde_json::from_str(text)
                .map_err(|_| anyhow::anyhow!("来源 {key} 不是合法 JSON"))?;
            let map = parsed
                .as_object()
                .ok_or_else(|| anyhow::anyhow!("来源 {key} 不是 JSON 对象"))?;
            crate::native_aliases::model::validate_source(map)?;
        }
    }
    Ok((policy, sources))
}

/// 导入：整体替换定义与来源。一个事务内完成，任一步失败即全部回滚。
/// 不动分配记录与写入指纹——恢复定义之后还要各自应用一次才会落到目标配置。
pub(crate) fn import(ctx: &Context, req: &Request) -> Result<Value> {
    let raw = req
        .value("--value")
        .ok_or_else(|| anyhow::anyhow!("缺少 --value（备份 JSON 正文）"))?;
    for arg in &req.args {
        if arg.starts_with("--") && arg != "--value" {
            bail!("config:import 不支持参数 '{}'", arg);
        }
    }
    let payload: Value =
        serde_json::from_str(raw).map_err(|_| anyhow::anyhow!("备份内容不是合法 JSON"))?;
    let (policy, sources) = validate_payload(&payload)?;

    let mut conn = store::open_write(ctx)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    store::write_policy(&tx, &policy)?;
    let mut restored = Vec::new();
    for (key, value) in &sources {
        let text = value.as_str().unwrap_or("");
        native_sources::write_source_in_tx(&tx, key, text)?;
        restored.push(key.clone());
    }
    store::record_operation(
        &tx,
        Some(&req.run_id),
        "config:import",
        None,
        Some(&json!({"action": "import", "sources": restored})),
    )?;
    tx.commit()?;

    Ok(json!({
        "domain": "config",
        "steps": [{"name": "config:import", "ok": true, "exit_code": 0}],
        "skipped": [],
    }))
}

pub(crate) fn handle(ctx: &Context, req: &Request) -> Result<Value> {
    match req.command.replace('.', ":").as_str() {
        "config:export" => export(ctx, req),
        "config:import" => import(ctx, req),
        other => bail!("config 域不支持命令 '{}'", other),
    }
}
