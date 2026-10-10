//! 按指定账号的实际条目检查 MCP 协议；不代表助手进程已经加载，结果不落盘。
//!
//! - 命令型（stdio）：用用户的 PATH 启动条目里的命令，按 MCP 协议发 `initialize`、
//!   `notifications/initialized`、`tools/list`，拿到工具清单后关掉进程。
//! - 地址型（Streamable HTTP）：用系统的 curl 依次 POST 同样三条消息，带上条目的请求头；
//!   请求头经 stdin 交给 curl（`-H @-`），不出现在进程参数里。旧的 SSE 传输不检测。
//! - 命令经 `crate::process` 启动：Windows 上不弹窗、超时整组结束，没有代理变量时带上系统代理。
//!
//! 变量引用按各助手的写法仅从显式选择的密钥文件取值；有变量取不到值就不启动，
//! 直接说缺哪个。命令可能自己下载依赖（如 npx），所以只在用户点「测试连接」时运行。
//! 输出里出现的变量取值一律打码，再经统一脱敏。

use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};

use super::entry::{env_ref, HeaderVal, Transport, View};
use super::inventory;
use super::model::{canonicalize_agent_id, is_valid_server_name};
use super::targets::Format;
use crate::native::{Context, Request};

const PROTOCOL_VERSION: &str = "2025-06-18";
/// 命令型默认等这么久（首次 npx 要下载）；条目写了 startup_timeout_sec 时用它的。
const STDIO_TIMEOUT: Duration = Duration::from_secs(30);
const HTTP_TIMEOUT: Duration = Duration::from_secs(20);
/// 报告里最多列这么多个工具名。
const TOOL_NAMES: usize = 30;
const STDERR_TAIL: usize = 600;

struct Outcome {
    ok: bool,
    /// 停在哪一步：variables / start / initialize / tools / done。
    stage: &'static str,
    message: String,
    server: Option<Value>,
    tools: Option<Vec<String>>,
    detail: Option<String>,
    exit_code: Option<i32>,
}

impl Outcome {
    fn fail(stage: &'static str, message: impl Into<String>) -> Self {
        Self {
            ok: false,
            stage,
            message: message.into(),
            server: None,
            tools: None,
            detail: None,
            exit_code: None,
        }
    }
}

fn parse_args(req: &Request) -> Result<(String, String, Option<String>, Option<PathBuf>)> {
    let mut only = None;
    let mut agent = None;
    let mut base = None;
    let mut cwd = None;
    let mut iter = req.args.iter();
    while let Some(arg) = iter.next() {
        let slot = match arg.as_str() {
            "--only" => &mut only,
            "--agent" => &mut agent,
            "--base" => &mut base,
            "--cwd" => &mut cwd,
            other => bail!("mcp:check 不接受参数 '{other}'"),
        };
        let value = iter
            .next()
            .filter(|value| !value.starts_with("--"))
            .ok_or_else(|| anyhow!("参数 '{arg}' 缺少参数值"))?;
        if slot.replace(value.clone()).is_some() {
            bail!("参数 '{arg}' 重复");
        }
    }
    let only = only.ok_or_else(|| anyhow!("缺少 --only mcp:<name>"))?;
    let server = only
        .strip_prefix("mcp:")
        .filter(|name| is_valid_server_name(name))
        .ok_or_else(|| anyhow!("--only 只接受 mcp:<name>，得到 '{only}'"))?;
    let agent = agent.ok_or_else(|| anyhow!("缺少 --agent"))?;
    if base
        .as_ref()
        .is_some_and(|value| !super::model::is_sha256_hex(value))
    {
        bail!("--base 必须是条目的 64 位小写十六进制摘要");
    }
    let cwd = cwd.map(PathBuf::from);
    if cwd
        .as_ref()
        .is_some_and(|path| !path.is_absolute() || !path.is_dir())
    {
        bail!("--cwd 必须是已存在目录的绝对路径");
    }
    Ok((server.to_string(), canonicalize_agent_id(&agent), base, cwd))
}

