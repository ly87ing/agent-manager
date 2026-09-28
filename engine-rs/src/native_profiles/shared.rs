//! 已建独立环境的共用设置：只读比较实际文件，用户逐项确认后按来源和目标基准写入。
//! 不维护第二份设置；主配置永远只读，凭据不进入预览、审计或数据库。

use anyhow::{anyhow, bail, Result};
use rusqlite::TransactionBehavior;
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};

use super::model::{ProfileDef, ProfileKind};
use super::seed;
use crate::native::{Context, Request};
use crate::native_mcp::model::sha256_hex_bytes;
use crate::native_mcp::rollback::RollbackGuard;
use crate::store;

const SETTINGS: &[&str] = &["permissions", "statusline"];

fn only(req: &Request, apply: bool) -> Result<(String, Option<(String, String, String)>)> {
    let mut values = std::collections::BTreeMap::new();
    let mut iter = req.args.iter();
    while let Some(flag) = iter.next() {
        if !matches!(
            flag.as_str(),
            "--only" | "--item" | "--source-base" | "--target-base"
        ) || (!apply && flag != "--only")
            || values.contains_key(flag.as_str())
        {
            bail!("不支持或重复的参数 {flag}");
        }
        values.insert(
            flag.as_str(),
            iter.next()
                .ok_or_else(|| anyhow!("{flag} 缺少取值"))?
                .as_str(),
        );
    }
    let id = values
        .remove("--only")
        .and_then(|raw| raw.strip_prefix("profiles:"))
        .ok_or_else(|| anyhow!("需要 --only profiles:<实例名>"))?;
    if !super::model::valid_profile_id(id) {
        bail!("实例名只能用小写字母、数字与连字符");
    }
    let selection = if apply {
        let item = values
            .remove("--item")
            .ok_or_else(|| anyhow!("缺少 --item"))?;
        let source = values
            .remove("--source-base")
            .ok_or_else(|| anyhow!("缺少 --source-base"))?;
        let target = values
            .remove("--target-base")
            .ok_or_else(|| anyhow!("缺少 --target-base"))?;
        if !matches!(item, "permissions" | "statusline" | "hooks" | "rules")
            && !item
                .strip_prefix("mcp:")
                .is_some_and(crate::native_mcp::model::is_valid_server_name)
        {
            bail!("不认识的共用设置项");
        }
        for base in [source, target] {
            if base != "missing"
                && (base.len() != 64
                    || !base
                        .bytes()
                        .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)))
            {
                bail!("基准必须是 64 位小写摘要或 missing");
            }
        }
        Some((item.to_string(), source.to_string(), target.to_string()))
    } else {
        None
    };
    if !values.is_empty() {
        bail!("多余的参数");
    }
    Ok((id.to_string(), selection))
}

pub(crate) fn preflight(ctx: &Context, req: &Request) -> Result<()> {
    let (id, _) = only(req, true)?;
    let def = seed::find_def(ctx, &id)?;
    seed::config_dir(ctx, &def)?;
    Ok(())
}

fn sub(req: &Request, command: &str, args: Vec<String>) -> Request {
    Request {
        command: command.into(),
        args,
        run_id: req.run_id.clone(),
    }
}

fn read_source(path: &Path) -> Result<Option<Vec<u8>>> {
    match std::fs::metadata(path) {
        Ok(meta) if meta.is_file() && meta.len() <= 8 * 1024 * 1024 => {
            Ok(Some(std::fs::read(path)?))
        }
        Ok(_) => bail!("来源配置不是 8 MiB 以内的普通文件"),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err.into()),
    }
}

fn settings_file(ctx: &Context, def: &ProfileDef) -> PathBuf {
    match def.kind {
        ProfileKind::Claude => ctx.home_dir.join(".claude/settings.json"),
        ProfileKind::Codex => ctx.home_dir.join(".codex/config.toml"),
    }
}

