//! MCP 策略视图、定义过滤、环境变量解析与写入条目构造

use anyhow::{anyhow, bail, Result};
use serde_json::{json, Map, Value};

use super::model::canonicalize_agent_id;
use super::targets::{Entry, Format};

#[derive(Default)]
pub struct LibraryView {
    pub definitions: Map<String, Value>,
    pub switches: Map<String, Value>,
}

pub fn library_view(policy: &Value) -> LibraryView {
    let mut view = LibraryView::default();
    let Some(mcp) = policy.get("mcp").and_then(Value::as_object) else {
        return view;
    };
    for (key, value) in mcp {
        if key == "definitions" {
            if let Value::Object(defs) = value {
                for (name, def) in defs {
                    let deleted = def.get("_delete").and_then(Value::as_bool).unwrap_or(false);
                    if !deleted && def.is_object() {
                        view.definitions.insert(name.clone(), def.clone());
                    }
                }
            }
        } else if value.is_object() {
            view.switches.insert(key.clone(), value.clone());
        }
    }
    view
}

pub fn library_enabled(view: &LibraryView, server: &str) -> bool {
    if !view.definitions.contains_key(server) {
        return false;
    }
    view.switches
        .get(server)
        .and_then(|sw| sw.get("enabled"))
        .and_then(Value::as_bool)
        .unwrap_or(true)
}

pub fn allowed_agents(definition: &Value) -> Result<Option<Vec<String>>> {
    let Some(val) = definition.get("agents") else {
        return Ok(None);
    };
    if val.is_null() {
        return Ok(None);
    }
    let list = val
        .as_array()
        .ok_or_else(|| anyhow!("'agents' 字段必须是字符串数组或 null"))?;
    let mut out = Vec::new();
    for item in list {
        let s = item
            .as_str()
            .ok_or_else(|| anyhow!("'agents' 数组中的元素必须为字符串"))?;
        out.push(canonicalize_agent_id(s));
    }
    Ok(Some(out))
}

pub fn is_allowed(allowed: &Option<Vec<String>>, agent: &str) -> bool {
    allowed
        .as_ref()
        .map_or(true, |list| list.iter().any(|a| a == agent))
}

pub fn placeholder_name(value: &Value) -> Option<String> {
    let text = value.as_str()?;
    if text.starts_with("${") && text.ends_with('}') && text.len() > 3 {
        let inner = &text[2..text.len() - 1];
        if !inner.is_empty()
            && inner.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            && inner
                .chars()
                .next()
                .map_or(false, |c| c.is_ascii_alphabetic() || c == '_')
        {
            return Some(inner.to_string());
        }
    }
    None
}

pub fn validate_definition_env(definition: &Value) -> Result<()> {
    if let Some(env_val) = definition.get("env") {
        let env = env_val
            .as_object()
            .ok_or_else(|| anyhow!("MCP 定义中的 'env' 必须是键值对象，当前类型非法"))?;
        for (key, value) in env {
            if placeholder_name(value).is_none() {
                bail!(
                    "MCP 环境变量 '{}' 仅允许 ${{VAR}} 占位符引用系统环境变量，禁止直接持久化明文字面量",
                    key
                );
            }
        }
    }
    Ok(())
}

pub fn expected_extra_fields(
    definition: &Value,
    format: Format,
    server: &str,
) -> Result<Map<String, Value>> {
    let mut extra = Map::new();
    let timeout = validate_startup_timeout(definition, server)?;
    if format == Format::CodexToml {
        if let Some(timeout) = timeout {
            extra.insert("startup_timeout_sec".into(), Value::Number(timeout.into()));
        }
    }
    Ok(extra)
}

fn validate_startup_timeout(definition: &Value, server: &str) -> Result<Option<u64>> {
    let Some(value) = definition.get("startup_timeout_sec") else {
        return Ok(None);
    };
    let timeout = value
        .as_u64()
        .filter(|timeout| *timeout > 0)
        .ok_or_else(|| anyhow!("{}.startup_timeout_sec 必须是正整数", server))?;
    Ok(Some(timeout))
}

