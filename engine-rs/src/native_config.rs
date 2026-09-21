//! 配置备份：把本机的定义与来源导出成一份可移植 JSON，或从这份 JSON 恢复。
//!
//! 导出只读零写；导入是整体替换，走与逐条保存相同的校验闸门（疑似凭据拒绝、
//! MCP 定义校验、来源结构校验），并在一个事务里完成——要么整份生效，要么原样不动。
//!
//! 不进备份的东西：分配记录与写入指纹（那是本机落地状态，换机器后必须重新应用）、
//! 操作记录、探测缓存、本机偏好。
//!
//! **备份里含明文凭据取值**（`secrets` 段，用户在密钥页填过的那些），这样换机器导入即可用。
//! 代价是这份文件本身就是凭据载体：误发、误传、放进网盘或仓库都等同泄密。导出方需自行妥善
//! 保管，界面上对此有明确标注，落盘文件权限为 0600。定义与来源里仍然只有 `${VAR}` 引用。

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
    if let Some(arg) = req.args.first() {
        bail!("config:export 不接受参数，得到 '{}'", arg);
    }
    let policy = store::effective_policy(ctx)?;
    let sources = collect_sources(ctx)?;
    // 凭据取值一并带上：换机器导入即可用。这也让这份文件成为凭据载体，
    // 调用方（界面/CLI）必须把它当密钥文件对待。
    let secrets: Map<String, Value> = match store::open_readonly(ctx)? {
        Some(conn) => crate::secret_values::read_all(&conn)?
            .into_iter()
            .map(|(k, v)| (k, Value::String(v)))
            .collect(),
        None => Map::new(),
    };
    Ok(json!({
        "format": FORMAT,
        "exported_at": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        "policy": policy,
        "sources": Value::Object(sources),
        "secrets": Value::Object(secrets),
    }))
}

/// 校验备份内容：格式标识、结构，以及与逐条保存同一套的凭据与定义闸门。
type Payload = (Value, Map<String, Value>, Map<String, Value>);

fn validate_payload(payload: &Value) -> Result<Payload> {
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

    // secrets 段是唯一允许携带明文取值的地方，因此不过 reject_secrets_value；
    // 但键必须是合法变量名、值必须是字符串，坏结构不进库。缺这一段的老备份按空处理。
    let secrets = match obj.get("secrets") {
        None | Some(Value::Null) => Map::new(),
        Some(Value::Object(map)) => {
            for (name, value) in map {
                if !crate::secret_values::valid_var_name(name) {
                    bail!("备份里有不合法的凭据变量名，拒绝导入");
                }
                if !value.is_string() {
                    bail!("备份里的凭据取值必须是字符串");
                }
            }
            map.clone()
        }
        Some(_) => bail!("备份里的 secrets 必须是对象"),
    };

    Ok((policy, sources, secrets))
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
    let (policy, sources, secrets) = validate_payload(&payload)?;

    let mut conn = store::open_write(ctx)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    store::write_policy(&tx, &policy)?;
    let mut restored = Vec::new();
    for (key, value) in &sources {
        let text = value.as_str().unwrap_or("");
        native_sources::write_source_in_tx(&tx, key, text)?;
        restored.push(key.clone());
    }
    // 凭据取值整体替换：审计只记变量名与条数，绝不记取值。
    crate::secret_values::ensure_table(&tx)?;
    let now = crate::native_mcp::model::now_iso();
    let mut restored_secrets: Vec<String> = Vec::new();
    for (name, value) in &secrets {
        crate::secret_values::set(&tx, name, value.as_str().unwrap_or(""), &now)?;
        restored_secrets.push(name.clone());
    }
    store::record_operation(
        &tx,
        Some(&req.run_id),
        "config:import",
        None,
        Some(&json!({
            "action": "import",
            "sources": restored,
            "secrets": restored_secrets,
        })),
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
