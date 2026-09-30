//! MCP 条目的已建模视图、脱敏展示与写入变换（Claude JSON / Codex TOML / OpenCode JSON）。
//!
//! 条目一律以「原生形态的 JSON 映射」处理：Claude/OpenCode 就是文件里的对象，Codex
//! 是 TOML 表转成的 JSON。本模块只认识表单负责的字段（已建模字段），其余官方字段
//! 原样保留；编辑时在原对象上改这些键，键顺序与未建模字段都不动。

use anyhow::{anyhow, bail, Result};
use serde_json::{json, Map, Value};

use super::model::{placeholder_var, MARKER_KEY};
use super::policy_view::{is_valid_header_name, validate_remote_url, MAX_REMOTE_HEADERS};
use super::targets::{Entry, Format};

pub const LITERAL_MASK: &str = "<literal>";
pub const HIDDEN_MASK: &str = "<hidden>";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Transport {
    Stdio,
    Remote,
}

/// 请求头取值：引用（客户端运行时从环境取）或字面量。
#[derive(Clone, PartialEq, Debug)]
pub enum HeaderVal {
    Ref(String),
    Lit(String),
}

/// 已建模视图。取值均为文件里的原值（未脱敏），只在进程内使用。
#[derive(Clone, Debug)]
pub struct View {
    pub transport: Transport,
    pub command: String,
    pub args: Vec<String>,
    /// env 原值（OpenCode 的 `environment`）。
    pub env: Vec<(String, String)>,
    pub url: String,
    pub headers: Vec<(String, HeaderVal)>,
    /// 只有 Codex 建模。
    pub startup_timeout_sec: Option<i64>,
    /// Codex/OpenCode 的官方 `enabled`；Claude 文件条目恒 true。
    pub enabled: bool,
    pub legacy_marker: bool,
    /// 本工具不建模、写入时原样保留的官方字段。
    pub extra: Map<String, Value>,
    /// 文件里写的是 `cmd /c <命令> …`（Windows 上运行 npx 这类 .cmd 的写法）：`command`/`args`
    /// 已去掉这层包装，写回时再包上。
    pub cmd_wrapped: bool,
}

/// Windows 上实际是 `.cmd` 批处理、要经 `cmd /c` 才能被 MCP 客户端启动的命令
/// （Claude Code `/doctor` 会对裸 `npx` 报警；cc-switch 同样处理）。
const CMD_WRAP_COMMANDS: &[&str] = &["npx", "npm", "pnpm", "yarn", "node", "bun", "bunx", "deno"];

fn needs_cmd_wrap(command: &str) -> bool {
    CMD_WRAP_COMMANDS
        .iter()
        .any(|c| command.eq_ignore_ascii_case(c))
}

/// 去掉 `cmd /c` 包装：返回里面的命令与参数；不是这种写法时为 None。
fn unwrap_cmd(command: &str, args: &[String]) -> Option<(String, Vec<String>)> {
    let is_cmd = command.eq_ignore_ascii_case("cmd") || command.eq_ignore_ascii_case("cmd.exe");
    if is_cmd && args.len() >= 2 && args[0].eq_ignore_ascii_case("/c") && !args[1].trim().is_empty()
    {
        Some((args[1].clone(), args[2..].to_vec()))
    } else {
        None
    }
}

/// 写入时的命令与参数：要包的包成 `cmd`, `["/c", 命令, …]`。
fn wrapped_command(wrap: bool, command: &str, args: &[String]) -> (String, Vec<String>) {
    if wrap {
        let mut full = vec!["/c".to_string(), command.to_string()];
        full.extend(args.iter().cloned());
        ("cmd".to_string(), full)
    } else {
        (command.to_string(), args.to_vec())
    }
}

fn container_env_key(format: &Format) -> &'static str {
    if *format == Format::OpenCodeJson {
        "environment"
    } else {
        "env"
    }
}

/// env 原值若是该客户端能展开的变量引用，返回变量名。
pub fn env_ref(format: &Format, raw: &str) -> Option<String> {
    match format {
        Format::OpenCodeJson => opencode_ref(raw),
        _ => placeholder_var(raw).map(str::to_string),
    }
}

fn opencode_ref(raw: &str) -> Option<String> {
    let inner = raw.strip_prefix("{env:")?.strip_suffix('}')?;
    crate::secret_values::valid_var_name(inner).then(|| inner.to_string())
}

fn string_map(
    obj: &Map<String, Value>,
    key: &str,
    what: &str,
) -> std::result::Result<Vec<(String, String)>, String> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Object(map)) => {
            let mut out = Vec::new();
            for (k, v) in map {
                if k == MARKER_KEY {
                    continue;
                }
                match v.as_str() {
                    Some(s) => out.push((k.clone(), s.to_string())),
                    None => return Err(format!("{what}.{k} 必须是字符串")),
                }
            }
            Ok(out)
        }
        Some(_) => Err(format!("{what} 必须是对象")),
    }
}

