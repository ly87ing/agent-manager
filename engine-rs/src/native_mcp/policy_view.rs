//! MCP 策略视图、定义过滤、环境变量/header 占位符解析与写入条目构造

use anyhow::{anyhow, bail, Result};
use serde_json::{json, Map, Value};

use super::model::{canonicalize_agent_id, MARKER_KEY};
use super::targets::{Entry, Format};

/// 远程定义 headers 数量上限
pub const MAX_REMOTE_HEADERS: usize = 20;

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

/// 远程定义判定：含 url 字段，或历史遗留 type=="remote"（等价按远程处理，
/// 后续校验强制 url 必填；错误消息一律不回显 url 正文）。
pub fn is_remote_definition(def: &Value) -> bool {
    def.get("url").is_some() || def.get("type").and_then(Value::as_str) == Some("remote")
}

/// HTTP header 名合法性（RFC 7230 token 字符集）
pub fn is_valid_header_name(name: &str) -> bool {
    !name.is_empty()
        && name.bytes().all(|b| {
            matches!(b,
                b'0'..=b'9' | b'a'..=b'z' | b'A'..=b'Z'
                | b'!' | b'#' | b'$' | b'%' | b'&' | b'\'' | b'*' | b'+' | b'-' | b'.'
                | b'^' | b'_' | b'`' | b'|' | b'~')
        })
}