fn json_doc(bytes: Option<&[u8]>) -> Result<Map<String, Value>> {
    match bytes {
        None => Ok(Map::new()),
        Some(raw) => serde_json::from_slice::<Value>(raw)?
            .as_object()
            .cloned()
            .ok_or_else(|| anyhow!("配置不是 JSON 对象")),
    }
}

fn toml_doc(bytes: Option<&[u8]>) -> Result<toml_edit::Document> {
    match bytes {
        None => Ok(toml_edit::Document::new()),
        Some(raw) => std::str::from_utf8(raw)?
            .parse::<toml_edit::Document>()
            .map_err(Into::into),
    }
}

fn codex_part(doc: &toml_edit::Document, item: &str) -> Result<Option<Value>> {
    let mut values = Map::new();
    if item == "permissions" {
        for key in seed::CODEX_PERMISSION_KEYS {
            if let Some(value) = doc.get(key) {
                values.insert((*key).into(), json!(value.to_string()));
            }
        }
    } else {
        let tui = doc.get("tui");
        if tui.is_some_and(|value| !value.is_table_like()) {
            bail!("config.toml 的 tui 不是表");
        }
        for key in seed::CODEX_STATUSLINE_KEYS {
            if let Some(value) = tui
                .and_then(toml_edit::Item::as_table_like)
                .and_then(|table| table.get(key))
            {
                values.insert((*key).into(), json!(value.to_string()));
            }
        }
    }
    Ok((!values.is_empty()).then_some(Value::Object(values)))
}

fn setting_part(kind: ProfileKind, bytes: Option<&[u8]>, item: &str) -> Result<Option<Value>> {
    match kind {
        ProfileKind::Claude => {
            let doc = json_doc(bytes)?;
            let value = doc
                .get(if item == "permissions" {
                    "permissions"
                } else {
                    "statusLine"
                })
                .cloned();
            if value.as_ref().is_some_and(|value| !value.is_object()) {
                bail!("settings.json 的 {item} 不是对象");
            }
            Ok(value)
        }
        ProfileKind::Codex => codex_part(&toml_doc(bytes)?, item),
    }
}

fn value_base(value: Option<&Value>) -> String {
    value.map_or_else(
        || "missing".into(),
        |value| sha256_hex_bytes(&serde_json::to_vec(value).unwrap_or_default()),
    )
}

fn summary(value: Option<&Value>) -> String {
    match value {
        None => "未设置".into(),
        Some(Value::Object(map)) if map.is_empty() => "空对象".into(),
        Some(Value::Object(map)) => format!(
            "包含 {}",
            map.keys().cloned().collect::<Vec<_>>().join("、")
        ),
        Some(Value::Array(items)) => format!("{} 项", items.len()),
        Some(Value::String(text)) => format!("{} 行文本", text.lines().count()),
        Some(_) => "已设置".into(),
    }
}

fn changed_fields(source: Option<&Value>, target: Option<&Value>) -> Vec<String> {
    match (source, target) {
        (Some(Value::Object(from)), Some(Value::Object(to))) => {
            let mut keys: Vec<String> = from.keys().chain(to.keys()).cloned().collect();
            keys.sort();
            keys.dedup();
            keys.into_iter()
                .filter(|key| from.get(key) != to.get(key))
                .collect()
        }
        (Some(Value::Object(from)), None) => from.keys().cloned().collect(),
        (Some(Value::String(_)), _) => vec!["正文".into()],
        _ => Vec::new(),
    }
}

fn state(source: bool, same: bool, target: bool, blocked: bool) -> &'static str {
    if blocked {
        "blocked"
    } else if !source {
        "absent"
    } else if same {
        "same"
    } else if target {
        "different"
    } else {
        "new"
    }
}

fn row(
    item: &str,
    source: Option<&Value>,
    target: Option<&Value>,
    source_base: Option<String>,
    target_base: Option<String>,
    blocked: Option<String>,
) -> Value {
    json!({
        "item": item,
        "state": state(source.is_some(), source == target, target.is_some(), blocked.is_some()),
        "source": summary(source),
        "target": summary(target),
        "changed_fields": changed_fields(source, target),
        "source_base": source_base.unwrap_or_else(|| value_base(source)),
        "target_base": target_base.unwrap_or_else(|| value_base(target)),
        "blocked_reason": blocked,
    })
}

