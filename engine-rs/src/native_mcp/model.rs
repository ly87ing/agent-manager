//! MCP 数据模型、规范摘要与定义清理

use anyhow::{bail, Result};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

/// 旧版写入的条目级 / env 内标记：新版不再写入，读取时识别并剥离，只用于清理提示。
pub const MARKER_KEY: &str = "_MANAGED_BY";
pub const MARKER_VALUE: &str = "agent-manager";
/// 旧版在 Codex `[mcp_servers.<server>]` 表头前写入的注释行。
pub const LEGACY_TOML_COMMENT: &str = "# [agent-manager:managed]";
pub const TABLE_DDL: &str = "CREATE TABLE IF NOT EXISTS mcp_bindings (
    server TEXT NOT NULL,
    agent TEXT NOT NULL,
    version TEXT NOT NULL,
    definition_json TEXT NOT NULL,
    enabled INTEGER NOT NULL,
    assigned INTEGER NOT NULL,
    applied_digest TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (server, agent)
)";

pub const AGENT_ACTIONS: [&str; 5] = ["add", "update", "enable", "disable", "remove"];
pub const LIBRARY_ACTIONS: [&str; 4] = ["enable", "disable", "remove", "upgrade"];

pub fn canonicalize_agent_id(raw: &str) -> String {
    let s = raw.strip_prefix("agents:").unwrap_or(raw).trim();
    if s == "claude" || s == "claude-code" {
        "claude-code".to_string()
    } else {
        s.to_string()
    }
}

pub fn is_valid_server_name(server: &str) -> bool {
    !server.is_empty()
        && server
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

pub fn is_env_placeholder(s: &str) -> bool {
    if !s.starts_with("${") || !s.ends_with('}') {
        return false;
    }
    let inner = &s[2..s.len() - 1];
    !inner.is_empty()
        && inner.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && inner
            .chars()
            .next()
            .map_or(false, |c| c.is_ascii_alphabetic() || c == '_')
}

pub fn validate_env_placeholders(env_map: &Map<String, Value>) -> Result<()> {
    for (key, val) in env_map {
        let val_str = val
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("MCP 环境变量 '{}' 的值必须是字符串", key))?;
        if !is_env_placeholder(val_str) {
            bail!(
                "MCP 环境变量 '{}' 仅允许 ${{VAR}} 占位符引用系统环境变量，禁止直接持久化明文字面量",
                key
            );
        }
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq)]
pub struct Binding {
    pub version: String,
    pub definition: Value,
    pub enabled: bool,
    pub assigned: bool,
    pub applied_digest: String,
}

pub type Bindings = BTreeMap<String, BTreeMap<String, Binding>>;

/// 归属判定的唯一依据：目标文件里的同名条目属于 Agent Manager，当且仅当
/// 该 (server, agent) 绑定 assigned 且 enabled。文件里的旧 `_MANAGED_BY`
/// 标记不代表归属。
pub fn owns_entry(binding: Option<&Binding>) -> bool {
    binding.map_or(false, |b| b.assigned && b.enabled)
}

pub fn now_iso() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{}", secs)
}

pub fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

pub fn validate_binding(server: &str, agent: &str, binding: &Binding) -> Result<()> {
    if !is_valid_server_name(server) {
        bail!("mcp_bindings 中存在非法 server 名: '{}'", server);
    }
    if canonicalize_agent_id(agent) != agent {
        bail!(
            "mcp_bindings 中存在非规范 Agent key '{}'（应为 '{}'），拒绝读取",
            agent,
            canonicalize_agent_id(agent)
        );
    }
    if !binding.definition.is_object() {
        bail!("mcp_bindings 中 definition 必须是 JSON 对象");
    }
    if binding.assigned {
        if !is_sha256_hex(&binding.version) {
            bail!("绑定条目 {}@{} 版本必须是 SHA-256 摘要", server, agent);
        }
        let expected = compute_definition_version(&binding.definition);
        if binding.version != expected {
            bail!(
                "绑定条目 {}@{} 版本与定义摘要不一致，fail-closed 拒绝",
                server,
                agent
            );
        }
        if !binding.applied_digest.is_empty() && !is_sha256_hex(&binding.applied_digest) {
            bail!("绑定条目 {}@{} applied_digest 非法", server, agent);
        }
    }
    Ok(())
}