fn string_array(value: &Value, what: &str) -> std::result::Result<Vec<String>, String> {
    let arr = value
        .as_array()
        .ok_or_else(|| format!("{what} 必须是字符串数组"))?;
    arr.iter()
        .enumerate()
        .map(|(i, item)| {
            item.as_str()
                .map(str::to_string)
                .ok_or_else(|| format!("{what}[{i}] 必须是字符串"))
        })
        .collect()
}

fn nonempty_command(value: Option<&Value>) -> std::result::Result<String, String> {
    match value {
        Some(Value::String(s)) if !s.trim().is_empty() => Ok(s.clone()),
        Some(Value::String(_)) => Err("command 不能为空字符串".into()),
        Some(_) => Err("command 必须是字符串".into()),
        None => Err("缺少 command".into()),
    }
}

/// 解析文件条目。结构非法返回 Err（原因不含取值），调用方显示为无效单元格。
pub fn parse(format: &Format, raw: &Value) -> std::result::Result<View, String> {
    let obj = raw
        .as_object()
        .ok_or_else(|| "条目必须是对象/表".to_string())?;
    let env_key = container_env_key(format);
    let env_map = obj.get(env_key);
    let legacy_marker = obj.contains_key(MARKER_KEY)
        || env_map
            .and_then(Value::as_object)
            .map_or(false, |m| m.contains_key(MARKER_KEY));
    let remote = obj.contains_key("url");
    let enabled = match format {
        Format::CodexToml | Format::OpenCodeJson => match obj.get("enabled") {
            None => true,
            Some(Value::Bool(b)) => *b,
            Some(_) => return Err("enabled 必须是布尔值".into()),
        },
        _ => true,
    };
    let mut view = View {
        transport: if remote {
            Transport::Remote
        } else {
            Transport::Stdio
        },
        command: String::new(),
        args: Vec::new(),
        env: Vec::new(),
        url: String::new(),
        headers: Vec::new(),
        startup_timeout_sec: None,
        enabled,
        legacy_marker,
        extra: Map::new(),
        cmd_wrapped: false,
    };
    let mut modeled: Vec<&str> = vec![MARKER_KEY];
    if matches!(format, Format::CodexToml | Format::OpenCodeJson) {
        modeled.push("enabled");
    }
    if *format == Format::OpenCodeJson {
        match obj.get("type") {
            None => {}
            Some(Value::String(kind)) if kind == "local" && !remote => {}
            Some(Value::String(kind)) if kind == "remote" && remote => {}
            Some(_) => {
                return Err("type 与条目形态不匹配（local 需 command，remote 需 url）".into())
            }
        }
    }
    if remote {
        view.url = obj
            .get("url")
            .and_then(Value::as_str)
            .ok_or_else(|| "url 必须是字符串".to_string())?
            .to_string();
        modeled.push("url");
        match format {
            Format::CodexToml => {
                for (name, var) in string_map(obj, "env_http_headers", "env_http_headers")? {
                    view.headers.push((name, HeaderVal::Ref(var)));
                }
                for (name, value) in string_map(obj, "http_headers", "http_headers")? {
                    if !view.headers.iter().any(|(n, _)| *n == name) {
                        view.headers.push((name, HeaderVal::Lit(value)));
                    }
                }
                modeled.extend(["env_http_headers", "http_headers"]);
            }
            _ => {
                for (name, value) in string_map(obj, "headers", "headers")? {
                    let reference = if *format == Format::OpenCodeJson {
                        opencode_ref(&value)
                    } else {
                        placeholder_var(&value).map(str::to_string)
                    };
                    view.headers.push((
                        name,
                        match reference {
                            Some(var) => HeaderVal::Ref(var),
                            None => HeaderVal::Lit(value),
                        },
                    ));
                }
                modeled.push("headers");
            }
        }
    } else {
        if *format == Format::OpenCodeJson {
            let mut items = string_array(
                obj.get("command")
                    .ok_or_else(|| "缺少 command".to_string())?,
                "command",
            )?;
            if items.is_empty() || items[0].trim().is_empty() {
                return Err("command 数组首项不能为空".into());
            }
            view.command = items.remove(0);
            view.args = items;
        } else {
            view.command = nonempty_command(obj.get("command"))?;
            view.args = match obj.get("args") {
                None | Some(Value::Null) => Vec::new(),
                Some(value) => string_array(value, "args")?,
            };
            modeled.push("args");
        }
        modeled.push("command");
        if let Some((command, args)) = unwrap_cmd(&view.command, &view.args) {
            view.command = command;
            view.args = args;
            view.cmd_wrapped = true;
        }
        view.env = string_map(obj, env_key, env_key)?;
        modeled.push(env_key);
        if *format == Format::CodexToml {
            if let Some(value) = obj.get("startup_timeout_sec") {
                let timeout = value
                    .as_i64()
                    .filter(|t| *t > 0)
                    .ok_or_else(|| "startup_timeout_sec 必须是正整数".to_string())?;
                view.startup_timeout_sec = Some(timeout);
            }
            modeled.push("startup_timeout_sec");
        }
    }
    for (key, value) in obj {
        if !modeled.contains(&key.as_str()) {
            view.extra.insert(key.clone(), value.clone());
        }
    }
    Ok(view)
}

