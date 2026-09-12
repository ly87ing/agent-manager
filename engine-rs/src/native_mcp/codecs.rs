//! MCP 配置文件读写与解析（Claude JSON / Codex TOML / OpenCode JSON）

use anyhow::{anyhow, bail, Context as _, Result};
use serde_json::{Map, Value};
use std::path::Path;

use super::model::{MARKER_KEY, MARKER_VALUE};
use super::targets::{validate_target_path, Entry, Format};

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

pub fn read_json_doc(path: &Path) -> Result<Option<Map<String, Value>>> {
    if !path.exists() {
        return Ok(None);
    }
    validate_target_path(path)?;
    let bytes =
        std::fs::read(path).with_context(|| format!("配置文件读取失败: {}", path.display()))?;
    Ok(Some(json_doc_from_bytes(path, &bytes)?))
}

pub fn extract_entry(path: &Path, format: Format, server: &str) -> Result<(Option<Entry>, bool)> {
    validate_target_path(path)?;
    match format {
        Format::ClaudeJson | Format::OpenCodeJson => {
            let Some(doc) = read_json_doc(path)? else {
                return Ok((None, false));
            };
            let container_key = if format == Format::OpenCodeJson {
                "mcp"
            } else {
                "mcpServers"
            };
            let container = match doc.get(container_key) {
                None | Some(Value::Null) => return Ok((None, false)),
                Some(Value::Object(map)) => map,
                Some(_) => bail!(
                    "配置文件中的 '{}' 不是对象，拒绝覆盖: {}",
                    container_key,
                    path.display()
                ),
            };
            let Some(raw) = container.get(server) else {
                return Ok((None, false));
            };
            let srv = raw.as_object().ok_or_else(|| {
                anyhow!(
                    "配置文件中 server '{}' 不是对象，拒绝覆盖: {}",
                    server,
                    path.display()
                )
            })?;
            let env_key = if format == Format::OpenCodeJson {
                "environment"
            } else {
                "env"
            };
            let env_map = match srv.get(env_key) {
                None => Map::new(),
                Some(Value::Object(map)) => map.clone(),
                Some(_) => bail!(
                    "配置文件中 server '{}' 的 {} 不是对象，拒绝覆盖: {}",
                    server,
                    env_key,
                    path.display()
                ),
            };
            let remote = srv.get("url").is_some();
            if format == Format::OpenCodeJson {
                match srv.get("type") {
                    None => {}
                    Some(Value::String(kind)) if kind == "local" && !remote => {}
                    Some(Value::String(kind)) if kind == "remote" && remote => {}
                    Some(Value::String(kind)) => bail!(
                        "OpenCode JSON 中 server '{}' 的 type 与条目形态不匹配（local 需 command，remote 需 url），当前为 '{}': {}",
                        server,
                        kind,
                        path.display()
                    ),
                    Some(_) => bail!(
                        "OpenCode JSON 中 server '{}' 的 type 必须是字符串: {}",
                        server,
                        path.display()
                    ),
                }
            }
            // 受管标记：远程条目统一在条目级；stdio 旧位置 env 内标记继续兼容
            let managed = srv.get(MARKER_KEY).and_then(Value::as_str) == Some(MARKER_VALUE)
                || env_map.get(MARKER_KEY).and_then(Value::as_str) == Some(MARKER_VALUE);
            let mut env = Map::new();
            for (k, v) in &env_map {
                if k == MARKER_KEY {
                    continue;
                }
                match v.as_str() {
                    Some(s) => {
                        env.insert(k.clone(), Value::String(s.to_string()));
                    }
                    None => bail!(
                        "配置文件中 server '{}' 的 {}.{} 必须是字符串，拒绝覆盖: {}",
                        server,
                        env_key,
                        k,
                        path.display()
                    ),
                }
            }
            let (command, args) = if remote {
                if let Some(headers_val) = srv.get("headers") {
                    match headers_val {
                        Value::Object(map) => {
                            for (hk, hv) in map {
                                if !hv.is_string() {
                                    bail!(
                                        "配置文件中 server '{}' 的 headers.{} 必须是字符串，拒绝覆盖: {}",
                                        server,
                                        hk,
                                        path.display()
                                    );
                                }
                            }
                        }
                        _ => bail!(
                            "配置文件中 server '{}' 的 headers 不是对象，拒绝覆盖: {}",
                            server,
                            path.display()
                        ),
                    }
                }
                (String::new(), Vec::new())
            } else if format == Format::OpenCodeJson {
                match srv.get("command") {
                    Some(Value::Array(arr)) => {
                        if arr.is_empty() {
                            bail!(
                                "OpenCode JSON 中 server '{}' 的 command 数组不能为空: {}",
                                server,
                                path.display()
                            );
                        }
                        let mut items = Vec::with_capacity(arr.len());
                        for (idx, item) in arr.iter().enumerate() {
                            match item.as_str() {
                                Some(s) => items.push(s.to_string()),
                                None => bail!(
                                    "OpenCode JSON 中 server '{}' 的 command[{}] 必须是字符串: {}",
                                    server,
                                    idx,
                                    path.display()
                                ),
                            }
                        }
                        if items[0].trim().is_empty() {
                            bail!(
                                "OpenCode JSON 中 server '{}' 的 command 首项不能为空字符串: {}",
                                server,
                                path.display()
                            );
                        }
                        let cmd = items.remove(0);
                        (cmd, items)
                    }
                    Some(_) => bail!(
                        "OpenCode JSON 中 server '{}' 的 command 必须是字符串数组: {}",
                        server,
                        path.display()
                    ),
                    None => bail!(
                        "OpenCode JSON 中 server '{}' 缺少 command 字段: {}",
                        server,
                        path.display()
                    ),
                }
            } else {
                let cmd = match srv.get("command") {
                    Some(Value::String(s)) if !s.trim().is_empty() => s.clone(),
                    Some(Value::String(_)) => bail!(
                        "Claude JSON 中 server '{}' 的 command 不能为空字符串: {}",
                        server,
                        path.display()
                    ),
                    Some(_) => bail!(
                        "Claude JSON 中 server '{}' 的 command 必须是字符串: {}",
                        server,
                        path.display()
                    ),
                    None => bail!(
                        "Claude JSON 中 server '{}' 缺少 command 字段: {}",
                        server,
                        path.display()
                    ),
                };
                let args = match srv.get("args") {
                    None => Vec::new(),
                    Some(Value::Array(arr)) => {
                        let mut res = Vec::with_capacity(arr.len());
                        for (idx, item) in arr.iter().enumerate() {
                            match item.as_str() {
                                Some(s) => res.push(s.to_string()),
                                None => bail!(
                                    "Claude JSON 中 server '{}' 的 args[{}] 必须是字符串: {}",
                                    server,
                                    idx,
                                    path.display()
                                ),
                            }
                        }
                        res
                    }
                    Some(_) => bail!(
                        "Claude JSON 中 server '{}' 的 args 必须是数组: {}",
                        server,
                        path.display()
                    ),
                };
                (cmd, args)
            };
            let mut extra = srv.clone();
            for key in ["command", "args", "env", "environment", "type"] {
                extra.remove(key);
            }
            Ok((
                Some(Entry {
                    command,
                    args,
                    env,
                    extra,
                }),
                managed,
            ))
        }
        Format::CodexToml => {
            if !path.exists() {
                return Ok((None, false));
            }
            let text = std::fs::read_to_string(path)
                .with_context(|| format!("TOML 配置文件读取失败: {}", path.display()))?;
            let parsed: toml::Value = toml::from_str(&text).map_err(|exc| {
                anyhow!(
                    "现有 TOML 配置解析失败，拒绝覆盖: {} ({})",
                    path.display(),
                    exc
                )
            })?;
            let servers = match parsed.get("mcp_servers") {
                None => None,
                Some(toml::Value::Table(table)) => Some(table),
                Some(_) => bail!(
                    "Codex TOML 中的 mcp_servers 必须是 table: {}",
                    path.display()
                ),
            };
            let Some(servers) = servers else {
                return Ok((None, false));
            };
            let Some(raw) = servers.get(server) else {
                return Ok((None, false));
            };
            let table = raw.as_table().ok_or_else(|| {
                anyhow!(
                    "Codex TOML 中 server '{}' 必须是 table: {}",
                    server,
                    path.display()
                )
            })?;
            let env_table = match table.get("env") {
                None => toml::value::Table::new(),
                Some(toml::Value::Table(t)) => t.clone(),
                Some(_) => bail!(
                    "Codex TOML 中 server '{}' 的 env 必须是 table: {}",
                    server,
                    path.display()
                ),
            };
            let remote = table.get("url").is_some();
            // 受管标记：远程条目统一在条目表级；stdio 旧位置 env 表内标记继续兼容
            let managed = table.get(MARKER_KEY).and_then(toml::Value::as_str) == Some(MARKER_VALUE)
                || env_table.get(MARKER_KEY).and_then(toml::Value::as_str) == Some(MARKER_VALUE);
            let mut env = Map::new();
            for (key, value) in &env_table {
                if key == MARKER_KEY {
                    continue;
                }
                match value.as_str() {
                    Some(text) => {
                        env.insert(key.clone(), Value::String(text.to_string()));
                    }
                    None => bail!(
                        "TOML 条目标 '{}' env.{} 必须是字符串，拒绝覆盖: {}",
                        server,
                        key,
                        path.display()
                    ),
                }
            }
            if let Some(headers_val) = table.get("env_http_headers") {
                match headers_val {
                    toml::Value::Table(t) => {
                        for (hk, hv) in t {
                            if hv.as_str().is_none() {
                                bail!(
                                    "Codex TOML 中 server '{}' 的 env_http_headers.{} 必须是字符串: {}",
                                    server,
                                    hk,
                                    path.display()
                                );
                            }
                        }
                    }
                    _ => bail!(
                        "Codex TOML 中 server '{}' 的 env_http_headers 必须是 table: {}",
                        server,
                        path.display()
                    ),
                }
            }
            let command = if remote {
                String::new()
            } else {
                match table.get("command") {
                    Some(toml::Value::String(s)) if !s.trim().is_empty() => s.clone(),
                    Some(toml::Value::String(_)) => bail!(
                        "Codex TOML 中 server '{}' 的 command 不能为空字符串: {}",
                        server,
                        path.display()
                    ),
                    Some(_) => bail!(
                        "Codex TOML 中 server '{}' 的 command 必须是字符串: {}",
                        server,
                        path.display()
                    ),
                    None => bail!(
                        "Codex TOML 中 server '{}' 缺少 command 字段: {}",
                        server,
                        path.display()
                    ),
                }
            };
            let args = if remote {
                Vec::new()
            } else {
                match table.get("args") {
                    None => Vec::new(),
                    Some(toml::Value::Array(arr)) => {
                        let mut res = Vec::with_capacity(arr.len());
                        for (idx, item) in arr.iter().enumerate() {
                            match item.as_str() {
                                Some(s) => res.push(s.to_string()),
                                None => bail!(
                                    "Codex TOML 中 server '{}' 的 args[{}] 必须是字符串: {}",
                                    server,
                                    idx,
                                    path.display()
                                ),
                            }
                        }
                        res
                    }
                    Some(_) => bail!(
                        "Codex TOML 中 server '{}' 的 args 必须是数组: {}",
                        server,
                        path.display()
                    ),
                }
            };
            let mut extra = Map::new();
            for (key, val) in table {
                if key != "command" && key != "args" && key != "env" {
                    if key == "startup_timeout_sec" {
                        let timeout = val.as_integer().filter(|&t| t > 0).ok_or_else(|| {
                            anyhow!(
                                "Codex TOML 中 server '{}' 的 startup_timeout_sec 必须是正整数: {}",
                                server,
                                path.display()
                            )
                        })?;
                        extra.insert("startup_timeout_sec".into(), Value::Number(timeout.into()));
                    } else if let Ok(json_val) = serde_json::to_value(val) {
                        extra.insert(key.clone(), json_val);
                    }
                }
            }
            Ok((
                Some(Entry {
                    command,
                    args,
                    env,
                    extra,
                }),
                managed,
            ))
        }
        Format::Unsupported(ref fmt) => {
            bail!("不支持的 MCP 配置格式: {}", fmt);
        }
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
        // 远程条目：条目表内 url + 条目级受管标记；headers 以
        // env_http_headers（header 名 → 变量名）引用形式写入，零明文。
        let mut out = format!(
            "\n# [agent-manager:managed]\n[mcp_servers.{}]\nurl = {}\n",
            toml_key(server),
            toml_basic_string(url)
        );
        out.push_str(&format!(
            "{} = {}\n",
            MARKER_KEY,
            toml_basic_string(MARKER_VALUE)
        ));
        if let Some(Value::Object(headers)) = entry.extra.get("env_http_headers") {
            if !headers.is_empty() {
                out.push_str(&format!(
                    "[mcp_servers.{}.env_http_headers]\n",
                    toml_key(server)
                ));
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
        return out;
    }
    let mut out = format!(
        "\n# [agent-manager:managed]\n[mcp_servers.{}]\ncommand = {}\n",
        toml_key(server),
        toml_basic_string(&entry.command)
    );
    let args: Vec<String> = entry.args.iter().map(|a| toml_basic_string(a)).collect();
    out.push_str(&format!("args = [{}]\n", args.join(", ")));
    if let Some(Value::Number(timeout)) = entry.extra.get("startup_timeout_sec") {
        out.push_str(&format!("startup_timeout_sec = {}\n", timeout));
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
    out.push_str(&format!(
        "{} = {}\n",
        MARKER_KEY,
        toml_basic_string(MARKER_VALUE)
    ));
    out
}

pub fn toml_remove_server_text(content: &str, server: &str) -> String {
    let mut out = String::new();
    let mut skip = false;
    let mut pending_comment = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == "# [agent-manager:managed]" {
            pending_comment = true;
            continue;
        }
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            let header = &trimmed[1..trimmed.len() - 1];
            skip = header_is_server(header, server);
            if !skip && pending_comment {
                out.push_str("# [agent-manager:managed]\n");
            }
            pending_comment = false;
        } else if pending_comment {
            out.push_str("# [agent-manager:managed]\n");
            pending_comment = false;
        }
        if !skip {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
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

pub fn json_set_entry(doc: &mut Map<String, Value>, format: Format, server: &str, entry: &Entry) {
    let container_key = if format == Format::OpenCodeJson {
        "mcp"
    } else {
        "mcpServers"
    };
    let container = doc
        .entry(container_key.to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if !container.is_object() {
        *container = Value::Object(Map::new());
    }
    let map = container.as_object_mut().unwrap();
    if entry.extra.contains_key("url") {
        // 远程条目：codec 推导 type 常量（Claude "http" / OpenCode "remote"），
        // url/headers 引用形式来自 entry.extra，受管标记统一条目级。
        let mut obj = Map::new();
        let kind = if format == Format::OpenCodeJson {
            "remote"
        } else {
            "http"
        };
        obj.insert("type".into(), Value::String(kind.into()));
        for (k, v) in &entry.extra {
            obj.insert(k.clone(), v.clone());
        }
        obj.insert(MARKER_KEY.into(), Value::String(MARKER_VALUE.into()));
        map.insert(server.to_string(), Value::Object(obj));
        return;
    }
    if format == Format::OpenCodeJson {
        let mut cmd_array = vec![Value::String(entry.command.clone())];
        cmd_array.extend(entry.args.iter().cloned().map(Value::String));
        let mut env = entry.env.clone();
        env.insert(MARKER_KEY.into(), Value::String(MARKER_VALUE.into()));
        let mut obj = Map::new();
        obj.insert("type".into(), Value::String("local".into()));
        obj.insert("command".into(), Value::Array(cmd_array));
        obj.insert("environment".into(), Value::Object(env));
        for (k, v) in &entry.extra {
            obj.insert(k.clone(), v.clone());
        }
        map.insert(server.to_string(), Value::Object(obj));
    } else {
        let mut obj = Map::new();
        obj.insert("command".into(), Value::String(entry.command.clone()));
        obj.insert(
            "args".into(),
            Value::Array(entry.args.iter().cloned().map(Value::String).collect()),
        );
        let mut env = entry.env.clone();
        env.insert(MARKER_KEY.into(), Value::String(MARKER_VALUE.into()));
        obj.insert("env".into(), Value::Object(env));
        for (k, v) in &entry.extra {
            obj.insert(k.clone(), v.clone());
        }
        map.insert(server.to_string(), Value::Object(obj));
    }
}

pub fn json_remove_entry(doc: &mut Map<String, Value>, format: Format, server: &str) {
    let container_key = if format == Format::OpenCodeJson {
        "mcp"
    } else {
        "mcpServers"
    };
    if let Some(Value::Object(map)) = doc.get_mut(container_key) {
        map.remove(server);
    }
}