/// 文本里的 `${NAME}`（Claude）/ `{env:NAME}`（OpenCode）按助手的写法换成取值。
/// Codex 不在字符串里展开变量，原样返回。取不到值时返回变量名。
fn expand(
    format: &Format,
    text: &str,
    used: &mut Vec<String>,
) -> std::result::Result<String, String> {
    let (open, close) = match format {
        Format::ClaudeJson => ("${", "}"),
        Format::OpenCodeJson => ("{env:", "}"),
        _ => return Ok(text.to_string()),
    };
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find(open) {
        let after = &rest[start + open.len()..];
        let Some(end) = after.find(close) else {
            return Err("MCP 变量引用不完整".into());
        };
        let inner = &after[..end];
        if !crate::secret_values::valid_var_name(inner) {
            return Err("MCP 变量引用无效或使用默认值".into());
        }
        let value = crate::secret_values::lookup_account(inner).map_err(|_| inner.to_string())?;
        used.push(value.clone());
        out.push_str(&rest[..start]);
        out.push_str(&value);
        rest = &after[end + close.len()..];
    }
    out.push_str(rest);
    Ok(out)
}

fn resolve_ref(name: &str, used: &mut Vec<String>) -> std::result::Result<String, String> {
    let value = crate::secret_values::lookup_account(name).map_err(|_| name.to_string())?;
    used.push(value.clone());
    Ok(value)
}

/// 去掉输出里的变量取值，再走统一脱敏。
fn scrub(text: &str, secrets: &[String]) -> String {
    let mut out = text.to_string();
    for secret in secrets.iter().filter(|value| value.len() >= 4) {
        out = out.replace(secret.as_str(), "***");
    }
    crate::sanitize::sanitize_diagnostic_message(&out)
}

fn request(id: u64, method: &str, params: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
}

fn initialize() -> Value {
    request(
        1,
        "initialize",
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": { "name": "agent-manager", "version": env!("CARGO_PKG_VERSION") },
        }),
    )
}

const INITIALIZED: &str = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;

/// 没等到响应的原因：进程关了输出（多半已退出），或到点了。
enum Waited {
    Exited,
    Timeout,
}

/// 一个子进程：stdout 按行进通道，stderr 只留结尾一段。
struct Running {
    child: Child,
    group: crate::process::Group,
    lines: Receiver<String>,
    stderr: Receiver<Vec<u8>>,
}

impl Running {
    fn spawn(mut command: Command, stdin: Option<&[u8]>) -> std::io::Result<Self> {
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let (mut child, group) = crate::process::spawn_group(&mut command)?;
        if let Some(data) = stdin {
            if let Some(mut pipe) = child.stdin.take() {
                let _ = pipe.write_all(data);
            }
        }
        let (tx, lines) = channel();
        let stdout = child.stdout.take().expect("piped stdout");
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        let (tx, stderr) = channel();
        let mut pipe = child.stderr.take().expect("piped stderr");
        std::thread::spawn(move || {
            let mut tail = Vec::new();
            let mut buf = [0u8; 1024];
            while let Ok(n) = pipe.read(&mut buf) {
                if n == 0 {
                    break;
                }
                tail.extend_from_slice(&buf[..n]);
                if tail.len() > STDERR_TAIL * 4 {
                    tail.drain(..tail.len() - STDERR_TAIL * 4);
                }
            }
            let _ = tx.send(tail);
        });
        Ok(Self {
            child,
            group,
            lines,
            stderr,
        })
    }

    fn send(&mut self, message: &str) -> bool {
        match self.child.stdin.as_mut() {
            Some(pipe) => pipe
                .write_all(format!("{message}\n").as_bytes())
                .and_then(|_| pipe.flush())
                .is_ok(),
            None => false,
        }
    }