/// 未建模字段中可以原样展示的（非凭据类）键；其余显示 `<hidden>`。
const EXTRA_SHOWN: &[&str] = &[
    "type",
    "cwd",
    "timeout",
    "alwaysLoad",
    "enabled_tools",
    "disabled_tools",
    "tool_timeout_sec",
    "startup_timeout_ms",
    "required",
    "bearer_token_env_var",
    "env_vars",
    "scopes",
    "auth",
    "oauth_resource",
    "default_tools_approval_mode",
    "experimental_environment",
];

/// env 的显示值：引用显示 `${VAR}`；不像凭据的字面量照原值显示（保存时原样写回，
/// 与 `reject_secret_literal` 同一判定）；像凭据的一律 `<literal>`（界面拿不到明文，保存时保留原值）。
fn mask_env_value(format: &Format, key: &str, raw: &str) -> String {
    match env_ref(format, raw) {
        Some(var) => format!("${{{var}}}"),
        None if reject_secret_literal(key, raw).is_ok() => raw.to_string(),
        None => LITERAL_MASK.to_string(),
    }
}

/// 被隐藏的原值给一个认得出的提示：够长的只露前 3 位与后 4 位，短的只说几位。
fn secret_hint(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    if chars.len() >= 12 {
        let head: String = chars[..3].iter().collect();
        let tail: String = chars[chars.len() - 4..].iter().collect();
        format!("{head}…{tail}")
    } else {
        format!("已隐藏，{} 位", chars.len())
    }
}

/// 显示成 `<literal>` 的那些值各自的提示，让界面说得出「原值是什么」而不交出明文：
/// env 里像凭据的字面量给头尾提示；headers 的字面量（写入只接受引用或保留原值）不像凭据时给完整值，
/// 像凭据时给头尾提示。
pub fn literal_hints(format: &Format, view: &View) -> Value {
    let mut env = Map::new();
    for (key, raw) in &view.env {
        if env_ref(format, raw).is_none() && reject_secret_literal(key, raw).is_err() {
            env.insert(key.clone(), json!(secret_hint(raw)));
        }
    }
    let mut headers = Map::new();
    for (name, value) in &view.headers {
        if let HeaderVal::Lit(raw) = value {
            let hint = if reject_secret_literal(name, raw).is_ok() {
                raw.clone()
            } else {
                secret_hint(raw)
            };
            headers.insert(name.clone(), json!(hint));
        }
    }
    json!({"env": env, "headers": headers})
}

fn mask_header(value: &HeaderVal) -> String {
    match value {
        HeaderVal::Ref(var) => format!("${{{var}}}"),
        HeaderVal::Lit(_) => LITERAL_MASK.to_string(),
    }
}

/// 脱敏后的已建模字段：env/headers 引用显示 `${VAR}`，字面量一律 `<literal>`。
pub fn masked_entry(format: &Format, view: &View) -> Value {
    let mut out = Map::new();
    match view.transport {
        Transport::Stdio => {
            out.insert("command".into(), json!(view.command));
            out.insert("args".into(), json!(view.args));
            let env: Map<String, Value> = view
                .env
                .iter()
                .map(|(k, v)| (k.clone(), Value::String(mask_env_value(format, k, v))))
                .collect();
            out.insert("env".into(), Value::Object(env));
            if let Some(timeout) = view.startup_timeout_sec {
                out.insert("startup_timeout_sec".into(), json!(timeout));
            }
        }
        Transport::Remote => {
            out.insert("url".into(), json!(view.url));
            let headers: Map<String, Value> = view
                .headers
                .iter()
                .map(|(k, v)| (k.clone(), Value::String(mask_header(v))))
                .collect();
            out.insert("headers".into(), Value::Object(headers));
        }
    }
    Value::Object(out)
}

pub fn masked_extra(view: &View) -> Value {
    let map: Map<String, Value> = view
        .extra
        .iter()
        .map(|(k, v)| {
            let shown = EXTRA_SHOWN.contains(&k.as_str())
                && (v.is_string() || v.is_number() || v.is_boolean() || v.is_array());
            let array_ok = !v.is_array()
                || v.as_array()
                    .map_or(false, |items| items.iter().all(|i| i.is_string()));
            (
                k.clone(),
                if shown && array_ok {
                    v.clone()
                } else {
                    Value::String(HIDDEN_MASK.to_string())
                },
            )
        })
        .collect();
    Value::Object(map)
}

