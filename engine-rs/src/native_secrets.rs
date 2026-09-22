//! Rust 原生 secrets 域：外部密钥文件状态、取值写入与旧数据库凭据迁移。
//!
//! 唯一元素 secrets:mcp-env：事实源为 store::effective_policy 与原生 MCP
//! 定义，仅纳入已启用定义；对每个合法 ${VAR} 占位符（stdio env 与远程
//! headers 同级）输出确定性行 {server, env_key, placeholder, resolved, kind}，
//! kind 为 "env" 或 "header"；resolved 判断该变量当前能否取到值（先看用户选择的
//! 外部 `.env` 文件，再回退进程环境），`stored` 说明取值是否来自外部文件。
//! 任何状态、操作记录与配置备份都不保存、不序列化、不输出变量值本身。历史畸形已启用 env/headers 定义
//! fail-closed，固定脱敏错误，不回显存储字面量/server/key/路径。
//!
//! `secrets:set --var NAME --value V` 只写用户明确选择的外部文件（空值即删除）；
//! `secrets:migrate-legacy` 需显式触发，先迁移旧表取值再删表。错误消息一律不回显取值。

use std::collections::BTreeMap;

use anyhow::{bail, Result};
use serde_json::{json, Value};

use crate::native::{Context, Request};
use crate::native_mcp::policy_view;
use crate::store;

const MALFORMED_MSG: &str = "MCP 环境变量定义存在非法内容（已脱敏），拒绝输出状态";

fn status(ctx: &Context) -> Result<Value> {
    let policy = store::effective_policy(ctx)?;
    let view = policy_view::library_view(&policy);
    let mut variables = Vec::new();
    let mut unresolved = 0i64;
    for (server, definition) in &view.definitions {
        if !policy_view::library_enabled(&view, server) {
            continue;
        }
        for (kind, field) in [("env", "env"), ("header", "headers")] {
            let Some(map_value) = definition.get(field) else {
                continue;
            };
            let Some(map) = map_value.as_object() else {
                bail!("{MALFORMED_MSG}");
            };
            for (key, value) in map {
                let Some(placeholder) = policy_view::placeholder_name(value) else {
                    bail!("{MALFORMED_MSG}");
                };
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
        }
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

/// 变量名推导：与界面接管时的规则一致（合法标识符直接用 env 键名）。
fn var_name_for(env_key: &str) -> String {
    if crate::secret_values::valid_var_name(env_key) {
        return env_key.to_string();
    }
    let mut out = String::new();
    for ch in env_key.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, '_');
    }
    out
}

/// 从某个助手的 MCP 配置里，把指定服务的明文 env 取值接到外部密钥文件。
///
/// 接管一条已有的 MCP 时，定义里只会留 `${VAR}` 引用；不把原来的取值一起接过来的话，
/// 同步时解析到的会是另一个值（或直接缺失），用户点「接管」的心智却是「保持原样」。
/// 只读该助手自己的配置文件，只取明文字面量（已经是 `${VAR}` 的跳过），
/// 回执只报接过来几个变量名，绝不回显取值。
fn adopt(ctx: &Context, req: &Request) -> Result<Value> {
    let mut agent: Option<String> = None;
    let mut server: Option<String> = None;
    let mut iter = req.args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--agent" => {
                if agent.is_some() {
                    bail!("参数错误: --agent 重复");
                }
                agent = Some(
                    iter.next()
                        .ok_or_else(|| anyhow::anyhow!("参数错误: --agent 缺少取值"))?
                        .clone(),
                );
            }
            "--server" => {
                if server.is_some() {
                    bail!("参数错误: --server 重复");
                }
                server = Some(
                    iter.next()
                        .ok_or_else(|| anyhow::anyhow!("参数错误: --server 缺少取值"))?
                        .clone(),
                );
            }
            other if other.starts_with("--") => bail!("参数错误: 存在不支持的选项"),
            _ => bail!("参数错误: 不接受位置参数"),
        }
    }
    let (Some(agent), Some(server)) = (agent, server) else {
        bail!("参数错误: 需要 --agent 与 --server");
    };

    let policy = store::effective_policy(ctx)?;
    let (targets, _) = crate::native_mcp::targets::agent_targets(&policy, &ctx.home_dir)?;
    let Some(target) = targets.into_iter().find(|t| t.agent == agent) else {
        // 该助手没有配置目标：没有可接过来的取值，不算错
        return Ok(json!({"domain": "secrets", "adopted": Vec::<String>::new()}));
    };
    crate::native_mcp::targets::validate_target_path(&target.path)?;

    let literals = read_server_env_literals(&target, &server);
    if literals.is_empty() {
        return Ok(json!({"domain": "secrets", "adopted": Vec::<String>::new()}));
    }

    let mut incoming = BTreeMap::new();
    for (env_key, value) in literals {
        let var = var_name_for(&env_key);
        if !crate::secret_values::valid_var_name(&var) {
            continue;
        }
        if incoming
            .insert(var.clone(), value.clone())
            .is_some_and(|previous| previous != value)
        {
            bail!("原配置中的变量 {var} 出现多个不同取值，拒绝自动接管");
        }
    }
    let adopted = crate::secret_values::merge_without_overwrite(ctx, &incoming)?;
    crate::secret_values::refresh(ctx);
    Ok(json!({"domain": "secrets", "adopted": adopted}))
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