    /// 等 id 对上的响应；中间的通知、日志行跳过。
    fn wait_for(&mut self, id: u64, deadline: Instant) -> std::result::Result<Value, Waited> {
        loop {
            let left = deadline
                .checked_duration_since(Instant::now())
                .ok_or(Waited::Timeout)?;
            match self.lines.recv_timeout(left) {
                Ok(line) => {
                    let text = line
                        .trim()
                        .strip_prefix("data:")
                        .unwrap_or(line.trim())
                        .trim();
                    if let Ok(value) = serde_json::from_str::<Value>(text) {
                        if value.get("id").and_then(Value::as_u64) == Some(id) {
                            return Ok(value);
                        }
                    }
                }
                Err(RecvTimeoutError::Timeout) => return Err(Waited::Timeout),
                Err(RecvTimeoutError::Disconnected) => return Err(Waited::Exited),
            }
        }
    }

    /// 剩下的 stdout 全部收齐（进程自己结束时用）。
    fn drain(&mut self, deadline: Instant) -> Vec<String> {
        let mut out = Vec::new();
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            match self.lines.recv_timeout(left) {
                Ok(line) => out.push(line),
                Err(_) => break,
            }
        }
        out
    }

    /// 关 stdin 让服务自己退出，稍等后结束整个进程组；返回 stderr 结尾。
    fn finish(mut self) -> (String, Option<i32>) {
        // Report only exits observed before our shutdown, never cleanup-induced exits.
        let exit_code = self
            .child
            .try_wait()
            .ok()
            .flatten()
            .and_then(|status| status.code());
        drop(self.child.stdin.take());
        let until = Instant::now() + Duration::from_millis(500);
        while Instant::now() < until {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let status = self.child.try_wait().ok().flatten();
        if status.is_none() {
            self.group.kill(&mut self.child);
        }
        let _ = self.child.wait();
        let tail = self
            .stderr
            .recv_timeout(Duration::from_secs(1))
            .unwrap_or_default();
        let text = String::from_utf8_lossy(&tail).to_string();
        let start = text
            .char_indices()
            .rev()
            .nth(STDERR_TAIL)
            .map(|(index, _)| index)
            .unwrap_or(0);
        (text[start..].trim().to_string(), exit_code)
    }
}

fn rpc_error(response: &Value) -> Option<String> {
    let error = response.get("error")?;
    Some(
        error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("服务返回错误")
            .to_string(),
    )
}

fn server_info(response: &Value) -> Option<Value> {
    response.pointer("/result/serverInfo").cloned()
}

fn valid_initialize(response: &Value) -> bool {
    response
        .pointer("/result/protocolVersion")
        .and_then(Value::as_str)
        .is_some()
        && response
            .pointer("/result/capabilities")
            .is_some_and(Value::is_object)
        && response
            .pointer("/result/serverInfo")
            .is_some_and(Value::is_object)
}

fn tool_names(response: &Value) -> Option<Vec<String>> {
    Some(
        response
            .pointer("/result/tools")?
            .as_array()?
            .iter()
            .filter_map(|tool| tool.get("name").and_then(Value::as_str).map(str::to_string))
            .collect(),
    )
}

/// Resolve programs from absolute PATH entries and known local install folders.
/// MCP checks do not execute user shell startup files to discover a command.
fn user_path(ctx: &Context) -> (Vec<PathBuf>, std::ffi::OsString) {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|paths| {
            std::env::split_paths(&paths)
                .filter(|path| path.is_absolute())
                .collect()
        })
        .unwrap_or_default();
    for dir in crate::native_agents::fallback_bin_dirs(&ctx.home_dir) {
        if !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    let joined = std::env::join_paths(&dirs).unwrap_or_default();
    (dirs, joined)
}