/// 条目里引用的变量：(键, kind, 变量名)。kind 为 "env" 或 "header"。
pub fn referenced_vars(format: &Format, view: &View) -> Vec<(String, &'static str, String)> {
    let mut out = Vec::new();
    for (key, raw) in &view.env {
        if let Some(var) = env_ref(format, raw) {
            out.push((key.clone(), "env", var));
        }
    }
    for (name, value) in &view.headers {
        if let HeaderVal::Ref(var) = value {
            out.push((name.clone(), "header", var.clone()));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// 表单提交（--spec）
// ---------------------------------------------------------------------------

/// 表单里一个 env/header 的取值。
#[derive(Clone, Debug, PartialEq)]
pub enum SpecVal {
    /// `<literal>`：保留该键当前的实际值。
    Keep,
    /// `${VAR}`。
    Ref(String),
    /// 新写的字面量（仅 env）。
    Lit(String),
}

#[derive(Clone, Debug)]
pub struct Spec {
    pub transport: Transport,
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<(String, SpecVal)>,
    pub startup_timeout_sec: Option<i64>,
    pub url: String,
    pub headers: Vec<(String, SpecVal)>,
}

fn spec_value(raw: &Value, what: &str) -> Result<SpecVal> {
    let text = raw
        .as_str()
        .ok_or_else(|| anyhow!("{what} 的取值必须是字符串"))?;
    if text == LITERAL_MASK {
        return Ok(SpecVal::Keep);
    }
    if let Some(var) = placeholder_var(text) {
        return Ok(SpecVal::Ref(var.to_string()));
    }
    Ok(SpecVal::Lit(text.to_string()))
}

fn valid_env_key(key: &str) -> bool {
    !key.is_empty()
        && key != MARKER_KEY
        && !key
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || c == '=')
}

/// 解析 `--spec`：只接受已建模字段，未知键一律拒绝；错误消息不回显取值。
pub fn parse_spec(raw: &str) -> Result<Spec> {
    let value: Value =
        serde_json::from_str(raw).map_err(|_| anyhow!("--spec 不是合法 JSON 对象"))?;
    let obj = value
        .as_object()
        .ok_or_else(|| anyhow!("--spec 必须是 JSON 对象"))?;
    let remote = obj.contains_key("url");
    let allowed: &[&str] = if remote {
        &["url", "headers"]
    } else {
        &["command", "args", "env", "startup_timeout_sec"]
    };
    for key in obj.keys() {
        if !allowed.contains(&key.as_str()) {
            bail!(
                "--spec 不支持字段 '{}'（{}只能填 {}）",
                key,
                if remote {
                    "远程服务"
                } else {
                    "本地服务"
                },
                allowed.join("/")
            );
        }
    }
    let mut spec = Spec {
        transport: if remote {
            Transport::Remote
        } else {
            Transport::Stdio
        },
        command: String::new(),
        args: Vec::new(),
        env: Vec::new(),
        startup_timeout_sec: None,
        url: String::new(),
        headers: Vec::new(),
    };
    if remote {
        spec.url = obj
            .get("url")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|u| !u.is_empty())
            .ok_or_else(|| anyhow!("远程服务地址 url 不能为空"))?
            .to_string();
        if let Some(headers) = obj.get("headers") {
            let map = headers
                .as_object()
                .ok_or_else(|| anyhow!("headers 必须是对象"))?;
            if map.len() > MAX_REMOTE_HEADERS {
                bail!("headers 数量超出上限 {} 个", MAX_REMOTE_HEADERS);
            }
            for (name, raw) in map {
                if !is_valid_header_name(name) {
                    bail!("header 名称 '{}' 非法：仅允许 RFC 7230 token 字符", name);
                }
                let value = spec_value(raw, &format!("header {name}"))?;
                if matches!(value, SpecVal::Lit(_)) {
                    bail!("header {name} 的取值只能是 ${{VAR}} 引用或保留原值（<literal>）");
                }
                spec.headers.push((name.clone(), value));
            }
        }
    } else {
        spec.command = obj
            .get("command")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|c| !c.is_empty())
            .ok_or_else(|| anyhow!("启动命令 command 不能为空"))?
            .to_string();
        if let Some(args) = obj.get("args") {
            spec.args = string_array(args, "args").map_err(|e| anyhow!(e))?;
        }
        if let Some(env) = obj.get("env") {
            let map = env.as_object().ok_or_else(|| anyhow!("env 必须是对象"))?;
            for (key, raw) in map {
                if !valid_env_key(key) {
                    bail!("环境变量名 '{}' 非法", key);
                }
                spec.env
                    .push((key.clone(), spec_value(raw, &format!("环境变量 {key}"))?));
            }
        }
        if let Some(timeout) = obj.get("startup_timeout_sec") {
            if !timeout.is_null() {
                spec.startup_timeout_sec = Some(
                    timeout
                        .as_i64()
                        .filter(|t| *t > 0)
                        .ok_or_else(|| anyhow!("startup_timeout_sec 必须是正整数"))?,
                );
            }
        }
    }
    Ok(spec)
}

// ---------------------------------------------------------------------------
// 中立形态：新增、复制、写回快照共用的「已建模字段 + 取值来源」
// ---------------------------------------------------------------------------