/// 读该助手配置里指定服务的 env 明文取值（已是 `${VAR}` 的跳过）。
/// 读不了/解析不了都按「没有」处理，不让接管因为一个文件读不动而失败。
fn read_server_env_literals(
    target: &crate::native_mcp::targets::AgentTarget,
    server: &str,
) -> Vec<(String, String)> {
    use crate::native_mcp::targets::Format;
    let is_ref = |v: &str| v.starts_with("${") && v.ends_with('}');
    let mut out = Vec::new();
    match target.format {
        Format::ClaudeJson | Format::OpenCodeJson => {
            let Ok(Some(doc)) = crate::native_mcp::codecs::read_json_doc(&target.path) else {
                return out;
            };
            let container = if matches!(target.format, Format::OpenCodeJson) {
                "mcp"
            } else {
                "mcpServers"
            };
            let Some(env) = doc
                .get(container)
                .and_then(Value::as_object)
                .and_then(|m| m.get(server))
                .and_then(Value::as_object)
                .and_then(|m| m.get("environment").or_else(|| m.get("env")))
                .and_then(Value::as_object)
            else {
                return out;
            };
            for (key, value) in env {
                if key == "_MANAGED_BY" {
                    continue;
                }
                if let Some(text) = value.as_str() {
                    if !text.is_empty() && !is_ref(text) {
                        out.push((key.clone(), text.to_string()));
                    }
                }
            }
        }
        Format::CodexToml => {
            let Ok(text) = std::fs::read_to_string(&target.path) else {
                return out;
            };
            let Ok(doc) = text.parse::<toml::Value>() else {
                return out;
            };
            let Some(env) = doc
                .get("mcp_servers")
                .and_then(toml::Value::as_table)
                .and_then(|t| t.get(server))
                .and_then(toml::Value::as_table)
                .and_then(|t| t.get("env"))
                .and_then(toml::Value::as_table)
            else {
                return out;
            };
            for (key, value) in env {
                if key == "_MANAGED_BY" {
                    continue;
                }
                if let Some(text) = value.as_str() {
                    if !text.is_empty() && !is_ref(text) {
                        out.push((key.clone(), text.to_string()));
                    }
                }
            }
        }
        Format::Unsupported(_) => {}
    }
    out
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
        "secrets:adopt" => adopt(ctx, req),
        "secrets:source" => set_source(ctx, req),
        "secrets:migrate-legacy" => migrate_legacy(ctx, req),
        _ => bail!("secrets 域不支持命令 '{cmd}'"),
    }
}