fn check_stdio(
    ctx: &Context,
    format: &Format,
    view: &View,
    cwd: &std::path::Path,
    timeout: Duration,
    tool_timeout: Duration,
) -> (Outcome, Vec<String>) {
    let mut secrets = Vec::new();
    let mut env = Vec::new();
    for (key, raw) in &view.env {
        let value = match env_ref(format, raw) {
            Some(var) => resolve_ref(&var, &mut secrets),
            None => expand(format, raw, &mut secrets),
        };
        match value {
            Ok(value) => env.push((key.clone(), value)),
            Err(var) => {
                return (
                    Outcome::fail(
                        "variables",
                        format!("变量 {var} 没有取值：到「设置 › 密钥」选择的外部文件里填上"),
                    ),
                    secrets,
                )
            }
        }
    }
    let mut args = Vec::new();
    for raw in &view.args {
        match expand(format, raw, &mut secrets) {
            Ok(value) => args.push(value),
            Err(var) => {
                return (
                    Outcome::fail("variables", format!("变量 {var} 没有取值")),
                    secrets,
                )
            }
        }
    }
    let command_text = match expand(format, &view.command, &mut secrets) {
        Ok(value) => value,
        Err(var) => {
            return (
                Outcome::fail("variables", format!("变量 {var} 没有取值")),
                secrets,
            )
        }
    };
    let (dirs, path) = user_path(ctx);
    let program = if command_text.contains('/') || command_text.contains('\\') {
        Some(PathBuf::from(&command_text))
    } else {
        crate::native_programs::locate::find_all(
            &crate::native_programs::locate::UserPath {
                dirs,
                stable: Vec::new(),
                error: None,
            },
            &command_text,
        )
        .into_iter()
        .next()
    };
    let Some(program) = program else {
        return (
            Outcome::fail("start", format!("在 PATH 里找不到命令 {command_text}")),
            secrets,
        );
    };
    let mut command = match crate::native_account_context::clean_command(ctx, &program) {
        Ok(command) => command,
        Err(err) => {
            return (
                Outcome::fail("start", format!("受控环境检查失败：{err}")),
                secrets,
            )
        }
    };
    command.envs(env);
    command
        .args(&args)
        .env("PATH", &path)
        .env("HOME", &ctx.home_dir)
        .current_dir(cwd);
    let deadline = Instant::now() + timeout;
    let mut running = match Running::spawn(command, None) {
        Ok(running) => running,
        Err(err) => return (Outcome::fail("start", format!("启动失败：{err}")), secrets),
    };
    let mut outcome = stdio_session(&mut running, deadline, timeout, tool_timeout);
    let (stderr, exit_code) = running.finish();
    if !outcome.ok {
        outcome.exit_code = exit_code;
    }
    if !outcome.ok && !stderr.is_empty() {
        outcome.detail = Some(stderr);
    }
    (outcome, secrets)
}

fn stdio_session(
    running: &mut Running,
    deadline: Instant,
    timeout: Duration,
    tool_timeout: Duration,
) -> Outcome {
    if !running.send(&initialize().to_string()) {
        return Outcome::fail("start", "命令启动后马上退出了");
    }
    let response = match running.wait_for(1, deadline) {
        Ok(response) => response,
        Err(Waited::Exited) => return Outcome::fail("initialize", "命令退出了，没有回应初始化"),
        Err(Waited::Timeout) => {
            return Outcome::fail(
                "initialize",
                format!(
                    "{} 秒内没有回应初始化（输出的可能不是 MCP 协议）",
                    timeout.as_secs()
                ),
            )
        }
    };
    if let Some(error) = rpc_error(&response) {
        return Outcome::fail("initialize", format!("初始化被拒绝：{error}"));
    }
    if !valid_initialize(&response) {
        return Outcome::fail("initialize", "响应里没有有效的初始化结果");
    }
    let server = server_info(&response);
    if !running.send(INITIALIZED) {
        return Outcome::fail("initialize", "发送初始化完成通知时命令退出了");
    }
    if response.pointer("/result/capabilities/tools").is_none() {
        return completed(server, None);
    }
    if !running.send(&request(2, "tools/list", json!({})).to_string()) {
        return Outcome::fail("tools", "读取工具列表时命令退出了");
    }
    match running.wait_for(2, Instant::now() + tool_timeout) {
        Ok(response) => tools_outcome(server, &response),
        Err(Waited::Exited) => Outcome::fail("tools", "命令退出了，没有返回工具列表"),
        Err(Waited::Timeout) => Outcome::fail(
            "tools",
            format!("{} 秒内没有返回工具列表", tool_timeout.as_secs()),
        ),
    }
}