/// 取值来源。`Ref.resolve = true` 表示写入时要解析成取值（本工具的
/// `${VAR}` 语义）；false 表示客户端运行时展开的引用，能保持引用就保持。
#[derive(Clone, Debug, PartialEq)]
pub enum NVal {
    Lit(String),
    Ref { var: String, resolve: bool },
}

#[derive(Clone, Debug)]
pub struct Neutral {
    pub transport: Transport,
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<(String, NVal)>,
    pub startup_timeout_sec: Option<i64>,
    pub url: String,
    pub headers: Vec<(String, NVal)>,
    /// Claude 远程 type（http/sse/ws）；None 按 http。
    pub remote_type: Option<String>,
}

/// 从文件条目视图得到中立形态（复制的来源）。
pub fn neutral_from_view(format: &Format, view: &View) -> Neutral {
    Neutral {
        transport: view.transport,
        command: view.command.clone(),
        args: view.args.clone(),
        env: view
            .env
            .iter()
            .map(|(k, raw)| {
                (
                    k.clone(),
                    match env_ref(format, raw) {
                        Some(var) => NVal::Ref {
                            var,
                            resolve: false,
                        },
                        None => NVal::Lit(raw.clone()),
                    },
                )
            })
            .collect(),
        startup_timeout_sec: view.startup_timeout_sec,
        url: view.url.clone(),
        headers: view
            .headers
            .iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    match v {
                        HeaderVal::Ref(var) => NVal::Ref {
                            var: var.clone(),
                            resolve: false,
                        },
                        HeaderVal::Lit(lit) => NVal::Lit(lit.clone()),
                    },
                )
            })
            .collect(),
        remote_type: view
            .extra
            .get("type")
            .and_then(Value::as_str)
            .map(str::to_string),
    }
}

/// 新增时由表单得到中立形态：env 的 `${VAR}` 按本工具语义写入时解析；
/// 新写的 env 字面量先过疑似凭据检测。
pub fn neutral_from_spec(spec: &Spec) -> Result<Neutral> {
    let mut env = Vec::new();
    for (key, value) in &spec.env {
        env.push((
            key.clone(),
            match value {
                SpecVal::Keep => bail!("新增服务时环境变量 {key} 没有可保留的原值"),
                SpecVal::Ref(var) => NVal::Ref {
                    var: var.clone(),
                    resolve: true,
                },
                SpecVal::Lit(lit) => {
                    reject_secret_literal(key, lit)?;
                    NVal::Lit(lit.clone())
                }
            },
        ));
    }
    let mut headers = Vec::new();
    for (name, value) in &spec.headers {
        headers.push((
            name.clone(),
            match value {
                SpecVal::Ref(var) => NVal::Ref {
                    var: var.clone(),
                    resolve: false,
                },
                _ => bail!("新增服务时 header {name} 只能写 ${{VAR}} 引用"),
            },
        ));
    }
    if spec.transport == Transport::Remote {
        validate_remote_url(&spec.url).map_err(|e| anyhow!("服务地址不合法: {e}"))?;
    }
    Ok(Neutral {
        transport: spec.transport,
        command: spec.command.clone(),
        args: spec.args.clone(),
        env,
        startup_timeout_sec: spec.startup_timeout_sec,
        url: spec.url.clone(),
        headers,
        remote_type: None,
    })
}

/// 新写的 env 字面量不得像凭据：取值只写进客户端文件，但界面与命令行参数都会经手它。
pub fn reject_secret_literal(key: &str, value: &str) -> Result<()> {
    if crate::native::credential_like_name(key)
        || crate::native::reject_secret_text(value).is_some()
    {
        bail!("环境变量 {key} 的取值疑似凭据，请改用 ${{VAR}} 引用（取值放进「设置 → 密钥」选择的外部文件）");
    }
    Ok(())
}

fn resolve_var(var: &str, missing: &mut Vec<String>) -> String {
    match crate::secret_values::lookup(var) {
        Some(value) => value,
        None => {
            missing.push(var.to_string());
            String::new()
        }
    }
}

fn check_missing(mut missing: Vec<String>) -> Result<()> {
    if missing.is_empty() {
        return Ok(());
    }
    missing.sort();
    missing.dedup();
    bail!("凭证变量未配置: {}", missing.join(", "))
}

