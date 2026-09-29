//! 配置备份：把本机的定义、来源与规则模板导出成一份可移植 JSON，或从这份 JSON 恢复。
//!
//! 规则不再有来源：生效规则就是各助手的规则文件（换机器后由用户自己决定写什么），
//! 备份只带已保存的规则模板（`rule_templates`）。旧备份里的 `rules.*` 来源仍接受，
//! 按旧规则渲染转换成「旧规则来源」模板。
//!
//! Hooks 已不再由本工具管理（2026-09-29）：不导出；旧备份里的 `hooks.*` 来源照常接受整份导入，
//! 但这一段跳过不写，在结果的 `skipped` 里列出。
//! 终端快捷命令同理：rc 文件受管区块里的实际别名不进备份，只有旧版本保存、与内置推荐不同的
//! 别名（`alias_sources` 里仍有候选的行）以 `aliases.shell` 来源导出；导入写回旧表，在别名页
//! 作为「已保存未生效」建议出现，不改 rc 文件。
//!
//! 导出只读零写；导入是整体替换，走与逐条保存相同的校验闸门（疑似凭据拒绝、
//! MCP 定义校验、来源结构校验），并在一个事务里完成——要么整份生效，要么原样不动。
//!
//! 不进备份的东西：写入指纹（那是本机落地状态）、操作记录、探测缓存、本机偏好。
//! MCP 的生效条目在各助手自己的配置文件里，不进备份；「已保存未生效」的 MCP 项
//! （停用快照、未分配定义，`mcp_saved`）进备份，导入时逐项写回（同名以备份为准，
//! 备份里没有的本机项保留，不丢失）。
//!
//! 凭据取值与外部密钥文件路径都不进备份；定义与来源里只有 `${VAR}` 引用，
//! MCP 已保存项另外只含未命中疑似凭据检测的普通字面量。

use anyhow::{bail, Result};
use rusqlite::TransactionBehavior;
use serde_json::{json, Map, Value};

use std::collections::BTreeMap;

use crate::native::{Context, Request};
use crate::native_aliases::legacy as alias_legacy;
use crate::native_mcp::saved::{self as mcp_saved, SavedRow};
use crate::native_rules::templates::{self, Template};
use crate::store;

/// 备份格式标识：导入时严格比对，不认识的格式直接拒绝而不是尽力解析。
const FORMAT: &str = "agent-manager/config-backup/v1";

/// 参与备份的来源键，顺序固定，便于逐份比对。都是旧版本保存、还没生效的内容，只在有
/// 候选时出现。
const SOURCE_KEYS: &[&str] = &[alias_legacy::LEGACY_KEY];

/// 已退役功能的来源键：旧备份里可能有，接受但不导入。
const RETIRED_SOURCE_KEYS: &[&str] = &["hooks.claude", "hooks.codex"];

fn collect_sources(ctx: &Context) -> Result<Map<String, Value>> {
    let mut legacy = BTreeMap::new();
    if let Some(text) = alias_legacy::exportable(alias_legacy::read_row_readonly(ctx)?.as_ref()) {
        legacy.insert(alias_legacy::LEGACY_KEY.to_string(), text);
    }
    let mut out = Map::new();
    for key in SOURCE_KEYS {
        if let Some(text) = legacy.get(*key) {
            out.insert((*key).to_string(), Value::String(text.clone()));
        }
    }
    Ok(out)
}

/// 只读导出：定义树 + 来源正文 + 规则模板 + MCP 已保存项。零写，不创建数据目录或数据库。
pub(crate) fn export(ctx: &Context, req: &Request) -> Result<Value> {
    if let Some(arg) = req.args.first() {
        bail!("config:export 不接受参数，得到 '{}'", arg);
    }
    let mut policy = store::effective_policy(ctx)?;
    let sources = collect_sources(ctx)?;
    let (_, rule_templates) = crate::native_rules::status::load_snapshot(ctx)?;
    // 旧库里还没迁移的 MCP 定义按迁移规则推导成已保存项导出，策略里不再重复带。
    let mcp = crate::native_mcp::inventory::load(ctx)?;
    if let Some(map) = policy.as_object_mut() {
        if map.contains_key("mcp") {
            map.insert("mcp".into(), json!({"definitions": {}}));
        }
    }
    let saved: Vec<Value> = mcp.saved.iter().map(|row| row.to_backup()).collect();
    Ok(json!({
        "format": FORMAT,
        "exported_at": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        "policy": policy,
        "sources": Value::Object(sources),
        "rule_templates": rule_templates
            .iter()
            .map(|t| json!({ "name": t.name, "body": t.body, "origin": t.origin }))
            .collect::<Vec<_>>(),
        "mcp_saved": saved,
    }))
}