pub fn inspect_env(definition: &Value) -> (Map<String, Value>, Vec<String>) {
    let mut out = Map::new();
    let mut missing = Vec::new();
    let Some(env) = definition.get("env").and_then(Value::as_object) else {
        return (out, missing);
    };
    for (key, value) in env {
        if let Some(var) = placeholder_name(value) {
            let resolved = std::env::var(&var).is_ok();
            if !resolved {
                missing.push(var.clone());
            }
            out.insert(
                key.clone(),
                json!({
                    "placeholder": format!("${{{}}}", var),
                    "resolved": resolved
                }),
            );
        } else {
            out.insert(
                key.clone(),
                json!({
                    "placeholder": Value::Null,
                    "resolved": false
                }),
            );
        }
    }
    missing.sort();
    (out, missing)
}

pub fn resolve_env(definition: &Value, _server: &str) -> Result<Map<String, Value>> {
    let mut out = Map::new();
    let Some(env) = definition.get("env").and_then(Value::as_object) else {
        return Ok(out);
    };
    let mut missing = Vec::new();
    for (key, value) in env {
        let Some(var) = placeholder_name(value) else {
            bail!(
                "MCP 环境变量 '{}' 仅允许 ${{VAR}} 占位符引用系统环境变量，禁止直接持久化明文字面量",
                key
            );
        };
        match std::env::var(&var) {
            Ok(resolved) => {
                out.insert(key.clone(), Value::String(resolved));
            }
            Err(_) => missing.push(var),
        }
    }
    if !missing.is_empty() {
        missing.sort();
        bail!("凭证变量未配置: {}", missing.join(", "));
    }
    Ok(out)
}

pub fn validate_definition_stdio(def: &Value, server: &str) -> Result<()> {
    validate_startup_timeout(def, server)?;
    let is_remote =
        def.get("url").is_some() || def.get("type").and_then(Value::as_str) == Some("remote");
    if is_remote {
        bail!("远程 HTTP MCP 暂未实现，仅支持本地 stdio 模式: {}", server);
    }
    match def.get("command") {
        Some(Value::String(s)) if !s.trim().is_empty() => {}
        Some(Value::String(_)) => bail!("MCP server '{}' 的 command 不能为空字符串", server),
        Some(_) => bail!("MCP server '{}' 的 command 必须是字符串", server),
        None => bail!("MCP server '{}' 缺少 command，仅支持 stdio 模式", server),
    }
    if let Some(args_val) = def.get("args") {
        match args_val {
            Value::Array(arr) => {
                for (idx, item) in arr.iter().enumerate() {
                    if !item.is_string() {
                        bail!("MCP server '{}' 的 args[{}] 必须是字符串", server, idx);
                    }
                }
            }
            _ => bail!("MCP server '{}' 的 args 必须是字符串数组", server),
        }
    }
    if let Some(extra_map_val) = def.get("extra_args_by_agent") {
        match extra_map_val {
            Value::Object(map) => {
                for (agent_k, arr_val) in map {
                    match arr_val {
                        Value::Array(arr) => {
                            for (idx, item) in arr.iter().enumerate() {
                                if !item.is_string() {
                                    bail!(
                                        "MCP server '{}' 的 extra_args_by_agent.{}[{}] 必须是字符串",
                                        server,
                                        agent_k,
                                        idx
                                    );
                                }
                            }
                        }
                        _ => bail!(
                            "MCP server '{}' 的 extra_args_by_agent.{} 必须是字符串数组",
                            server,
                            agent_k
                        ),
                    }
                }
            }
            _ => bail!("MCP server '{}' 的 extra_args_by_agent 必须是对象", server),
        }
    }
    Ok(())
}

pub fn build_entry(definition: &Value, server: &str, format: Format, agent: &str) -> Result<Entry> {
    validate_definition_stdio(definition, server)?;
    validate_definition_env(definition)?;
    let command = definition
        .get("command")
        .and_then(Value::as_str)
        .unwrap()
        .to_string();
    let mut args: Vec<String> = match definition.get("args") {
        None => Vec::new(),
        Some(Value::Array(arr)) => arr
            .iter()
            .map(|item| item.as_str().unwrap().to_string())
            .collect(),
        _ => unreachable!(),
    };
    if let Some(extra_arr) = definition
        .get("extra_args_by_agent")
        .and_then(Value::as_object)
        .and_then(|m| m.get(agent))
        .and_then(Value::as_array)
    {
        for item in extra_arr {
            args.push(item.as_str().unwrap().to_string());
        }
    }
    let env = resolve_env(definition, server)?;
    let extra = expected_extra_fields(definition, format, server)?;
    Ok(Entry {
        command,
        args,
        env,
        extra,
    })
}