fn tools_outcome(server: Option<Value>, response: &Value) -> Outcome {
    if let Some(error) = rpc_error(response) {
        return Outcome::fail("tools", format!("读取工具列表失败：{error}"));
    }
    match tool_names(response) {
        Some(tools) => completed(server, Some(tools)),
        None => Outcome::fail("tools", "响应里没有有效的工具列表"),
    }
}

fn completed(server: Option<Value>, tools: Option<Vec<String>>) -> Outcome {
    Outcome {
        ok: true,
        stage: "done",
        message: match &tools {
            Some(tools) => format!("连接成功，提供 {} 个工具", tools.len()),
            None => "初始化成功，服务未声明工具能力".to_string(),
        },
        server,
        tools,
        detail: None,
        exit_code: None,
    }
}

/// 一次 POST：返回（响应头里的会话 id，id 对上的 JSON-RPC 响应）。
fn http_post(
    ctx: &Context,
    url: &str,
    headers: &[(String, String)],
    session: Option<&str>,
    body: &Value,
    want: Option<u64>,
    deadline: Instant,
) -> std::result::Result<(Option<String>, Option<Value>), String> {
    super::policy_view::validate_remote_url(url)
        .map_err(|_| "远程地址不安全：仅允许 HTTPS 或本机 HTTP，没有发送请求".to_string())?;
    if url.chars().any(char::is_control) {
        return Err("远程地址包含控制字符，没有发送请求".into());
    }
    for (name, value) in headers {
        if !super::policy_view::is_valid_header_name(name) || value.chars().any(char::is_control) {
            return Err("请求头包含非法名称或控制字符，没有发送请求".into());
        }
    }
    if session.is_some_and(|id| id.chars().any(char::is_control)) {
        return Err("远程会话标识包含控制字符，没有发送请求".into());
    }
    let left = deadline
        .checked_duration_since(Instant::now())
        .ok_or("超时")?
        .as_secs()
        .max(1);
    let mut header_text = format!(
        "Content-Type: application/json\nAccept: application/json, text/event-stream\nMCP-Protocol-Version: {PROTOCOL_VERSION}\n"
    );
    for (name, value) in headers {
        header_text.push_str(&format!("{name}: {value}\n"));
    }
    if let Some(id) = session {
        header_text.push_str(&format!("Mcp-Session-Id: {id}\n"));
    }
    let (dirs, _) = user_path(ctx);
    let curl = crate::native_programs::locate::find_all(
        &crate::native_programs::locate::UserPath {
            dirs,
            stable: Vec::new(),
            error: None,
        },
        "curl",
    )
    .into_iter()
    .next()
    .ok_or_else(|| "在 PATH 中找不到 curl".to_string())?;
    let mut command = crate::native_account_context::clean_command(ctx, &curl)
        .map_err(|_| "无法建立受控 curl 环境".to_string())?;
    command.args([
        "--disable",
        "--globoff",
        "--proto",
        "=http,https",
        "-sS",
        "-i",
        "--max-time",
        &left.to_string(),
        "-X",
        "POST",
        "-H",
        "@-",
        "--data-binary",
        &body.to_string(),
        "--url",
        url,
    ]);
    let mut running = Running::spawn(command, Some(header_text.as_bytes()))
        .map_err(|err| format!("运行 curl 失败：{err}"))?;
    drop(running.child.stdin.take());
    let mut session_id = None;
    let mut status = None;
    let mut found = None;
    let mut body_lines = Vec::new();
    let mut in_body = false;
    loop {
        let Some(left) = deadline.checked_duration_since(Instant::now()) else {
            break;
        };
        let line = match running.lines.recv_timeout(left) {
            Ok(line) => line,
            Err(_) => break,
        };
        let line = line.trim_end_matches('\r').to_string();
        if !in_body {
            if line.starts_with("HTTP/") {
                status = line
                    .split_whitespace()
                    .nth(1)
                    .and_then(|code| code.parse::<u16>().ok());
            } else if line.is_empty() {
                // 100 Continue 之后还有一段响应头
                in_body = status.is_some_and(|code| code >= 200);
            } else if let Some((name, value)) = line.split_once(':') {
                if name.trim().eq_ignore_ascii_case("mcp-session-id") {
                    session_id = Some(value.trim().to_string());
                }
            }
            continue;
        }
        if let Some(id) = want {
            let text = line.strip_prefix("data:").unwrap_or(&line).trim();
            if let Ok(value) = serde_json::from_str::<Value>(text) {
                if value.get("id").and_then(Value::as_u64) == Some(id) {
                    found = Some(value);
                    break;
                }
            }
        }
        body_lines.push(line);
    }
    if found.is_none() {
        body_lines.extend(running.drain(Instant::now() + Duration::from_millis(200)));
    }
    let (stderr, _) = running.finish();
    let Some(code) = status else {
        return Err(if stderr.is_empty() {
            "没有收到响应".to_string()
        } else {
            stderr
        });
    };
    if !(200..300).contains(&code) {
        let body = body_lines.join(" ");
        let snippet: String = body.chars().take(200).collect();
        return Err(format!(
            "HTTP {code}{}",
            if snippet.trim().is_empty() {
                String::new()
            } else {
                format!("：{}", snippet.trim())
            }
        ));
    }
    if found.is_none() {
        if let Some(id) = want {
            // 普通 JSON 响应可能跨行
            let body = body_lines.join("\n");
            found = serde_json::from_str::<Value>(body.trim())
                .ok()
                .filter(|value| value.get("id").and_then(Value::as_u64) == Some(id));
        }
    }
    Ok((session_id, found))
}