/// 校验后的备份内容：policy、来源（不含旧规则键）、规则模板（None = 备份没带这一段）、
/// 旧规则来源（旧备份的 rules.* 键）、MCP 已保存项。
struct Payload {
    policy: Value,
    sources: Map<String, Value>,
    rule_templates: Option<Vec<(String, String, String)>>,
    legacy_rules: BTreeMap<String, String>,
    mcp_saved: Vec<SavedRow>,
    /// 已退役功能的来源键：跳过不导入。
    retired: Vec<String>,
}

/// 规则模板段：[{name, body, origin?}]，名称合法且不重复，正文无 NUL、不含疑似凭据。
fn validate_rule_templates(value: &Value) -> Result<Vec<(String, String, String)>> {
    let items = value
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("备份里的 rule_templates 必须是数组"))?;
    let mut out: Vec<(String, String, String)> = Vec::new();
    for item in items {
        let obj = item
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("rule_templates 的每一项必须是对象"))?;
        let name = obj
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("rule_templates 的 name 必须是字符串"))?;
        let name = templates::validate_name(name)?;
        let body = obj
            .get("body")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("rule_templates 的 body 必须是字符串"))?;
        templates::validate_body(body)?;
        let origin = match obj.get("origin") {
            None | Some(Value::Null) => templates::ORIGIN_USER,
            Some(Value::String(o)) if o == templates::ORIGIN_USER => templates::ORIGIN_USER,
            Some(Value::String(o)) if o == templates::ORIGIN_BACKUP => templates::ORIGIN_BACKUP,
            Some(Value::String(o)) if o == templates::ORIGIN_LEGACY => templates::ORIGIN_LEGACY,
            _ => bail!("rule_templates 的 origin 不认识"),
        };
        if out.iter().any(|(n, _, _)| n == &name) {
            bail!("rule_templates 里有重名模板，拒绝导入");
        }
        out.push((name, body.to_string(), origin.to_string()));
    }
    Ok(out)
}

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

    let mut sources = obj
        .get("sources")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let retired: Vec<String> = RETIRED_SOURCE_KEYS
        .iter()
        .filter(|key| sources.remove(**key).is_some())
        .map(|key| key.to_string())
        .collect();
    for (key, value) in &sources {
        if !SOURCE_KEYS.contains(&key.as_str()) && !templates::LEGACY_KEYS.contains(&key.as_str()) {
            bail!("备份里有不认识的来源键，拒绝导入");
        }
        let text = value
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("来源 {key} 必须是字符串"))?;
        crate::native::reject_secrets_value(&Value::String(text.to_string()))?;
        // JSON 形态的来源按各自域的结构校验，坏结构不写进去
        if key == alias_legacy::LEGACY_KEY {
            let parsed: Value = serde_json::from_str(text)
                .map_err(|_| anyhow::anyhow!("来源 {key} 不是合法 JSON"))?;
            let map = parsed
                .as_object()
                .ok_or_else(|| anyhow::anyhow!("来源 {key} 不是 JSON 对象"))?;
            crate::native_aliases::model::validate_source(map)?;
        }
    }

    // 旧版可能把明文放在 secrets 段。空段兼容；非空一律拒绝，避免重新落入数据库。
    if obj
        .get("secrets")
        .and_then(Value::as_object)
        .is_some_and(|map| !map.is_empty())
    {
        bail!("这份旧备份含明文凭据，已拒绝导入；请先把凭据移到外部密钥文件，并从备份中删除 secrets 段");
    }
    if obj
        .get("secrets")
        .is_some_and(|value| !value.is_null() && !value.is_object())
    {
        bail!("备份里的 secrets 必须为空对象或省略");
    }

    let rule_templates = match obj.get("rule_templates") {
        None | Some(Value::Null) => None,
        Some(value) => Some(validate_rule_templates(value)?),
    };
    // 旧备份的规则来源：不再是来源，导入时转换成旧来源模板
    let mut legacy_rules = BTreeMap::new();
    for key in templates::LEGACY_KEYS {
        if let Some(Value::String(text)) = sources.remove(key) {
            legacy_rules.insert(key.to_string(), text);
        }
    }

    // MCP 已保存项：与逐项写入同一套不变式（无疑似凭据、headers 只存引用）
    let mcp_saved = match obj.get("mcp_saved") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(items)) => items
            .iter()
            .map(SavedRow::from_backup)
            .collect::<Result<Vec<_>>>()?,
        Some(_) => bail!("备份里的 mcp_saved 必须是数组"),
    };

    Ok(Payload {
        policy,
        sources,
        rule_templates,
        legacy_rules,
        mcp_saved,
        retired,
    })
}