/// 中立形态 → 目标客户端的原生条目（全新条目，不含任何管理标记）。
///
/// env：字面量照写；需解析的引用解析成取值；运行时引用在 Claude 保持 `${VAR}`、
/// OpenCode 写 `{env:VAR}`，Codex 不展开 env 引用，一律解析。headers：引用按客户端
/// 原生引用形式写（Claude `${VAR}`、Codex `env_http_headers`、OpenCode `{env:VAR}`），
/// 变量须能解析；字面量写到对应的字面量位置。
pub fn render_new(format: &Format, n: &Neutral) -> Result<Map<String, Value>> {
    let mut missing = Vec::new();
    let mut env = Map::new();
    for (key, value) in &n.env {
        let text = match value {
            NVal::Lit(lit) => lit.clone(),
            NVal::Ref { var, resolve } => match (format, resolve) {
                (Format::ClaudeJson, false) => format!("${{{var}}}"),
                (Format::OpenCodeJson, false) => format!("{{env:{var}}}"),
                _ => resolve_var(var, &mut missing),
            },
        };
        env.insert(key.clone(), Value::String(text));
    }
    let mut out = Map::new();
    match n.transport {
        Transport::Stdio => {
            check_missing(missing)?;
            // Windows 上新增的 npx 一类命令包成 `cmd /c`
            let (command, args) = wrapped_command(
                cfg!(windows) && needs_cmd_wrap(&n.command),
                &n.command,
                &n.args,
            );
            match format {
                // 不写原来没有、用户也没填的字段：环境变量为空就不写 env
                Format::ClaudeJson => {
                    out.insert("command".into(), json!(command));
                    out.insert("args".into(), json!(args));
                    if !env.is_empty() {
                        out.insert("env".into(), Value::Object(env));
                    }
                }
                Format::CodexToml => {
                    out.insert("command".into(), json!(command));
                    out.insert("args".into(), json!(args));
                    if let Some(timeout) = n.startup_timeout_sec {
                        out.insert("startup_timeout_sec".into(), json!(timeout));
                    }
                    if !env.is_empty() {
                        out.insert("env".into(), Value::Object(env));
                    }
                }
                Format::OpenCodeJson => {
                    let mut command = vec![command];
                    command.extend(args);
                    out.insert("type".into(), json!("local"));
                    out.insert("command".into(), json!(command));
                    if !env.is_empty() {
                        out.insert("environment".into(), Value::Object(env));
                    }
                }
                Format::Unsupported(fmt) => bail!("不支持的 MCP 配置格式: {fmt}"),
            }
        }
        Transport::Remote => {
            let mut refs = Map::new();
            let mut lits = Map::new();
            for (name, value) in &n.headers {
                match value {
                    NVal::Lit(lit) => {
                        lits.insert(name.clone(), Value::String(lit.clone()));
                    }
                    NVal::Ref { var, resolve } => {
                        if *resolve {
                            lits.insert(
                                name.clone(),
                                Value::String(resolve_var(var, &mut missing)),
                            );
                        } else {
                            if !crate::secret_values::is_set(var) {
                                missing.push(var.clone());
                            }
                            refs.insert(name.clone(), Value::String(var.clone()));
                        }
                    }
                }
            }
            check_missing(missing)?;
            match format {
                Format::ClaudeJson => {
                    out.insert(
                        "type".into(),
                        json!(n.remote_type.clone().unwrap_or_else(|| "http".into())),
                    );
                    out.insert("url".into(), json!(n.url));
                    let mut headers = lits;
                    for (name, var) in refs {
                        headers.insert(name, json!(format!("${{{}}}", var.as_str().unwrap())));
                    }
                    if !headers.is_empty() {
                        out.insert("headers".into(), Value::Object(headers));
                    }
                }
                Format::CodexToml => {
                    let kind = n.remote_type.as_deref().unwrap_or("http");
                    if !matches!(kind, "http" | "streamable-http") {
                        bail!(
                            "Codex 只支持 streamable HTTP 远程服务（type={kind} 无法写入 Codex）"
                        );
                    }
                    out.insert("url".into(), json!(n.url));
                    if !refs.is_empty() {
                        out.insert("env_http_headers".into(), Value::Object(refs));
                    }
                    if !lits.is_empty() {
                        out.insert("http_headers".into(), Value::Object(lits));
                    }
                }
                Format::OpenCodeJson => {
                    out.insert("type".into(), json!("remote"));
                    out.insert("url".into(), json!(n.url));
                    let mut headers = lits;
                    for (name, var) in refs {
                        headers.insert(name, json!(format!("{{env:{}}}", var.as_str().unwrap())));
                    }
                    if !headers.is_empty() {
                        out.insert("headers".into(), Value::Object(headers));
                    }
                    out.insert("oauth".into(), json!(false));
                }
                Format::Unsupported(fmt) => bail!("不支持的 MCP 配置格式: {fmt}"),
            }
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// 编辑与启停：在原对象上只改已建模键
// ---------------------------------------------------------------------------

fn strip_marker(map: &mut Map<String, Value>, env_key: &str) {
    map.shift_remove(MARKER_KEY);
    if let Some(Value::Object(env)) = map.get_mut(env_key) {
        env.shift_remove(MARKER_KEY);
    }
}

/// 按表单重建一个字符串映射：保留原有键顺序，spec 里没有的键删除，新键追加。
fn rebuild_map(old: Option<&Value>, ordered: Vec<(String, String)>) -> Map<String, Value> {
    let mut out = Map::new();
    let wanted: std::collections::BTreeMap<String, String> = ordered.iter().cloned().collect();
    if let Some(Value::Object(old)) = old {
        for key in old.keys() {
            if let Some(value) = wanted.get(key) {
                out.insert(key.clone(), Value::String(value.clone()));
            }
        }
    }
    for (key, value) in ordered {
        if !out.contains_key(&key) {
            out.insert(key, Value::String(value));
        }
    }
    out
}

fn set_or_drop(map: &mut Map<String, Value>, key: &str, value: Map<String, Value>) {
    if value.is_empty() && !map.contains_key(key) {
        return;
    }
    map.insert(key.to_string(), Value::Object(value));
}

/// 编辑：返回新的原生条目。未建模字段、键顺序原样保留，旧标记顺带清除。
pub fn apply_edit(
    format: &Format,
    old: &Map<String, Value>,
    view: &View,
    spec: &Spec,
) -> Result<Map<String, Value>> {
    if spec.transport != view.transport {
        bail!("编辑不能切换传输方式（本地 stdio ↔ 远程 HTTP），请删除后重新新增");
    }
    let env_key = container_env_key(format);
    let mut new = old.clone();
    strip_marker(&mut new, env_key);
    let mut missing = Vec::new();
    match spec.transport {
        Transport::Stdio => {
            if spec.startup_timeout_sec.is_some() && *format != Format::CodexToml {
                bail!("startup_timeout_sec 只适用于 Codex");
            }
            let mut env = Vec::new();
            for (key, value) in &spec.env {
                let current = view.env.iter().find(|(k, _)| k == key).map(|(_, v)| v);
                let text = match value {
                    SpecVal::Keep => current
                        .cloned()
                        .ok_or_else(|| anyhow!("环境变量 {key} 没有可保留的原值"))?,
                    SpecVal::Ref(var) => match current {
                        Some(raw) if env_ref(format, raw).as_deref() == Some(var) => raw.clone(),
                        _ => resolve_var(var, &mut missing),
                    },
                    SpecVal::Lit(lit) => {
                        if current != Some(lit) {
                            reject_secret_literal(key, lit)?;
                        }
                        lit.clone()
                    }
                };
                env.push((key.clone(), text));
            }
            check_missing(missing)?;
            let env_map = rebuild_map(old.get(env_key), env);
            // 原来包着 `cmd /c` 的照样包；Windows 上用户改成 npx 一类命令时包上。没改命令的不动
            let wrap = view.cmd_wrapped
                || (cfg!(windows) && spec.command != view.command && needs_cmd_wrap(&spec.command));
            let (command, args) = wrapped_command(wrap, &spec.command, &spec.args);
            if *format == Format::OpenCodeJson {
                let mut full = vec![command];
                full.extend(args);
                new.insert("command".into(), json!(full));
            } else {
                new.insert("command".into(), json!(command));
                if !args.is_empty() || new.contains_key("args") {
                    new.insert("args".into(), json!(args));
                }
            }
            set_or_drop(&mut new, env_key, env_map);
            if *format == Format::CodexToml {
                match spec.startup_timeout_sec {
                    Some(t) => {
                        new.insert("startup_timeout_sec".into(), json!(t));
                    }
                    None => {
                        new.shift_remove("startup_timeout_sec");
                    }
                }
            }
        }
        Transport::Remote => {
            if spec.url != view.url {
                validate_remote_url(&spec.url).map_err(|e| anyhow!("服务地址不合法: {e}"))?;
            }
            new.insert("url".into(), json!(spec.url));
            let mut refs = Vec::new();
            let mut lits = Vec::new();
            for (name, value) in &spec.headers {
                let current = view.headers.iter().find(|(n, _)| n == name).map(|(_, v)| v);
                match value {
                    SpecVal::Keep => match current {
                        Some(HeaderVal::Ref(var)) => refs.push((name.clone(), var.clone())),
                        Some(HeaderVal::Lit(lit)) => lits.push((name.clone(), lit.clone())),
                        None => bail!("header {name} 没有可保留的原值"),
                    },
                    SpecVal::Ref(var) => {
                        if current != Some(&HeaderVal::Ref(var.clone()))
                            && !crate::secret_values::is_set(var)
                        {
                            missing.push(var.clone());
                        }
                        refs.push((name.clone(), var.clone()));
                    }
                    SpecVal::Lit(_) => {
                        bail!("header {name} 的取值只能是 ${{VAR}} 引用或保留原值（<literal>）")
                    }
                }
            }
            check_missing(missing)?;
            match format {
                Format::CodexToml => {
                    let refs_map = rebuild_map(old.get("env_http_headers"), refs);
                    let lits_map = rebuild_map(old.get("http_headers"), lits);
                    set_or_drop(&mut new, "env_http_headers", refs_map);
                    set_or_drop(&mut new, "http_headers", lits_map);
                }
                _ => {
                    let mut all: Vec<(String, String)> = Vec::new();
                    for (name, _) in &spec.headers {
                        if let Some((_, var)) = refs.iter().find(|(n, _)| n == name) {
                            all.push((
                                name.clone(),
                                if *format == Format::OpenCodeJson {
                                    format!("{{env:{var}}}")
                                } else {
                                    format!("${{{var}}}")
                                },
                            ));
                        } else if let Some((_, lit)) = lits.iter().find(|(n, _)| n == name) {
                            all.push((name.clone(), lit.clone()));
                        }
                    }
                    let headers = rebuild_map(old.get("headers"), all);
                    set_or_drop(&mut new, "headers", headers);
                }
            }
        }
    }
    Ok(new)
}

/// 官方 enabled 字段启停（Codex/OpenCode），顺带清除旧标记。
pub fn apply_enabled(
    format: &Format,
    old: &Map<String, Value>,
    enabled: bool,
) -> Map<String, Value> {
    let mut new = old.clone();
    strip_marker(&mut new, container_env_key(format));
    if enabled {
        // 启用就是回到默认：去掉 enabled，而不是写一个原来没有的 enabled = true
        new.remove("enabled");
    } else {
        new.insert("enabled".into(), Value::Bool(false));
    }
    new
}

/// 去掉旧标记后的原生条目（Claude 停用存快照前用）。
pub fn without_marker(format: &Format, old: &Map<String, Value>) -> Map<String, Value> {
    let mut new = old.clone();
    strip_marker(&mut new, container_env_key(format));
    new
}

/// Codex 新增条目走文本渲染：只接受本工具会生成的字段。
pub fn codex_entry_from_map(map: &Map<String, Value>) -> Result<Entry> {
    let mut entry = Entry::default();
    for (key, value) in map {
        match key.as_str() {
            "command" => entry.command = value.as_str().unwrap_or_default().to_string(),
            "args" => {
                entry.args = string_array(value, "args").map_err(|e| anyhow!(e))?;
            }
            "env" => {
                entry.env = value
                    .as_object()
                    .cloned()
                    .ok_or_else(|| anyhow!("env 必须是对象"))?;
            }
            "startup_timeout_sec" | "url" | "env_http_headers" | "http_headers" => {
                entry.extra.insert(key.clone(), value.clone());
            }
            other => bail!("条目含无法写入 Codex 新条目的字段 '{other}'"),
        }
    }
    Ok(entry)
}

/// 复制时未带过去的字段（未建模字段 + 目标不适用的已建模字段）；只列键名。
pub fn dropped_on_copy(view: &View, to: &Format) -> Vec<String> {
    let mut out: Vec<String> = view
        .extra
        .keys()
        .filter(|k| !matches!(k.as_str(), "type" | "oauth"))
        .cloned()
        .collect();
    if view.startup_timeout_sec.is_some() && *to != Format::CodexToml {
        out.push("startup_timeout_sec".into());
    }
    out.sort();
    out
}

#[cfg(test)]
mod cmd_wrap_tests {
    use super::*;

    #[test]
    fn cmd_c_包装读成里面的命令_编辑后照样包着_没改命令的不动() {
        let raw = json!({"command": "cmd", "args": ["/c", "npx", "-y", "srv@1.0.0"]});
        let view = parse(&Format::ClaudeJson, &raw).unwrap();
        assert_eq!(view.command, "npx");
        assert_eq!(view.args, vec!["-y", "srv@1.0.0"]);
        assert!(view.cmd_wrapped);

        // 改版本号：照样包着 cmd /c
        let spec = parse_spec(r#"{"command":"npx","args":["-y","srv@2.0.0"]}"#).unwrap();
        let old = raw.as_object().unwrap().clone();
        let new = apply_edit(&Format::ClaudeJson, &old, &view, &spec).unwrap();
        assert_eq!(new["command"], "cmd");
        assert_eq!(new["args"], json!(["/c", "npx", "-y", "srv@2.0.0"]));

        // 没包装的条目，命令没改：不加包装（只动用户改了的）
        let raw = json!({"command": "npx", "args": ["-y", "srv"]});
        let view = parse(&Format::ClaudeJson, &raw).unwrap();
        assert!(!view.cmd_wrapped);
        let spec = parse_spec(r#"{"command":"npx","args":["-y","srv@2.0.0"]}"#).unwrap();
        let new = apply_edit(&Format::ClaudeJson, raw.as_object().unwrap(), &view, &spec).unwrap();
        assert_eq!(new["command"], "npx");
    }

    #[test]
    fn 新增的_npx_只在_windows_上包_cmd_c() {
        let spec = parse_spec(r#"{"command":"npx","args":["-y","srv"]}"#).unwrap();
        let neutral = neutral_from_spec(&spec).unwrap();
        let out = render_new(&Format::CodexToml, &neutral).unwrap();
        if cfg!(windows) {
            assert_eq!(out["command"], "cmd");
            assert_eq!(out["args"], json!(["/c", "npx", "-y", "srv"]));
        } else {
            assert_eq!(out["command"], "npx");
        }
        // uvx 在 Windows 上是 .exe，不用包
        let spec = parse_spec(r#"{"command":"uvx","args":["srv"]}"#).unwrap();
        let out = render_new(&Format::ClaudeJson, &neutral_from_spec(&spec).unwrap()).unwrap();
        assert_eq!(out["command"], "uvx");
    }
}