fn blocked_row(item: &str, error: &anyhow::Error) -> Value {
    row(
        item,
        None,
        None,
        None,
        None,
        Some(crate::sanitize::sanitize_diagnostic_message(&format!(
            "{error:#}"
        ))),
    )
}

fn settings_rows(ctx: &Context, def: &ProfileDef, dir: &Path) -> Result<Vec<Value>> {
    if !dir.is_dir() {
        bail!("独立环境尚未写入配置，先在环境卡片写入一次");
    }
    let source = read_source(&settings_file(ctx, def))?;
    let (_, target) = seed::read_target(&dir.join(def.kind.target_file()))?;
    SETTINGS
        .iter()
        .map(|item| {
            let from = setting_part(def.kind, source.as_deref(), item)?;
            let to = setting_part(def.kind, target.as_deref(), item)?;
            Ok(row(item, from.as_ref(), to.as_ref(), None, None, None))
        })
        .collect()
}

fn domain_row(ctx: &Context, req: &Request, def: &ProfileDef, item: &str) -> Result<Value> {
    let (command, main_id, target_id, pointer) = match (item, def.kind) {
        ("hooks", ProfileKind::Claude) => (
            "hooks:status",
            "hooks:claude".to_string(),
            format!("hooks:profile:{}", def.id),
            "/observed/hooks",
        ),
        ("hooks", ProfileKind::Codex) => (
            "hooks:status",
            "hooks:codex".to_string(),
            format!("hooks:profile:{}", def.id),
            "/observed/hooks",
        ),
        ("rules", ProfileKind::Claude) => (
            "rules:status",
            "rules:claude_global".to_string(),
            format!("rules:profile:{}", def.id),
            "/observed/content",
        ),
        _ => (
            "rules:status",
            "rules:codex_global".to_string(),
            format!("rules:profile:{}", def.id),
            "/observed/content",
        ),
    };
    let status = if item == "hooks" {
        crate::native_hooks::handle(ctx, &sub(req, command, vec![]))?
    } else {
        crate::native_rules::handle(ctx, &sub(req, command, vec![]))?
    };
    let main =
        seed::element(&status, &main_id).ok_or_else(|| anyhow!("找不到默认环境的{item}目标"))?;
    let target =
        seed::element(&status, &target_id).ok_or_else(|| anyhow!("找不到独立环境的{item}目标"))?;
    let raw_source = main.pointer(pointer).filter(|v| !v.is_null());
    let raw_target = target.pointer(pointer).filter(|v| !v.is_null());
    let meaningful = |value: &Value| {
        if item == "hooks" {
            value.as_object().is_some_and(|map| !map.is_empty())
        } else {
            value.as_str().is_some_and(|text| !text.trim().is_empty())
        }
    };
    let source = raw_source.filter(|value| meaningful(value));
    let dest = raw_target.filter(|value| meaningful(value));
    let blocked = main
        .pointer("/observed/blocked_reason")
        .and_then(Value::as_str)
        .or_else(|| {
            target
                .pointer("/observed/blocked_reason")
                .and_then(Value::as_str)
        })
        .map(str::to_string);
    let source_base = main
        .pointer("/observed/base")
        .and_then(Value::as_str)
        .map(str::to_string);
    let target_base = target
        .pointer("/observed/base")
        .and_then(Value::as_str)
        .map(str::to_string);
    let blocked = blocked.or_else(|| {
        (source_base.is_none() || target_base.is_none())
            .then(|| "无法取得来源或目标写入基准".into())
    });
    Ok(row(item, source, dest, source_base, target_base, blocked))
}

