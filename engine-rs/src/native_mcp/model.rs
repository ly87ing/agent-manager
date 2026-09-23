//! MCP 通用小工具：标识校验、规范摘要与旧版标记常量

use serde_json::Value;

/// 旧版写入的条目级 / env 内标记：新版不再写入，读取时识别并剥离，只用于清理提示。
pub const MARKER_KEY: &str = "_MANAGED_BY";
pub const MARKER_VALUE: &str = "agent-manager";
/// 旧版在 Codex `[mcp_servers.<server>]` 表头前写入的注释行。
pub const LEGACY_TOML_COMMENT: &str = "# [agent-manager:managed]";

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

/// `${VAR}` → `VAR`；其余形态返回 None。
pub fn placeholder_var(s: &str) -> Option<&str> {
    let inner = s.strip_prefix("${")?.strip_suffix('}')?;
    crate::secret_values::valid_var_name(inner).then_some(inner)
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

/// 键排序后的紧凑 JSON：同一内容恒得同一文本，用于摘要。
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

/// 条目/快照摘要：界面拿它作写入基准（`--base`），写前在锁内重算比较。
pub fn digest_of(value: &Value) -> String {
    sha256_hex(&canonical_compact(value))
}