fn write_canonical(value: &Value, out: &mut String) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                out.push_str(&i.to_string());
            } else if let Some(u) = n.as_u64() {
                out.push_str(&u.to_string());
            } else if let Some(f) = n.as_f64() {
                if f.fract() == 0.0 && f.is_finite() && f.abs() < 1e15 {
                    out.push_str(&format!("{:.1}", f));
                } else {
                    out.push_str(&f.to_string());
                }
            }
        }
        Value::String(s) => write_json_string(s, out),
        Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_canonical(item, out);
            }
            out.push(']');
        }
        Value::Object(map) => {
            out.push('{');
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            for (index, key) in keys.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_json_string(key, out);
                out.push(':');
                write_canonical(&map[*key], out);
            }
            out.push('}');
        }
    }
}

fn write_json_string(value: &str, out: &mut String) {
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            _c => out.push(ch),
        }
    }
    out.push('"');
}

pub fn canonical_compact(value: &Value) -> String {
    let mut out = String::new();
    write_canonical(value, &mut out);
    out
}

pub fn sha256_hex(text: &str) -> String {
    sha256_hex_bytes(text.as_bytes())
}

pub fn sha256_hex_bytes(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect()
}

pub fn clean_definition(definition: &Value) -> Value {
    let empty = Map::new();
    let map = definition.as_object().unwrap_or(&empty);
    let mut out = Map::new();
    for (key, value) in map {
        if key.starts_with('_') || key == "enabled" || key == "agents" {
            continue;
        }
        out.insert(key.clone(), value.clone());
    }
    Value::Object(out)
}

pub fn mask_definition_for_status(definition: &Value) -> Value {
    let cleaned = clean_definition(definition);
    let mut obj = match cleaned {
        Value::Object(map) => map,
        other => return other,
    };
    // env（stdio）与 headers（远程）同级掩码：非 ${VAR} 占位符的值掩码为 <literal>
    for field in ["env", "headers"] {
        if let Some(map_val) = obj.get_mut(field) {
            if let Some(map) = map_val.as_object_mut() {
                for (_, v) in map.iter_mut() {
                    let is_ph = v.as_str().map_or(false, is_env_placeholder);
                    if !is_ph {
                        *v = Value::String("<literal>".to_string());
                    }
                }
            }
        }
    }
    Value::Object(obj)
}

pub fn mask_definition_with_agents_for_status(definition: &Value) -> Value {
    let mut masked = mask_definition_for_status(definition);
    if let Some(obj) = masked.as_object_mut() {
        if let Some(agents) = definition.get("agents") {
            obj.insert("agents".into(), agents.clone());
        } else {
            obj.insert("agents".into(), Value::Null);
        }
        if let Some(extra_args) = definition.get("extra_args_by_agent") {
            obj.insert("extra_args_by_agent".into(), extra_args.clone());
        }
    }
    masked
}

pub fn clean_definition_with_agents(definition: &Value) -> Value {
    let mut cleaned = clean_definition(definition);
    if let Some(obj) = cleaned.as_object_mut() {
        if let Some(agents) = definition.get("agents") {
            obj.insert("agents".into(), agents.clone());
        } else {
            obj.insert("agents".into(), Value::Null);
        }
        if let Some(extra_args) = definition.get("extra_args_by_agent") {
            obj.insert("extra_args_by_agent".into(), extra_args.clone());
        }
    }
    cleaned
}

pub fn compute_definition_version(definition: &Value) -> String {
    sha256_hex(&canonical_compact(&clean_definition(definition)))
}

/// 写入配置的实际条目摘要：覆盖除 command/args/env/environment/marker/type 外的全部
/// 实际字段（含未知字段），用于识别外部变更。绝不输出 env 明文。
pub fn compute_applied_digest(command: &Value, args: &Value, env: &Value, extra: &Value) -> String {
    let command_norm = match command {
        Value::String(_) => command.clone(),
        Value::Array(_) => command.clone(),
        _ => Value::Array(vec![]),
    };
    let args_norm = match args {
        Value::Array(_) => args.clone(),
        _ => Value::Array(vec![]),
    };
    let mut clean_env = Map::new();
    if let Value::Object(map) = env {
        for (key, value) in map {
            if key != MARKER_KEY {
                clean_env.insert(key.clone(), value.clone());
            }
        }
    }
    let mut extra_dict = Map::new();
    if let Value::Object(map) = extra {
        for (key, value) in map {
            if matches!(
                key.as_str(),
                "command" | "args" | "env" | "environment" | "type"
            ) || key == MARKER_KEY
            {
                continue;
            }
            extra_dict.insert(key.clone(), value.clone());
        }
    }
    let canonical = json!({
        "command": command_norm,
        "args": args_norm,
        "env": Value::Object(clean_env),
        "extra": Value::Object(extra_dict),
    });
    sha256_hex(&canonical_compact(&canonical))
}
