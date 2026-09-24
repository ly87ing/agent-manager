//! Rust 原生 secrets 域：外部密钥文件状态、取值写入与旧数据库凭据迁移。
//!
//! 唯一元素 secrets:mcp-env：事实源为各助手 MCP 配置里的实际条目与本工具里的
//! 停用快照，停用的条目同样计入（重新启用时要用到）；
//! 对每个变量引用（stdio env 与远程 headers 同级）输出确定性行
//! {server, env_key, placeholder, resolved, kind}，kind 为 "env" 或 "header"；
//! 字面量不是变量，不列出。resolved 判断该变量当前能否取到值（先看用户选择的
//! 外部 `.env` 文件，再回退进程环境），`stored` 说明取值是否来自外部文件。
//! 任何状态、操作记录与配置备份都不保存、不序列化、不输出变量值本身。
//!
//! `secrets:set --var NAME --value V` 只写用户明确选择的外部文件（空值即删除）；
//! MCP 停用时的明文转存在 native_mcp 内完成（见 native_mcp::snapshot）。
//! `secrets:migrate-legacy` 需显式触发，先迁移旧表取值再删表。错误消息一律不回显取值。

use std::collections::BTreeSet;

use anyhow::{bail, Result};
use serde_json::{json, Value};

use crate::native::{Context, Request};
use crate::native_mcp::entry::{self, referenced_vars};
use crate::native_mcp::targets::Format;
use crate::store;

type VarRef = (String, String, String, String);

fn add_entry_refs(refs: &mut BTreeSet<VarRef>, server: &str, format: &Format, value: &Value) {
    if let Ok(view) = entry::parse(format, value) {
        for (key, kind, var) in referenced_vars(format, &view) {
            refs.insert((server.to_string(), key, kind.to_string(), var));
        }
    }
}

/// MCP 实际条目与已保存项里引用的变量：(server, env_key, kind, 变量名)，去重。
fn mcp_references(ctx: &Context) -> Result<BTreeSet<VarRef>> {
    let inv = crate::native_mcp::inventory::load(ctx)?;
    let mut refs = BTreeSet::new();
    for scan in &inv.scans {
        for item in &scan.entries {
            add_entry_refs(&mut refs, &item.name, &scan.target.format, &item.raw);
        }
    }
    // 停用快照写回时要用到这些变量；未分配的已保存定义要等加到某个助手时才用得上，不列。
    for row in inv.saved.iter().filter(|row| !row.agent.is_empty()) {
        if let Some(scan) = inv.scan(&row.agent) {
            add_entry_refs(&mut refs, &row.server, &scan.target.format, &row.entry);
        }
    }
    Ok(refs)
}

/// 接入配置与独立环境引用的凭据变量：变量名 → 用在哪些地方。只看变量名，不读取值。
///
/// 独立环境写配置时把取值写进了自己的文件，改了取值要到那个环境重新写入；接入配置只在检查或
/// 建成独立环境时解析。界面据此说明「改了之后要做什么」。
fn provider_key_references(ctx: &Context) -> std::collections::BTreeMap<String, Vec<Value>> {
    let mut refs: std::collections::BTreeMap<String, Vec<Value>> = Default::default();
    if let Ok(defs) = crate::native_agents::load_definitions(ctx) {
        for def in defs
            .iter()
            .filter(|def| def.spec_ref().get("profile").is_none())
        {
            let Ok(settings) = crate::native_agent_settings::parse(def.spec_ref()) else {
                continue;
            };
            for (name, profile) in &settings.profiles {
                if let Some(var) = &profile.token_var {
                    refs.entry(var.clone()).or_default().push(json!({
                        "kind": "connection",
                        "agent": def.id,
                        "name": name,
                        "label": profile.label.clone().unwrap_or_else(|| name.clone()),
                    }));
                }
            }
        }
    }
    if let Ok(policy) = store::effective_policy(ctx) {
        for def in crate::native_profiles::model::load_definitions(&policy)
            .into_iter()
            .flatten()
        {
            let mut vars: Vec<String> = def
                .env
                .values()
                .filter_map(Value::as_str)
                .filter_map(|text| text.strip_prefix("${")?.strip_suffix('}'))
                .map(str::to_string)
                .collect();
            vars.extend(def.env_key.clone());
            vars.sort();
            vars.dedup();
            for var in vars {
                refs.entry(var).or_default().push(json!({
                    "kind": "profile",
                    "id": def.id,
                    "label": def.display_name,
                }));
            }
        }
    }
    refs
}

