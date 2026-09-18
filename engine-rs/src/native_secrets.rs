//! Rust 原生 secrets 域：只读 `secrets:status` 与写入取值的 `secrets:set`。
//!
//! 唯一元素 secrets:mcp-env：事实源为 store::effective_policy 与原生 MCP
//! 定义，仅纳入已启用定义；对每个合法 ${VAR} 占位符（stdio env 与远程
//! headers 同级）输出确定性行 {server, env_key, placeholder, resolved, kind}，
//! kind 为 "env" 或 "header"；resolved 判断该变量当前能否取到值（先看用户在
//! 界面上填过的本机取值，再回退进程环境），`stored` 说明取值是否存在本机库里。
//! 任何路径都不保存、不序列化、不输出变量值本身。外部私有密钥
//! 仓库在 Agent Manager 之外注入进程环境：不扫描、不定位、不克隆、不探测
//! git、不读取或暴露任何仓库/路径/env 文件。历史畸形已启用 env/headers 定义
//! fail-closed，固定脱敏错误，不回显存储字面量/server/key/路径。
//!
//! `secrets:set --var NAME --value V` 把取值写进本机库（空值即删除）。取值是明文，
//! 只在 apply 渲染目标配置时使用；错误消息一律不回显取值。

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
    Ok(json!({
        "domain": "secrets",
        "elements": [{
            "id": "secrets:mcp-env",
            "domain": "secrets",
            "spec": {"sources": ["saved", "process env"]},
            "policy": {"enabled": true},
            "observed": {"variables": variables, "unresolved": unresolved},
            "state": state,
            "freshness": {"state": "not-applicable", "current": Value::Null, "upstream": Value::Null, "checked_at": Value::Null, "probe": Value::Null, "cache_age_sec": Value::Null},
            "actions": [],
        }],
    }))
}

/// `secrets:set --var NAME --value V`：把取值写进本机库；空值即删除该变量。
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

    let mut conn = store::open_write(ctx)?;
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    crate::secret_values::ensure_table(&tx)?;
    crate::secret_values::set(&tx, &var, &value, &crate::native_mcp::model::now_iso())?;
    tx.commit()?;
    // 本次分发内后续读取也要看到新值
    crate::secret_values::refresh(ctx);

    Ok(json!({
        "domain": "secrets",
        "variable": var,
        "stored": !value.is_empty(),
    }))
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
        _ => bail!("secrets 域不支持命令 '{cmd}'"),
    }
}
