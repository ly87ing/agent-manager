//! Rust 原生 secrets 域：只读 secrets:status。
//!
//! 唯一元素 secrets:mcp-env：事实源为 store::effective_policy 与原生 MCP
//! 定义，仅纳入已启用定义；对每个合法 ${VAR} 占位符（stdio env 与远程
//! headers 同级）输出确定性行 {server, env_key, placeholder, resolved, kind}，
//! kind 为 "env" 或 "header"；resolved 仅判断当前进程环境是否包含 VAR
//! （存在性），不保存、不序列化、不输出变量值。外部私有密钥
//! 仓库在 Agent Manager 之外注入进程环境：不扫描、不定位、不克隆、不探测
//! git、不读取或暴露任何仓库/路径/env 文件。历史畸形已启用 env/headers 定义
//! fail-closed，固定脱敏错误，不回显存储字面量/server/key/路径。

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
                let resolved = std::env::var_os(&placeholder).is_some();
                if !resolved {
                    unresolved += 1;
                }
                variables.push(json!({
                    "server": server,
                    "env_key": key,
                    "placeholder": placeholder,
                    "resolved": resolved,
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
            "spec": {"sources": ["process env"]},
            "policy": {"enabled": true},
            "observed": {"variables": variables, "unresolved": unresolved},
            "state": state,
            "freshness": {"state": "not-applicable", "current": Value::Null, "upstream": Value::Null, "checked_at": Value::Null, "probe": Value::Null, "cache_age_sec": Value::Null},
            "actions": [],
        }],
    }))
}

pub(crate) fn handle(ctx: &Context, req: &Request) -> Result<Value> {
    let cmd = req.command.replace('.', ":");
    if cmd != "secrets:status" {
        bail!("secrets 域不支持命令 '{cmd}'");
    }
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