fn mcp_rows(ctx: &Context, def: &ProfileDef) -> Result<Vec<Value>> {
    let inv = crate::native_mcp::inventory::load(ctx)?;
    let source_id = crate::native_mcp::model::canonicalize_agent_id(&def.base_agent);
    let source = inv
        .scan(&source_id)
        .ok_or_else(|| anyhow!("找不到默认环境的 MCP 目标"))?;
    let target = inv
        .scan(&def.id)
        .ok_or_else(|| anyhow!("找不到独立环境的 MCP 目标"))?;
    if let Some(err) = &source.error {
        bail!("默认环境 MCP 配置不可读：{err}");
    }
    if let Some(err) = &target.error {
        bail!("独立环境 MCP 配置不可读：{err}");
    }
    Ok(source
        .entries
        .iter()
        .map(|entry| {
            let dest = target.entry(&entry.name);
            let mut blocked = match &entry.view {
                Ok(view) if seed::has_literal_credentials(&source.target.format, view) => {
                    Some("来源条目含明文凭据，请在 MCP 页逐条处理".to_string())
                }
                Err(_) => Some("来源条目结构无法识别".to_string()),
                _ if dest.is_some_and(|entry| entry.view.is_err()) => {
                    Some("目标条目结构无法识别".to_string())
                }
                _ if inv.saved_row(&entry.name, &def.id).is_some() => {
                    Some("目标还有已保存的同名条目，请在 MCP 页处理".to_string())
                }
                _ => None,
            };
            let expected = match &entry.view {
                Ok(view) if blocked.is_none() => {
                    let dropped =
                        crate::native_mcp::entry::dropped_on_copy(view, &target.target.format);
                    if !dropped.is_empty() {
                        blocked = Some(format!(
                            "复制会丢失 {}，请在 MCP 页逐条处理",
                            dropped.join("、")
                        ));
                        None
                    } else {
                        let neutral = crate::native_mcp::entry::neutral_from_view(
                            &source.target.format,
                            view,
                        );
                        match crate::native_mcp::entry::render_new(&target.target.format, &neutral)
                        {
                            Ok(raw) => Some(Value::Object(raw)),
                            Err(err) => {
                                blocked = Some(crate::sanitize::sanitize_diagnostic_message(
                                    &format!("{err:#}"),
                                ));
                                None
                            }
                        }
                    }
                }
                _ => None,
            };
            row(
                &format!("mcp:{}", entry.name),
                Some(expected.as_ref().unwrap_or(&entry.raw)),
                dest.map(|entry| &entry.raw),
                Some(entry.base.clone()),
                Some(dest.map_or_else(|| "missing".into(), |entry| entry.base.clone())),
                blocked,
            )
        })
        .collect())
}

fn preview(ctx: &Context, req: &Request, def: &ProfileDef) -> Vec<Value> {
    let dir = match seed::config_dir(ctx, def) {
        Ok(dir) => dir,
        Err(err) => {
            return ["permissions", "statusline", "hooks", "rules", "mcp"]
                .iter()
                .map(|item| blocked_row(item, &err))
                .collect()
        }
    };
    let mut rows = match settings_rows(ctx, def, &dir) {
        Ok(rows) => rows,
        Err(err) => SETTINGS
            .iter()
            .map(|item| blocked_row(item, &err))
            .collect(),
    };
    for item in ["hooks", "rules"] {
        rows.push(domain_row(ctx, req, def, item).unwrap_or_else(|err| blocked_row(item, &err)));
    }
    match mcp_rows(ctx, def) {
        Ok(items) => rows.extend(items),
        Err(err) => rows.push(blocked_row("mcp", &err)),
    }
    rows
}

pub(super) fn status(ctx: &Context, req: &Request) -> Result<Value> {
    let (id, _) = only(req, false)?;
    let def = seed::find_def(ctx, &id)?;
    Ok(
        json!({ "domain": "profiles", "action": "shared-status", "element": format!("profiles:{id}"), "items": preview(ctx, req, &def) }),
    )
}