/// 导入：整体替换定义与来源；带 rule_templates 时整体替换规则模板，旧备份的 rules.*
/// 来源转换成旧来源模板；MCP 已保存项逐项写入（同名以备份为准，本机其余项保留）。
/// 一个事务内完成，任一步失败即全部回滚。
/// 不动分配记录、写入指纹与任何规则文件——恢复定义之后还要各自应用一次才会落到目标配置。
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
    let payload = validate_payload(&payload)?;

    let mut conn = store::open_write(ctx)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    // 本机旧库先迁移（本机的已保存项因此不会随策略替换而丢失），再换策略；
    // 备份策略里的旧版 MCP 定义随即转成未分配的已保存定义。
    let current = store::policy_from_connection(&tx)?;
    let scans = crate::native_mcp::inventory::scan_targets(&current, &ctx.home_dir);
    crate::native_mcp::legacy::migrate(&tx, &scans, Some(&req.run_id))?;
    store::write_policy(&tx, &payload.policy)?;
    crate::native_mcp::legacy::migrate(&tx, &scans, Some(&req.run_id))?;
    if !payload.mcp_saved.is_empty() {
        mcp_saved::ensure_table(&tx)?;
        for row in &payload.mcp_saved {
            mcp_saved::upsert(&tx, row)?;
        }
    }
    let mut restored = Vec::new();
    for (key, value) in &payload.sources {
        let text = value.as_str().unwrap_or("");
        // 旧版本保存的别名：写回旧表，作为已保存未生效的建议出现
        let parsed: Value = serde_json::from_str(text)?;
        let map = parsed
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("来源 {key} 不是 JSON 对象"))?;
        alias_legacy::write_row_in_tx(&tx, map)?;
        restored.push(key.clone());
    }
    let now = templates::now();
    let mut restored_templates = 0usize;
    if let Some(items) = &payload.rule_templates {
        let list: Vec<Template> = items
            .iter()
            .map(|(name, body, origin)| Template {
                id: templates::new_id(),
                name: name.clone(),
                body: body.clone(),
                origin: origin.clone(),
                created_at: now.clone(),
                updated_at: now.clone(),
            })
            .collect();
        templates::replace_all(&tx, &list)?;
        restored_templates = list.len();
    }
    if !payload.legacy_rules.is_empty() {
        let existing = templates::load_from(&tx)?;
        for mut template in templates::legacy_templates(&payload.legacy_rules, &now) {
            let others: Vec<Template> = existing
                .iter()
                .filter(|t| t.id != template.id)
                .cloned()
                .collect();
            template.name = templates::unique_name(&others, &template.name);
            templates::upsert(&tx, &template)?;
            restored_templates += 1;
        }
    }
    store::record_operation(
        &tx,
        Some(&req.run_id),
        "config:import",
        None,
        Some(&json!({
            "action": "import",
            "sources": restored,
            "rule_templates": restored_templates,
            "mcp_saved": payload.mcp_saved.len(),
        })),
    )?;
    tx.commit()?;

    Ok(json!({
        "domain": "config",
        "steps": [{"name": "config:import", "ok": true, "exit_code": 0}],
        "skipped": payload.retired,
    }))
}

pub(crate) fn handle(ctx: &Context, req: &Request) -> Result<Value> {
    match req.command.replace('.', ":").as_str() {
        "config:export" => export(ctx, req),
        "config:import" => import(ctx, req),
        other => bail!("config 域不支持命令 '{}'", other),
    }
}