fn check_remote(
    ctx: &Context,
    format: &Format,
    view: &View,
    raw: &Value,
    timeout: Duration,
    tool_timeout: Duration,
) -> (Outcome, Vec<String>) {
    let mut secrets = Vec::new();
    if raw.get("type").and_then(Value::as_str) == Some("sse") {
        return (
            Outcome::fail("start", "旧的 SSE 传输暂不支持测试，只测 Streamable HTTP"),
            secrets,
        );
    }
    let url = match expand(format, &view.url, &mut secrets) {
        Ok(url) => url,
        Err(var) => {
            return (
                Outcome::fail("variables", format!("变量 {var} 没有取值")),
                secrets,
            )
        }
    };
    if super::policy_view::validate_remote_url(&url).is_err() || url.chars().any(char::is_control) {
        return (
            Outcome::fail(
                "start",
                "远程地址展开后不安全：仅允许 HTTPS 或本机 HTTP，没有发送请求",
            ),
            secrets,
        );
    }
    let mut headers = Vec::new();
    for (name, value) in &view.headers {
        if !super::policy_view::is_valid_header_name(name) {
            return (
                Outcome::fail("variables", "请求头名称非法，没有发送请求"),
                secrets,
            );
        }
        let resolved = match value {
            HeaderVal::Ref(var) => resolve_ref(var, &mut secrets),
            HeaderVal::Lit(text) => expand(format, text, &mut secrets),
        };
        match resolved {
            Ok(value) if !value.chars().any(char::is_control) => {
                headers.push((name.clone(), value))
            }
            Ok(_) => {
                return (
                    Outcome::fail("variables", "请求头取值含控制字符，没有发送请求"),
                    secrets,
                )
            }
            Err(var) => {
                return (
                    Outcome::fail(
                        "variables",
                        format!("变量 {var} 没有取值：到「设置 › 密钥」里填上"),
                    ),
                    secrets,
                )
            }
        }
    }
    let deadline = Instant::now() + timeout;
    let (session, response) =
        match http_post(ctx, &url, &headers, None, &initialize(), Some(1), deadline) {
            Ok(result) => result,
            Err(err) => {
                return (
                    Outcome::fail("initialize", format!("初始化失败：{err}")),
                    secrets,
                )
            }
        };
    let Some(response) = response else {
        return (
            Outcome::fail(
                "initialize",
                "响应里没有初始化结果（地址可能不是 MCP 服务）",
            ),
            secrets,
        );
    };
    if let Some(error) = rpc_error(&response) {
        return (
            Outcome::fail("initialize", format!("初始化被拒绝：{error}")),
            secrets,
        );
    }
    if !valid_initialize(&response) {
        return (
            Outcome::fail("initialize", "响应里没有有效的初始化结果"),
            secrets,
        );
    }
    let server = server_info(&response);
    let initialized: Value = serde_json::from_str(INITIALIZED).expect("static json");
    if let Err(err) = http_post(
        ctx,
        &url,
        &headers,
        session.as_deref(),
        &initialized,
        None,
        deadline,
    ) {
        return (
            Outcome::fail("initialize", format!("发送初始化完成通知失败：{err}")),
            secrets,
        );
    }
    if response.pointer("/result/capabilities/tools").is_none() {
        return (completed(server, None), secrets);
    }
    let outcome = match http_post(
        ctx,
        &url,
        &headers,
        session.as_deref(),
        &request(2, "tools/list", json!({})),
        Some(2),
        Instant::now() + tool_timeout,
    ) {
        Ok((_, Some(response))) => tools_outcome(server, &response),
        Ok((_, None)) => Outcome::fail("tools", "响应里没有工具列表结果"),
        Err(err) => Outcome::fail("tools", format!("读取工具列表失败：{err}")),
    };
    (outcome, secrets)
}

