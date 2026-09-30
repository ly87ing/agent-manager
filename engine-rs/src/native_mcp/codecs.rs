//! MCP 配置文件解析与 Codex TOML 文本渲染

use anyhow::{anyhow, bail, Result};
use serde_json::{Map, Value};
use std::path::Path;

use super::model::LEGACY_TOML_COMMENT;
use super::targets::Entry;

pub fn json_doc_from_bytes(path: &Path, bytes: &[u8]) -> Result<Map<String, Value>> {
    let text = std::str::from_utf8(bytes).map_err(|exc| {
        anyhow!(
            "配置文件不是有效 UTF-8，拒绝覆盖: {} ({})",
            path.display(),
            exc
        )
    })?;
    let value: Value = serde_json::from_str(text).map_err(|exc| {
        anyhow!(
            "配置文件 JSON 解析失败，拒绝覆盖: {} ({})",
            path.display(),
            exc
        )
    })?;
    match value {
        Value::Object(map) => Ok(map),
        _ => bail!("配置文件根节点不是对象，拒绝覆盖: {}", path.display()),
    }
}

pub fn toml_basic_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c < ' ' => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

pub fn toml_key(key: &str) -> String {
    if key
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        && !key.is_empty()
    {
        key.to_string()
    } else {
        toml_basic_string(key)
    }
}

pub fn render_entry_toml(server: &str, entry: &Entry) -> String {
    if let Some(Value::String(url)) = entry.extra.get("url") {
        // 远程条目：条目表内只写 url；引用型 header 写 env_http_headers
        // （header 名 → 变量名），字面量 header（只来自复制）写 http_headers。
        let mut out = format!(
            "\n[mcp_servers.{}]\nurl = {}\n",
            toml_key(server),
            toml_basic_string(url)
        );
        for field in ["env_http_headers", "http_headers"] {
            if let Some(Value::Object(headers)) = entry.extra.get(field) {
                if !headers.is_empty() {
                    out.push_str(&format!("[mcp_servers.{}.{}]\n", toml_key(server), field));
                    let mut keys: Vec<&String> = headers.keys().collect();
                    keys.sort();
                    for key in keys {
                        let value = headers[key].as_str().unwrap_or_default();
                        out.push_str(&format!(
                            "{} = {}\n",
                            toml_key(key),
                            toml_basic_string(value)
                        ));
                    }
                }
            }
        }
        return out;
    }
    let mut out = format!(
        "\n[mcp_servers.{}]\ncommand = {}\n",
        toml_key(server),
        toml_basic_string(&entry.command)
    );
    let args: Vec<String> = entry.args.iter().map(|a| toml_basic_string(a)).collect();
    out.push_str(&format!("args = [{}]\n", args.join(", ")));
    if let Some(Value::Number(timeout)) = entry.extra.get("startup_timeout_sec") {
        out.push_str(&format!("startup_timeout_sec = {}\n", timeout));
    }
    if entry.env.is_empty() {
        return out;
    }
    out.push_str(&format!("[mcp_servers.{}.env]\n", toml_key(server)));
    let mut keys: Vec<&String> = entry.env.keys().collect();
    keys.sort();
    for key in keys {
        let value = entry.env[key].as_str().unwrap_or_default();
        out.push_str(&format!(
            "{} = {}\n",
            toml_key(key),
            toml_basic_string(value)
        ));
    }
    out
}

/// 删除 `[mcp_servers.<server>]` 及其子表。被删表头紧邻的上一行若是旧版注释
/// `# [agent-manager:managed]` 一并删除；被删区域末尾紧贴下一个保留表头的注释/空行
/// 属于下一个表，原样保留。其余行（含换行符）逐字节保留。
pub fn toml_remove_server_text(content: &str, server: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    let mut skip = false;
    // 删除区域内、紧贴下一个表头之前的连续注释/空行
    let mut trailing: Vec<&str> = Vec::new();
    for line in content.split_inclusive('\n') {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            let header = &trimmed[1..trimmed.len() - 1];
            let removing = header_is_server(header, server);
            if skip && !removing {
                // 被删表前后的空行不叠加：前文已以空行结尾时，不再保留紧随的空行
                let prev_blank = out.last().map_or(false, |prev| prev.trim().is_empty());
                let skip_blank = if prev_blank {
                    trailing
                        .iter()
                        .take_while(|line| line.trim().is_empty())
                        .count()
                } else {
                    0
                };
                out.extend(trailing.drain(..).skip(skip_blank));
            }
            trailing.clear();
            if removing && !skip && out.last().map(|prev| prev.trim()) == Some(LEGACY_TOML_COMMENT)
            {
                out.pop();
            }
            skip = removing;
        } else if skip {
            if trimmed.is_empty() || trimmed.starts_with('#') {
                trailing.push(line);
            } else {
                trailing.clear();
            }
        }
        if !skip {
            out.push(line);
        }
    }
    out.concat()
}

fn header_is_server(header: &str, server: &str) -> bool {
    let prefix = "mcp_servers.";
    let Some(rest) = header.strip_prefix(prefix) else {
        return false;
    };
    let first = rest.split('.').next().unwrap_or("");
    let bare = first.trim_matches('"');
    bare == server
}