/// 远程 url 白名单：仅 https:// 任意主机，或本机回环 http://localhost /
/// http://127.0.0.1（可带端口与路径）。错误消息不回显 url 正文。
fn validate_remote_url(url: &str) -> Result<()> {
    if url.is_empty() || url.chars().any(|c: char| c.is_whitespace()) {
        bail!("url 为空或包含非法字符");
    }
    let (https, rest) = if let Some(rest) = url.strip_prefix("https://") {
        (true, rest)
    } else if let Some(rest) = url.strip_prefix("http://") {
        (false, rest)
    } else {
        bail!("url 协议仅允许 https://（本机回环允许 http://localhost 或 http://127.0.0.1）");
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.is_empty() || authority.contains('@') {
        bail!("url 缺少合法主机（不允许 userinfo）");
    }
    let host = authority.split(':').next().unwrap_or("");
    if https {
        if host.is_empty() {
            bail!("url 缺少合法主机");
        }
        return Ok(());
    }
    if host == "localhost" || host == "127.0.0.1" {
        return Ok(());
    }
    bail!("http url 仅限本机回环 localhost / 127.0.0.1，其余一律使用 https://");
}

/// headers 校验（远程定义专用；policy:set 与 preflight 共用）：
/// 必须是对象、数量 ≤ MAX_REMOTE_HEADERS、名称为合法 token、
/// 值必须是 ${VAR} 占位符（禁止明文落盘）。
pub fn validate_definition_headers(definition: &Value) -> Result<()> {
    if let Some(headers_val) = definition.get("headers") {
        let headers = headers_val
            .as_object()
            .ok_or_else(|| anyhow!("MCP 定义中的 'headers' 必须是键值对象，当前类型非法"))?;
        if headers.len() > MAX_REMOTE_HEADERS {
            bail!("MCP 定义的 headers 数量超出上限 {} 个", MAX_REMOTE_HEADERS);
        }
        for (key, value) in headers {
            if !is_valid_header_name(key) {
                bail!("MCP header 名称 '{}' 非法：仅允许 RFC 7230 token 字符", key);
            }
            if placeholder_name(value).is_none() {
                bail!(
                    "MCP header '{}' 的值仅允许 ${{VAR}} 占位符引用系统环境变量，禁止持久化明文字面量",
                    key
                );
            }
        }
    }
    Ok(())
}

/// 未受管外部属性守卫的期望字段集（按 format + 传输形态产出）：
/// stdio 仅 Codex 消费 startup_timeout_sec；远程按 codec 推导常量与白名单
/// （Claude {type,url,headers,marker} / Codex {url,env_http_headers,marker} /
/// OpenCode {type,url,headers,oauth,marker}）。
pub fn expected_extra_fields(
    definition: &Value,
    format: Format,
    server: &str,
) -> Result<Map<String, Value>> {
    let mut extra = Map::new();
    if is_remote_definition(definition) {
        let keys: &[&str] = match format {
            Format::ClaudeJson => &["type", "url", "headers", MARKER_KEY],
            Format::CodexToml => &["url", "env_http_headers", MARKER_KEY],
            Format::OpenCodeJson => &["type", "url", "headers", "oauth", MARKER_KEY],
            Format::Unsupported(_) => &[],
        };
        for key in keys {
            extra.insert((*key).to_string(), Value::Null);
        }
        return Ok(extra);
    }
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

fn inspect_placeholder_map(definition: &Value, field: &str) -> (Map<String, Value>, Vec<String>) {
    let mut out = Map::new();
    let mut missing = Vec::new();
    let Some(map) = definition.get(field).and_then(Value::as_object) else {
        return (out, missing);
    };
    for (key, value) in map {
        if let Some(var) = placeholder_name(value) {
            let resolved = crate::secret_values::is_set(&var);
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

pub fn inspect_env(definition: &Value) -> (Map<String, Value>, Vec<String>) {
    inspect_placeholder_map(definition, "env")
}

/// 远程 headers 占位符解析状态（与 env 同级：只输出占位符与布尔 resolved）
pub fn inspect_headers(definition: &Value) -> (Map<String, Value>, Vec<String>) {
    inspect_placeholder_map(definition, "headers")
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
        match crate::secret_values::lookup(&var) {
            Some(resolved) => {
                out.insert(key.clone(), Value::String(resolved));
            }
            None => missing.push(var),
        }
    }
    if !missing.is_empty() {
        missing.sort();
        bail!("凭证变量未配置: {}", missing.join(", "));
    }
    Ok(out)
}

/// 远程 headers 引用的变量存在性检查（与 env 同级 fail-closed）：
/// 只校验当前进程环境是否包含变量，绝不读取或写出变量值。
pub fn resolve_header_vars(definition: &Value, _server: &str) -> Result<()> {
    let Some(headers) = definition.get("headers").and_then(Value::as_object) else {
        return Ok(());
    };
    let mut missing = Vec::new();
    for (key, value) in headers {
        let Some(var) = placeholder_name(value) else {
            bail!(
                "MCP header '{}' 仅允许 ${{VAR}} 占位符引用系统环境变量，禁止持久化明文字面量",
                key
            );
        };
        if !crate::secret_values::is_set(&var) {
            missing.push(var);
        }
    }
    if !missing.is_empty() {
        missing.sort();
        bail!("凭证变量未配置: {}", missing.join(", "));
    }
    Ok(())
}

/// 定义顶层校验：按传输形态分发。远程（含 url 或遗留 type=="remote"）与
/// stdio 字段互斥；stdio 定义照旧。
pub fn validate_definition(def: &Value, server: &str) -> Result<()> {
    if is_remote_definition(def) {
        validate_definition_remote(def, server)
    } else {
        validate_definition_stdio(def, server)
    }
}

fn validate_definition_remote(def: &Value, server: &str) -> Result<()> {
    for forbidden in ["command", "args", "env", "startup_timeout_sec"] {
        if def.get(forbidden).is_some() {
            bail!(
                "远程 MCP 定义 '{}' 不允许包含 stdio 专用字段 '{}'，两种传输字段互斥",
                server,
                forbidden
            );
        }
    }
    match def.get("url") {
        Some(Value::String(url)) => {
            validate_remote_url(url)
                .map_err(|e| anyhow!("远程 MCP 定义 '{}' 的 url 不合法: {}", server, e))?;
        }
        Some(_) => bail!("远程 MCP 定义 '{}' 的 url 必须是字符串", server),
        None => bail!("远程 MCP 定义 '{}' 缺少 url 字段", server),
    }
    validate_definition_headers(def)?;
    Ok(())
}

pub fn validate_definition_stdio(def: &Value, server: &str) -> Result<()> {
    validate_startup_timeout(def, server)?;
    let is_remote =
        def.get("url").is_some() || def.get("type").and_then(Value::as_str) == Some("remote");
    if is_remote {
        bail!(
            "MCP server '{}' 是远程定义（含 url），不允许混入 stdio 字段，两种传输字段互斥",
            server
        );
    }
    if def.get("headers").is_some() {
        bail!(
            "MCP server '{}' 的 headers 字段仅用于远程 HTTP 定义（需含 url）",
            server
        );
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
    validate_definition(definition, server)?;
    validate_definition_env(definition)?;
    if is_remote_definition(definition) {
        return build_remote_entry(definition, format);
    }
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

/// 远程条目构造：零明文落盘，headers 全部以引用形式写入——
/// Claude JSON 字面量 ${VAR}（客户端原生展开）；Codex TOML env_http_headers
/// （header 名 → 变量名）；OpenCode JSON {env:VAR} 插值。
fn build_remote_entry(definition: &Value, format: Format) -> Result<Entry> {
    let url = definition
        .get("url")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let empty = Map::new();
    let headers = definition
        .get("headers")
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    let mut extra = Map::new();
    extra.insert("url".into(), Value::String(url));
    match format {
        Format::ClaudeJson => {
            if !headers.is_empty() {
                extra.insert("headers".into(), Value::Object(headers.clone()));
            }
        }
        Format::OpenCodeJson => {
            if !headers.is_empty() {
                let mut converted = Map::new();
                for (name, value) in headers {
                    let var = placeholder_name(value).expect("headers 已通过占位符校验");
                    converted.insert(name.clone(), Value::String(format!("{{env:{var}}}")));
                }
                extra.insert("headers".into(), Value::Object(converted));
            }
            extra.insert("oauth".into(), Value::Bool(false));
        }
        Format::CodexToml => {
            if !headers.is_empty() {
                let mut converted = Map::new();
                for (name, value) in headers {
                    let var = placeholder_name(value).expect("headers 已通过占位符校验");
                    converted.insert(name.clone(), Value::String(var));
                }
                extra.insert("env_http_headers".into(), Value::Object(converted));
            }
        }
        Format::Unsupported(ref fmt) => bail!("不支持的 MCP 配置格式: {}", fmt),
    }
    Ok(Entry {
        command: String::new(),
        args: Vec::new(),
        env: Map::new(),
        extra,
    })
}