/// `mcp:check --only mcp:<name> --agent <账号>`。
pub fn check(ctx: &Context, req: &Request) -> Result<Value> {
    let (server, agent, base, selected_cwd) = parse_args(req)?;
    let _account_lock = crate::native_secrets::lock_credential_account(ctx, &agent, false)?;
    let inv = inventory::load(ctx)?;
    let scan = inv
        .scan(&agent)
        .ok_or_else(|| anyhow!("没有账号 {agent} 的 MCP 配置"))?;
    if let Some(error) = &scan.error {
        bail!("{agent} 的 MCP 配置读不了：{error}");
    }
    let entry = scan
        .entry(&server)
        .ok_or_else(|| anyhow!("{agent} 里没有 MCP 服务 {server}"))?;
    let view = entry
        .view
        .as_ref()
        .map_err(|reason| anyhow!("条目无法识别：{reason}"))?;
    super::preflight::validate_entry_variables(ctx, &agent, &scan.target.format, view)?;
    if base.as_ref().is_some_and(|base| base != &entry.base) {
        bail!("{server} 的配置已变化，请刷新后重新检查");
    }
    let configured_cwd = view
        .extra
        .get("cwd")
        .and_then(Value::as_str)
        .map(PathBuf::from);
    let cwd_source = if configured_cwd.is_some() {
        "config"
    } else if selected_cwd.is_some() {
        "selected"
    } else {
        "home"
    };
    let cwd = configured_cwd
        .or(selected_cwd)
        .unwrap_or_else(|| ctx.home_dir.clone());
    let codex = scan.target.format == Format::CodexToml;
    let default_timeout = if codex {
        Duration::from_secs(10)
    } else if view.transport == Transport::Remote {
        HTTP_TIMEOUT
    } else {
        STDIO_TIMEOUT
    };
    let timeout = view
        .startup_timeout_sec
        .filter(|sec| *sec > 0)
        .map(|sec| Duration::from_secs(sec as u64))
        .or_else(|| {
            view.extra
                .get("startup_timeout_ms")
                .and_then(Value::as_u64)
                .filter(|ms| *ms > 0)
                .map(Duration::from_millis)
        })
        .unwrap_or(default_timeout);
    let tool_timeout = view
        .extra
        .get("tool_timeout_sec")
        .and_then(Value::as_u64)
        .filter(|sec| *sec > 0)
        .map(Duration::from_secs)
        .unwrap_or(if codex {
            Duration::from_secs(60)
        } else {
            default_timeout
        });
    let started = Instant::now();
    let (outcome, secrets) = match view.transport {
        Transport::Stdio
            if !cwd.is_absolute()
                || !cwd.is_dir()
                || (view.extra.contains_key("cwd") && view.extra["cwd"].as_str().is_none()) =>
        {
            (
                Outcome::fail("start", "配置中的工作目录必须是已存在目录的绝对路径"),
                Vec::new(),
            )
        }
        Transport::Stdio => {
            check_stdio(ctx, &scan.target.format, view, &cwd, timeout, tool_timeout)
        }
        Transport::Remote => check_remote(
            ctx,
            &scan.target.format,
            view,
            &entry.raw,
            timeout,
            tool_timeout,
        ),
    };
    let tools = outcome.tools.as_ref();
    Ok(json!({
        "domain": "mcp",
        "action": "check",
        "server": server,
        "agent": agent,
        "base": entry.base,
        "cwd": if view.transport == Transport::Stdio { Some(&cwd) } else { None },
        "cwd_source": if view.transport == Transport::Stdio { Some(cwd_source) } else { None },
        "startup_timeout_sec": timeout.as_secs_f64(),
        "tool_timeout_sec": tool_timeout.as_secs_f64(),
        "exit_code": outcome.exit_code,
        "ok": outcome.ok,
        "stage": outcome.stage,
        "message": scrub(&outcome.message, &secrets),
        "server_info": outcome.server,
        "tool_count": tools.map(Vec::len),
        "tools": tools.map(|names| names.iter().take(TOOL_NAMES).cloned().collect::<Vec<_>>()),
        "detail": outcome.detail.map(|text| scrub(&text, &secrets)),
        "elapsed_ms": started.elapsed().as_millis() as u64,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_request_rejects_unsafe_url_header_and_session_before_starting_curl() {
        let root = std::env::temp_dir().join(format!(
            "am-mcp-http-guard-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let ctx = Context {
            home_dir: root.join("home"),
            data_dir: root.join("data"),
        };
        for (url, headers, session) in [
            ("--config=/YOUR_OTHER_ACCOUNT/curlrc", Vec::new(), None),
            ("file:///YOUR_OTHER_ACCOUNT/curlrc", Vec::new(), None),
            (
                "https://mcp.example.invalid",
                vec![("X-Test\nAuthorization".into(), "static".into())],
                None,
            ),
            (
                "https://mcp.example.invalid",
                vec![(
                    "X-Test".into(),
                    "static\r\nAuthorization: YOUR_OTHER_ACCOUNT".into(),
                )],
                None,
            ),
            (
                "https://mcp.example.invalid",
                Vec::new(),
                Some("session\nYOUR_OTHER_ACCOUNT"),
            ),
            (
                "https://mcp.example.invalid",
                Vec::new(),
                Some("session\0YOUR_OTHER_ACCOUNT"),
            ),
        ] {
            let result = http_post(
                &ctx,
                url,
                &headers,
                session,
                &json!({}),
                Some(1),
                Instant::now() + Duration::from_secs(1),
            );
            assert!(result.is_err());
            assert!(result.unwrap_err().contains("没有发送请求"));
            assert!(!root.exists());
        }
    }
}