fn write_setting(
    ctx: &Context,
    req: &Request,
    def: &ProfileDef,
    item: &str,
    source_base: &str,
    target_base: &str,
) -> Result<()> {
    let dir = seed::config_dir(ctx, def)?;
    let source = read_source(&settings_file(ctx, def))?;
    let (path, previous) = seed::read_target(&dir.join(def.kind.target_file()))?;
    let from = setting_part(def.kind, source.as_deref(), item)?;
    let before = setting_part(def.kind, previous.as_deref(), item)?;
    if from.is_none()
        || value_base(from.as_ref()) != source_base
        || value_base(before.as_ref()) != target_base
    {
        bail!("来源或环境设置已变化，请刷新差异后重试");
    }
    let next = match def.kind {
        ProfileKind::Claude => {
            let mut doc = json_doc(previous.as_deref())?;
            doc.insert(
                if item == "permissions" {
                    "permissions"
                } else {
                    "statusLine"
                }
                .into(),
                from.unwrap(),
            );
            let mut bytes = serde_json::to_vec_pretty(&Value::Object(doc))?;
            bytes.push(b'\n');
            bytes
        }
        ProfileKind::Codex => {
            let source_doc = toml_doc(source.as_deref())?;
            let mut doc = toml_doc(previous.as_deref())?;
            if item == "permissions" {
                for key in seed::CODEX_PERMISSION_KEYS {
                    doc.remove(key);
                    if let Some(value) = source_doc.get(key) {
                        doc[key] = value.clone();
                    }
                }
            } else {
                if doc.get("tui").is_none() {
                    doc["tui"] = toml_edit::Item::Table(toml_edit::Table::new());
                }
                let source_tui = source_doc
                    .get("tui")
                    .and_then(toml_edit::Item::as_table_like);
                let target_tui = doc["tui"]
                    .as_table_like_mut()
                    .ok_or_else(|| anyhow!("tui 不是表"))?;
                for key in seed::CODEX_STATUSLINE_KEYS {
                    target_tui.remove(key);
                    if let Some(value) = source_tui.and_then(|table| table.get(key)) {
                        target_tui.insert(key, value.clone());
                    }
                }
            }
            let text = doc.to_string();
            let parsed: toml::Value = text.parse()?;
            let fatal = crate::native_agent_settings::codex_fatal_issues(&parsed);
            if !fatal.is_empty() {
                bail!("写入后 Codex 无法加载：{}", fatal.join("；"));
            }
            text.into_bytes()
        }
    };
    let mut conn = store::open_write(ctx)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    crate::native_mcp::rollback::atomic_write_bytes_checked(&path, &next, previous.as_deref())?;
    let mut guard = RollbackGuard::new();
    guard.record_written(&path, previous, next.clone());
    let finish = (|| -> Result<()> {
        let (_, written) = seed::read_target(&dir.join(def.kind.target_file()))?;
        if setting_part(def.kind, written.as_deref(), item)?
            != setting_part(def.kind, source.as_deref(), item)?
        {
            bail!("写后回读不一致，请检查环境配置");
        }
        store::record_operation(
            &tx,
            Some(&req.run_id),
            "profiles:shared-apply",
            Some(&format!("profiles:{}", def.id)),
            Some(&json!({"item": item, "after_digest": sha256_hex_bytes(&next)})),
        )?;
        Ok(())
    })();
    if let Err(err) = finish {
        drop(tx);
        if let Err(rollback) = guard.rollback() {
            return Err(anyhow!("更新失败：{err:#}；环境配置恢复失败：{rollback:#}"));
        }
        return Err(err);
    }
    if let Err(err) = tx.commit() {
        if let Err(rollback) = guard.rollback() {
            return Err(anyhow!(
                "审计提交失败：{err:#}；环境配置恢复失败：{rollback:#}"
            ));
        }
        return Err(err.into());
    }
    guard.commit();
    Ok(())
}