fn status(ctx: &Context) -> Result<Value> {
    let mut variables = Vec::new();
    let mut unresolved = 0i64;
    for (server, key, kind, placeholder) in mcp_references(ctx)? {
        let resolved = crate::secret_values::is_set(&placeholder);
        if !resolved {
            unresolved += 1;
        }
        variables.push(json!({
            "server": server,
            "env_key": key,
            "placeholder": placeholder,
            "resolved": resolved,
            // 取值是在这里填的还是来自进程环境：界面据此说明改哪儿才有用
            "stored": crate::secret_values::is_stored(&placeholder),
            "kind": kind,
        }));
    }
    variables.sort_by(|a, b| {
        (
            a["server"].as_str().unwrap_or(""),
            a["env_key"].as_str().unwrap_or(""),
            a["kind"].as_str().unwrap_or(""),
        )
            .cmp(&(
                b["server"].as_str().unwrap_or(""),
                b["env_key"].as_str().unwrap_or(""),
                b["kind"].as_str().unwrap_or(""),
            ))
    });
    let state = if unresolved == 0 {
        "managed-ok"
    } else {
        "missing"
    };
    let (source_path, source_error) = crate::secret_values::source_state();
    let source_ready = source_path.is_some() && source_error.is_none();
    let legacy_count = crate::secret_values::legacy_count(ctx)?;
    Ok(json!({
        "domain": "secrets",
        "elements": [{
            "id": "secrets:mcp-env",
            "domain": "secrets",
            "spec": {"sources": ["external file", "process env"]},
            "policy": {"enabled": true},
            "observed": {
                "variables": variables,
                "unresolved": unresolved,
                "source": {
                    "configured": source_path.is_some(),
                    "path": source_path,
                    "ready": source_ready,
                    "error": source_error,
                },
                "legacy_database_values": legacy_count,
                "provider_keys": provider_key_references(ctx)
                    .into_iter()
                    .map(|(var, used_by)| json!({
                        "variable": var,
                        "resolved": crate::secret_values::is_set(&var),
                        "stored": crate::secret_values::is_stored(&var),
                        "used_by": used_by,
                    }))
                    .collect::<Vec<_>>(),
            },
            "state": state,
            "freshness": {"state": "not-applicable", "current": Value::Null, "upstream": Value::Null, "checked_at": Value::Null, "probe": Value::Null, "cache_age_sec": Value::Null},
            "actions": [],
        }],
    }))
}

/// `secrets:set --var NAME --value V`：把取值写进外部文件；空值即删除该变量。
/// 错误消息只说变量种类，绝不回显取值。
fn set_value(ctx: &Context, req: &Request) -> Result<Value> {
    let mut var: Option<String> = None;
    let mut value: Option<String> = None;
    let mut iter = req.args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--var" => {
                if var.is_some() {
                    bail!("参数错误: --var 重复");
                }
                let Some(next) = iter.next() else {
                    bail!("参数错误: --var 缺少变量名");
                };
                if !crate::secret_values::valid_var_name(next) {
                    bail!("参数错误: 变量名不合法（须以字母或下划线开头，只含字母、数字与下划线）");
                }
                var = Some(next.clone());
            }
            "--value" => {
                if value.is_some() {
                    bail!("参数错误: --value 重复");
                }
                let Some(next) = iter.next() else {
                    bail!("参数错误: --value 缺少取值");
                };
                value = Some(next.clone());
            }
            other if other.starts_with("--") => bail!("参数错误: 存在不支持的选项"),
            _ => bail!("参数错误: 不接受位置参数"),
        }
    }
    let Some(var) = var else {
        bail!("参数错误: 缺少 --var");
    };
    let value = value.unwrap_or_default();
    if value.contains('\0') {
        bail!("参数错误: 取值含非法字符");
    }

    crate::secret_values::set(ctx, &var, &value)?;
    crate::secret_values::refresh(ctx);

    Ok(json!({
        "domain": "secrets",
        "variable": var,
        "stored": !value.is_empty(),
    }))
}

fn set_source(ctx: &Context, req: &Request) -> Result<Value> {
    let mut path: Option<String> = None;
    let mut iter = req.args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--path" if path.is_none() => {
                path = Some(
                    iter.next()
                        .ok_or_else(|| anyhow::anyhow!("参数错误: --path 缺少取值"))?
                        .trim()
                        .to_string(),
                );
            }
            "--path" => bail!("参数错误: --path 重复"),
            other if other.starts_with("--") => bail!("参数错误: 存在不支持的选项"),
            _ => bail!("参数错误: 不接受位置参数"),
        }
    }
    let path = path.ok_or_else(|| anyhow::anyhow!("参数错误: 缺少 --path"))?;
    crate::secret_values::validate_source(ctx, &path)?;
    let mut conn = store::open_write(ctx)?;
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    // 与设置页各自只改自己拥有的字段；同一事务读取可避免并发保存时互相覆盖。
    let mut settings = store::settings_from_connection(&tx)?;
    settings.secrets_file_path = path.clone();
    store::write_settings(&tx, &settings)?;
    tx.commit()?;
    crate::secret_values::refresh(ctx);
    Ok(json!({"domain": "secrets", "configured": !path.is_empty()}))
}

fn migrate_legacy(ctx: &Context, req: &Request) -> Result<Value> {
    if !req.args.is_empty() {
        bail!("secrets:migrate-legacy 不接受参数");
    }
    let migrated = crate::secret_values::migrate_legacy(ctx)?;
    crate::secret_values::refresh(ctx);
    Ok(json!({"domain": "secrets", "migrated": migrated}))
}

pub(crate) fn handle(ctx: &Context, req: &Request) -> Result<Value> {
    let cmd = req.command.replace('.', ":");
    match cmd.as_str() {
        "secrets:status" => {
            let mut offline = false;
            for arg in &req.args {
                if arg == "--offline" && !offline {
                    offline = true;
                } else {
                    bail!("secrets:status 仅接受可选的单个 --offline 参数");
                }
            }
            status(ctx)
        }
        "secrets:set" => set_value(ctx, req),
        "secrets:source" => set_source(ctx, req),
        "secrets:migrate-legacy" => migrate_legacy(ctx, req),
        _ => bail!("secrets 域不支持命令 '{cmd}'"),
    }
}