pub(super) fn apply(ctx: &Context, req: &Request) -> Result<Value> {
    let (id, selection) = only(req, true)?;
    let (item, source_base, target_base) = selection.ok_or_else(|| anyhow!("缺少要更新的项"))?;
    let def = seed::find_def(ctx, &id)?;
    let old = preview(ctx, req, &def);
    let current = old
        .iter()
        .find(|row| row["item"] == item)
        .ok_or_else(|| anyhow!("来源没有这个共用设置项"))?;
    if current["source_base"] != source_base || current["target_base"] != target_base {
        bail!("来源或环境配置已变化，请刷新差异后重试");
    }
    if !matches!(current["state"].as_str(), Some("new" | "different")) {
        bail!("这个共用设置当前不可更新，请刷新差异");
    }
    match item.as_str() {
        "permissions" | "statusline" => {
            write_setting(ctx, req, &def, &item, &source_base, &target_base)?
        }
        "hooks" | "rules" => {
            let (domain, main_id, target_id, pointer) = match (item.as_str(), def.kind) {
                ("hooks", ProfileKind::Claude) => (
                    "hooks",
                    "hooks:claude",
                    format!("hooks:profile:{id}"),
                    "/observed/hooks",
                ),
                ("hooks", ProfileKind::Codex) => (
                    "hooks",
                    "hooks:codex",
                    format!("hooks:profile:{id}"),
                    "/observed/hooks",
                ),
                ("rules", ProfileKind::Claude) => (
                    "rules",
                    "rules:claude_global",
                    format!("rules:profile:{id}"),
                    "/observed/content",
                ),
                _ => (
                    "rules",
                    "rules:codex_global",
                    format!("rules:profile:{id}"),
                    "/observed/content",
                ),
            };
            let status = if domain == "hooks" {
                crate::native_hooks::handle(ctx, &sub(req, "hooks:status", vec![]))?
            } else {
                crate::native_rules::handle(ctx, &sub(req, "rules:status", vec![]))?
            };
            let main = seed::element(&status, main_id).ok_or_else(|| anyhow!("找不到来源"))?;
            let value = main
                .pointer(pointer)
                .ok_or_else(|| anyhow!("来源内容不可读"))?;
            let main_base = main
                .pointer("/observed/base")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("来源基准不可读"))?;
            if main_base != source_base {
                bail!("来源已变化，请刷新差异");
            }
            let args = vec![
                "--only".into(),
                target_id,
                "--base".into(),
                target_base,
                "--value".into(),
                if domain == "hooks" {
                    serde_json::to_string(value)?
                } else {
                    value
                        .as_str()
                        .ok_or_else(|| anyhow!("规则不是文本"))?
                        .into()
                },
            ];
            if domain == "hooks" {
                crate::native_hooks::handle(ctx, &sub(req, "hooks:apply", args))?;
            } else {
                crate::native_rules::handle(ctx, &sub(req, "rules:apply", args))?;
            }
        }
        _ => {
            let name = item
                .strip_prefix("mcp:")
                .ok_or_else(|| anyhow!("不认识的项"))?;
            let source_id = crate::native_mcp::model::canonicalize_agent_id(&def.base_agent);
            let mut args = vec![
                "--only".into(),
                format!("mcp:{name}"),
                "--action".into(),
                "copy".into(),
                "--agent".into(),
                source_id,
                "--to".into(),
                id.clone(),
                "--base".into(),
                source_base,
            ];
            if target_base != "missing" {
                args.extend(["--replace".into(), "--to-base".into(), target_base]);
            }
            crate::native_mcp::dispatch(ctx, &sub(req, "mcp:apply", args))?;
        }
    }
    let after = preview(ctx, req, &def);
    let updated = after
        .iter()
        .find(|row| row["item"] == item)
        .ok_or_else(|| anyhow!("写后找不到该项"))?;
    if updated["state"] != "same" {
        bail!("写后回读未确认，请检查实际配置");
    }
    Ok(
        json!({ "domain": "profiles", "action": "shared-apply", "element": format!("profiles:{id}"), "item": item, "state": "updated" }),
    )
}
