//! Rust sidecar 桥：把前端的引擎调用转发给 `<root>/manage.sh <command> --json`，
//! 并把 stdout 上的 Contract v1 envelope 反序列化、校验基本形状后返回给前端。
//!
//! 设计要点（docs/goal-rust-gui.md / docs/GUI-ARCHITECTURE-PROPOSAL.md §6.1）：
//! - 开发期不调 engine_v1.py 本体，而是经 manage.sh，由它完成三层策略合并与 env 覆盖。
//! - 引擎契约约定：envelope 永远写 stdout（退出码 0/1/2 都可能伴随合法 envelope），
//!   人类可读文本只走 stderr。因此先解析 stdout，解析失败才落到结构化错误。
//! - 桥只允许契约命令面内的只读/刷新命令，args 仅放行 `--only <element-id>`。

use serde::{Deserialize, Serialize};
use tauri::Manager;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::Duration;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

pub const CONTRACT_ID: &str = "agent-manager/engine/v1";
const DEFAULT_TIMEOUT_SEC: u64 = 600;
const STDERR_SUMMARY_LIMIT: usize = 2000;
const RESOURCE_ROOT: &str = "resources/engine";
static STANDALONE_INIT: OnceLock<Mutex<()>> = OnceLock::new();

/// 序列化后经 invoke 返回给前端的结构化错误。
#[derive(Debug, Clone, Serialize)]
pub struct EngineError {
    pub kind: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stderr: Option<String>,
}

impl EngineError {
    fn new(kind: &str, message: impl Into<String>) -> Self {
        Self { kind: kind.to_string(), message: message.into(), exit_code: None, stderr: None }
    }

    fn with_process(mut self, exit_code: Option<i32>, stderr: &str) -> Self {
        self.exit_code = exit_code;
        let trimmed = stderr.trim();
        if !trimmed.is_empty() {
            let summary: String = trimmed.chars().take(STDERR_SUMMARY_LIMIT).collect();
            self.stderr = Some(summary);
        }
        self
    }
}

// ---------- Contract v1 envelope 镜像（字段与 envelope.schema.json 一致） ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mode {
    pub dry_run: bool,
    pub offline: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockInfo {
    pub held: bool,
    pub pid: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub element: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope {
    pub contract: String,
    pub run_id: String,
    pub command: String,
    pub mode: Mode,
    pub lock: LockInfo,
    pub log_file: Option<String>,
    pub ok: bool,
    /// 域负载的精确形状由各域 schema 定义（status/plan/audit）；envelope 层只要求是 object。
    pub data: serde_json::Value,
    pub warnings: Vec<Diagnostic>,
    pub errors: Vec<Diagnostic>,
}

/// 解析并校验 envelope 基本形状（contract 标识、ok、data object）。
pub fn parse_envelope(stdout: &str) -> Result<Envelope, EngineError> {
    let envelope: Envelope = serde_json::from_str(stdout)
        .map_err(|err| EngineError::new("invalid_output", format!("引擎 stdout 不是合法 envelope JSON: {err}")))?;
    if envelope.contract != CONTRACT_ID {
        return Err(EngineError::new(
            "contract_mismatch",
            format!("未知契约标识 '{}'，期望 '{CONTRACT_ID}'", envelope.contract),
        ));
    }
    if !envelope.data.is_object() {
        return Err(EngineError::new("contract_violation", "envelope.data 必须是 object"));
    }
    Ok(envelope)
}

// ---------- 命令面白名单 ----------

/// 支持 status + plan 的域（secrets 只有 status；system 只有 audit / refresh-upstream）。
/// 支持 status/plan/apply 的域（对齐引擎 APPLY_DOMAINS；secrets 只读、profile 单列）。
const READ_WRITE_DOMAINS: &[&str] = &["agents", "mcp", "skills", "rules", "hooks", "tools", "aliases"];

pub fn validate_command(command: &str) -> Result<(), EngineError> {
    let rejected = || {
        EngineError::new(
            "unknown_command",
            format!(
                "命令 '{command}' 不在契约命令面内（允许 <domain>:status|plan|apply / system:audit|refresh-upstream|converge / snapshots:* / runs:* / policy:* / profile:*）"
            ),
        )
    };
    let Some((domain, verb)) = command.split_once(':') else {
        return Err(rejected());
    };
    if domain.is_empty() || verb.is_empty() || verb.contains(':') {
        return Err(rejected());
    }
    let allowed = match (domain, verb) {
        ("system", "audit" | "refresh-upstream" | "converge") => true,
        ("secrets", "status") => true,
        ("snapshots", "list" | "restore") => true,
        ("runs", "list" | "get") => true,
        ("policy", "get" | "set" | "delete") => true,
        ("sources", "get" | "set") => true,
        ("profile", "status" | "pull" | "push") => true,
        ("db", "status" | "migrate") => true,
        (d, "status" | "plan" | "apply") => READ_WRITE_DOMAINS.contains(&d),
        _ => false,
    };
    if allowed { Ok(()) } else { Err(rejected()) }
}

/// 参数取值类型（按类型校验；不经 shell，argv 直传引擎）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArgValueKind {
    /// 纯开关（--dry-run / --allow-uninstall），不带值。
    Flag,
    /// 元素 id，如 mcp:grok-search / skill:agent-skills/find-skills。
    ElementId,
    /// 运行 id（runs:get / snapshots:restore 的 --run），字母数字与 _/-。
    RunId,
    /// runs:get 的 --lines，非负整数。
    Lines,
    /// policy:set 落点层。
    PolicyLayer,
    /// policy:set 点分键路径（对齐引擎 key 段规则：字母/数字/_/-）。
    DotKey,
    /// policy:set 的 JSON 值——合法性即 JSON 可解析。
    JsonValue,
    /// agents:plan/apply 的显式生命周期动作：仅 install/upgrade/uninstall。
    AgentAction,
}

/// 每个命令允许的 flag 集合（命令感知白名单）。
fn allowed_arg_specs(command: &str) -> Vec<(&'static str, ArgValueKind)> {
    let (domain, verb) = command.split_once(':').unwrap_or(("", ""));
    let mut specs: Vec<(&'static str, ArgValueKind)> = Vec::new();
    match (domain, verb) {
        ("sources", "get") => specs.push(("--key", ArgValueKind::DotKey)),
        ("sources", "set") => {
            specs.push(("--key", ArgValueKind::DotKey));
            specs.push(("--value", ArgValueKind::JsonValue));
            specs.push(("--dry-run", ArgValueKind::Flag));
        }
        ("system", "refresh-upstream") => specs.push(("--only", ArgValueKind::ElementId)),
        ("runs", "get") => {
            specs.push(("--run", ArgValueKind::RunId));
            specs.push(("--lines", ArgValueKind::Lines));
        }
        ("snapshots", "restore") => specs.push(("--run", ArgValueKind::RunId)),
        ("policy", "set") => {
            specs.push(("--layer", ArgValueKind::PolicyLayer));
            specs.push(("--key", ArgValueKind::DotKey));
            specs.push(("--value", ArgValueKind::JsonValue));
        }
        ("policy", "delete") => {
            specs.push(("--layer", ArgValueKind::PolicyLayer));
            specs.push(("--key", ArgValueKind::DotKey));
        }
        ("profile", "pull") => {
            specs.push(("--accept-remote", ArgValueKind::Flag));
            // 键级以远端为准：--only <顶层键>（点分键字符集，与 policy key 同规则）
            specs.push(("--only", ArgValueKind::DotKey));
        }
        ("profile", "push") => {
            specs.push(("--dry-run", ArgValueKind::Flag));
            specs.push(("--force", ArgValueKind::Flag));
        }
        (_, "plan") => {
            specs.push(("--only", ArgValueKind::ElementId));
            if domain == "agents" {
                specs.push(("--action", ArgValueKind::AgentAction));
            }
            if domain == "skills" {
                specs.push(("--agent", ArgValueKind::ElementId));
            }
        }
        (_, "apply") => {
            specs.push(("--dry-run", ArgValueKind::Flag));
            specs.push(("--only", ArgValueKind::ElementId));
            if domain == "agents" {
                specs.push(("--allow-uninstall", ArgValueKind::Flag));
                specs.push(("--action", ArgValueKind::AgentAction));
            }
            if domain == "skills" {
                specs.push(("--agent", ArgValueKind::ElementId));
            }
        }
        ("system", "converge") => specs.push(("--dry-run", ArgValueKind::Flag)),
        _ => {}
    }
    specs
}

fn validate_element_id(value: &str) -> Result<(), EngineError> {
    let valid = !value.is_empty()
        && value
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_:/@+.-=".contains(c));
    if valid {
        Ok(())
    } else {
        Err(EngineError::new("invalid_args", format!("非法元素 id '{value}'")))
    }
}

/// 前端生成的 run id 字符集（透传 AGENT_MANAGER_RUN_ID）。
pub fn validate_run_id(value: &str) -> Result<(), EngineError> {
    let valid = !value.is_empty()
        && value.len() <= 64
        && value.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
        && value.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if valid {
        Ok(())
    } else {
        Err(EngineError::new("invalid_args", format!("非法 run id '{value}'")))
    }
}

fn validate_arg_value(kind: ArgValueKind, value: &str) -> Result<(), EngineError> {
    match kind {
        ArgValueKind::Flag => Ok(()),
        ArgValueKind::ElementId => validate_element_id(value),
        ArgValueKind::RunId => validate_run_id(value),
        ArgValueKind::Lines => {
            let ok = !value.is_empty()
                && value.bytes().all(|b| b.is_ascii_digit())
                && value.parse::<u32>().is_ok_and(|n| n <= 100_000);
            if ok {
                Ok(())
            } else {
                Err(EngineError::new("invalid_args", format!("非法行数 '{value}'")))
            }
        }
        ArgValueKind::PolicyLayer => {
            if value == "synced" || value == "local" {
                Ok(())
            } else {
                Err(EngineError::new(
                    "invalid_args",
                    format!("非法策略层 '{value}'（仅 synced|local；baseline 只读）"),
                ))
            }
        }
        ArgValueKind::DotKey => {
            let ok = !value.is_empty()
                && value
                    .split('.')
                    .all(|seg| !seg.is_empty() && seg.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'));
            if ok {
                Ok(())
            } else {
                Err(EngineError::new("invalid_args", format!("非法 key 路径 '{value}'")))
            }
        }
        ArgValueKind::JsonValue => serde_json::from_str::<serde_json::Value>(value)
            .map(|_| ())
            .map_err(|err| EngineError::new("invalid_args", format!("--value 不是合法 JSON: {err}"))),
        ArgValueKind::AgentAction => match value {
            "install" | "upgrade" | "uninstall" => Ok(()),
            _ => Err(EngineError::new(
                "invalid_args",
                format!("非法动作 '{value}'（仅 install|upgrade|uninstall）"),
            )),
        },
    }
}

pub fn validate_args(command: &str, args: &[String]) -> Result<(), EngineError> {
    let specs = allowed_arg_specs(command);
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        let Some((_, kind)) = specs.iter().find(|(flag, _)| flag == &arg.as_str()) else {
            return Err(EngineError::new(
                "invalid_args",
                format!("命令 '{command}' 不允许参数 '{arg}'"),
            ));
        };
        if *kind != ArgValueKind::Flag {
            index += 1;
            let Some(value) = args.get(index) else {
                return Err(EngineError::new("invalid_args", format!("参数 '{arg}' 缺少取值")));
            };
            validate_arg_value(*kind, value)?;
        }
        index += 1;
    }
    Ok(())
}

// ---------- 仓库根解析 ----------

/// 解析顺序：env `AGENT_MANAGER_ROOT` > 可执行文件祖先目录探测（含 manage.sh）> dev 时
/// `app/` 的父目录（src-tauri 的上两级）。
pub fn resolve_root() -> Result<PathBuf, EngineError> {
    resolve_root_from(
        std::env::var("AGENT_MANAGER_ROOT").ok().as_deref(),
        std::env::current_exe().ok().as_deref(),
        Path::new(env!("CARGO_MANIFEST_DIR")),
    )
}

#[derive(Debug, Deserialize)]
struct StandaloneManifest {
    schema: u32,
    managed_files: Vec<String>,
    #[serde(default)]
    managed_content_sha256: String,
}

struct RuntimeLayout {
    root: PathBuf,
    python: Option<PathBuf>,
    rust_engine: Option<PathBuf>,
    production: bool,
}

fn standalone_data_dir(app: &tauri::AppHandle) -> Result<PathBuf, EngineError> {
    if let Ok(raw) = std::env::var("AGENT_MANAGER_DATA_DIR") {
        if !raw.trim().is_empty() {
            return Ok(PathBuf::from(raw));
        }
    }
    app.path().app_data_dir().map_err(|err| {
        EngineError::new("data_dir_unavailable", format!("无法定位应用数据目录: {err}"))
    })
}

fn valid_relative(path: &str) -> bool {
    let candidate = Path::new(path);
    !candidate.is_absolute() && candidate.components().all(|c| matches!(c, Component::Normal(_)))
}

fn executable(path: &Path) -> bool {
    if !path.is_file() { return false; }
    #[cfg(unix)]
    { return path.metadata().map(|m| m.permissions().mode() & 0o111 != 0).unwrap_or(false); }
    #[cfg(not(unix))]
    { true }
}

fn copy_standalone_resources(resource_root: &Path, data_root: &Path) -> Result<PathBuf, EngineError> {
    let manifest_path = resource_root.join("standalone-manifest.json");
    let manifest_bytes = std::fs::read(&manifest_path).map_err(|err| {
            EngineError::new("installation_incomplete", format!("缺少内置引擎清单: {err}"))
        })?;
    let manifest: StandaloneManifest = serde_json::from_slice(&manifest_bytes)
    .map_err(|err| EngineError::new("installation_incomplete", format!("内置引擎清单无效: {err}")))?;
    if manifest.schema != 1 || manifest.managed_files.is_empty() {
        return Err(EngineError::new("installation_incomplete", "内置引擎清单版本或内容无效"));
    }
    // Validate the complete read-only package before creating or changing the
    // user data root. This keeps a broken upgrade from leaving a partial tree.
    for relative in &manifest.managed_files {
        if !valid_relative(relative) || !resource_root.join(relative).is_file() {
            return Err(EngineError::new("installation_incomplete", format!("内置资源缺失或路径非法: {relative}")));
        }
    }
    let python = resource_root.join("runtime/python/bin/python3");
    let rust_engine = resource_root.join("bin/agent-manager-engine");
    if !executable(&python) || !executable(&rust_engine) {
        return Err(EngineError::new("installation_incomplete", "缺少内置便携 Python 运行时"));
    }
    let _guard = STANDALONE_INIT.get_or_init(|| Mutex::new(())).lock().unwrap();
    let marker = data_root.join(".standalone-install.json");
    if data_root.join("manage.sh").is_file()
        && std::fs::read(&marker).ok().as_deref() == Some(manifest_bytes.as_slice())
    {
        return Ok(data_root.to_path_buf());
    }
    std::fs::create_dir_all(data_root).map_err(|err| {
        EngineError::new("data_dir_unavailable", format!("无法创建应用数据目录: {err}"))
    })?;
    let local_path = std::env::var_os("AGENT_MANAGER_LOCAL_CONFIG").map(PathBuf::from)
        .unwrap_or_else(|| data_root.join("config/manager.local.json"));
    let local: serde_json::Value = std::fs::read(local_path).ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default();
    let edited_sources = local.get("source_edits").and_then(serde_json::Value::as_array);
    for relative in manifest.managed_files {
        let source = resource_root.join(&relative);
        let target = data_root.join(&relative);
        let editable = matches!(relative.as_str(), "config/claude-hooks.json" | "config/codex-hooks.json" | "config/shell-aliases.json" | "agent-rule/rules/src/shared/core.md" | "agent-rule/rules/src/agents/claude/adapter.md" | "agent-rule/rules/src/agents/codex/adapter.md");
        if editable && target.is_file() && edited_sources.is_some_and(|paths| paths.iter().any(|path| path.as_str() == Some(relative.as_str()))) {
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|err| {
                EngineError::new("data_dir_unavailable", format!("无法创建引擎运行目录: {err}"))
            })?;
        }
        let temp = target.with_extension(format!("tmp.{}", std::process::id()));
        std::fs::copy(&source, &temp).map_err(|err| {
            EngineError::new("data_dir_unavailable", format!("无法更新引擎程序文件 {relative}: {err}"))
        })?;
        std::fs::rename(&temp, &target).map_err(|err| {
            EngineError::new("data_dir_unavailable", format!("无法原子替换引擎程序文件 {relative}: {err}"))
        })?;
    }
    if let Some(parent) = marker.parent() {
        std::fs::create_dir_all(parent).map_err(|err| EngineError::new("data_dir_unavailable", format!("无法创建安装标记目录: {err}")))?;
    }
    let marker_temp = marker.with_extension(format!("tmp.{}", std::process::id()));
    std::fs::copy(&manifest_path, &marker_temp).map_err(|err| EngineError::new("data_dir_unavailable", format!("无法写入安装标记: {err}")))?;
    std::fs::rename(marker_temp, marker).map_err(|err| EngineError::new("data_dir_unavailable", format!("无法原子更新安装标记: {err}")))?;
    Ok(data_root.to_path_buf())
}

fn resolve_runtime_layout(_app: &tauri::AppHandle) -> Result<RuntimeLayout, EngineError> {
    #[cfg(debug_assertions)]
    {
        let root = resolve_root()?;
        return Ok(RuntimeLayout { root, python: None, rust_engine: None, production: false });
    }
    #[cfg(not(debug_assertions))]
    {
        let resource_root = _app.path().resource_dir().map_err(|err| EngineError::new("installation_incomplete", format!("无法定位应用资源目录: {err}")))?.join(RESOURCE_ROOT);
        let root = copy_standalone_resources(&resource_root, &standalone_data_dir(_app)?.join("engine"))?;
        Ok(RuntimeLayout { root, python: Some(resource_root.join("runtime/python/bin/python3")), rust_engine: Some(resource_root.join("bin/agent-manager-engine")), production: true })
    }
}

fn ensure_manage_sh(dir: &Path) -> Result<PathBuf, EngineError> {
    if dir.join("manage.sh").is_file() {
        Ok(dir.to_path_buf())
    } else {
        Err(EngineError::new(
            "root_not_found",
            format!("目录 '{}' 下没有 manage.sh，无法作为引擎仓库根", dir.display()),
        ))
    }
}

fn resolve_root_from(
    env_root: Option<&str>,
    exe: Option<&Path>,
    manifest_dir: &Path,
) -> Result<PathBuf, EngineError> {
    if let Some(root) = env_root {
        return ensure_manage_sh(Path::new(root));
    }
    if let Some(exe) = exe {
        for ancestor in exe.ancestors().skip(1) {
            if ancestor.join("manage.sh").is_file() {
                return Ok(ancestor.to_path_buf());
            }
        }
    }
    // dev 回退：src-tauri -> app -> 仓库根
    if let Some(root) = manifest_dir.parent().and_then(Path::parent) {
        return ensure_manage_sh(root);
    }
    Err(EngineError::new(
        "root_not_found",
        "无法定位引擎仓库根；请设置 AGENT_MANAGER_ROOT 环境变量",
    ))
}

// ---------- 进程执行 ----------

fn engine_timeout() -> Duration {
    std::env::var("AGENT_MANAGER_ENGINE_TIMEOUT_SEC")
        .ok()
        .and_then(|raw| raw.parse::<u64>().ok())
        .filter(|secs| *secs > 0)
        .map_or(Duration::from_secs(DEFAULT_TIMEOUT_SEC), Duration::from_secs)
}

/// 超时后尽力终止：Unix 发 TERM（manage.sh 的 trap 会清理其子进程），Windows 用 taskkill。
fn terminate_process(pid: u32) {
    #[cfg(unix)]
    {
        let _ = Command::new("kill").arg("-TERM").arg(pid.to_string()).status();
    }
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .status();
    }
}

/// 经 manage.sh 运行引擎命令，返回校验过的 envelope。
/// 退出码 0/1/2 都可能有合法 envelope（契约语义），只有 stdout 无法解析时才报错。
/// run_id 非空时经 AGENT_MANAGER_RUN_ID 透传（manage.sh 优先采用外部 RUN_ID），
/// 使 GUI 发起的写路径能在执行中心按 run_id 追踪按次日志。
pub fn run_engine(
    root: &Path,
    command: &str,
    args: &[String],
    run_id: Option<&str>,
) -> Result<Envelope, EngineError> {
    let python = root.join("runtime/python/bin/python3");
    let rust = root.join("bin/agent-manager-engine");
    run_engine_with_layout(root, Some(&python).filter(|p| p.is_file()).map(|p| p.as_path()), Some(&rust).filter(|p| p.is_file()).map(|p| p.as_path()), false, command, args, run_id)
}

fn run_engine_with_layout(
    root: &Path,
    python_override: Option<&Path>,
    rust_override: Option<&Path>,
    production: bool,
    command: &str,
    args: &[String],
    run_id: Option<&str>,
) -> Result<Envelope, EngineError> {
    run_engine_with_preferences(root, python_override, rust_override, production, command, args, run_id, None)
}

fn run_engine_with_preferences(
    root: &Path,
    python_override: Option<&Path>,
    rust_override: Option<&Path>,
    production: bool,
    command: &str,
    args: &[String],
    run_id: Option<&str>,
    preferences: Option<&crate::settings::SettingsPreferences>,
) -> Result<Envelope, EngineError> {
    run_engine_full(root, python_override, rust_override, production, command, args, run_id, preferences, None)
}

fn run_engine_full(
    root: &Path,
    python_override: Option<&Path>,
    rust_override: Option<&Path>,
    production: bool,
    command: &str,
    args: &[String],
    run_id: Option<&str>,
    preferences: Option<&crate::settings::SettingsPreferences>,
    data_dir_override: Option<&Path>,
) -> Result<Envelope, EngineError> {
    validate_command(command)?;
    validate_args(command, args)?;
    if let Some(run_id) = run_id {
        validate_run_id(run_id)?;
    }
    let script = root.join("manage.sh");
    if !script.is_file() {
        return Err(EngineError::new(
            "root_not_found",
            format!("manage.sh 不存在: {}", script.display()),
        ));
    }

    let mut cmd = Command::new("bash");
    let mut effective_args = args.to_vec();
    if preferences.is_some_and(|p| p.offline) && !effective_args.iter().any(|arg| arg == "--offline") {
        effective_args.push("--offline".into());
    }
    cmd.arg(&script)
        .arg(command)
        .arg("--json")
        .args(&effective_args)
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    cmd.env("PYTHONDONTWRITEBYTECODE", "1");
    if let Some(run_id) = run_id {
        cmd.env("AGENT_MANAGER_RUN_ID", run_id);
    }
    if let Some(python) = python_override {
        let path = std::env::var_os("PATH").unwrap_or_default();
        let mut paths = vec![python.parent().unwrap().to_path_buf()];
        paths.extend(std::env::split_paths(&path));
        if let Ok(joined) = std::env::join_paths(paths) {
            cmd.env("PATH", joined);
        }
        cmd.env("AGENT_MANAGER_PYTHON_CMD", python);
    } else if production {
        return Err(EngineError::new("installation_incomplete", "发行包缺少内置 Python 运行时"));
    }
    cmd.env("AGENT_MANAGER_ROOT_DIR", root);
    if let Some(data_dir) = data_dir_override {
        cmd.env("AGENT_MANAGER_DATA_DIR", data_dir);
    }
    cmd.env("AGENT_MANAGER_BASE_CONFIG", root.join("config/manager.json"));
    cmd.env("AGENT_MANAGER_SYNCED_CONFIG", root.join("config/manager.synced.json"));
    cmd.env("AGENT_MANAGER_LOCAL_CONFIG", root.join("config/manager.local.json"));
    if let Some(prefs) = preferences {
        cmd.env("AGENT_MANAGER_UPSTREAM_TTL_SEC", prefs.upstream_ttl_sec.to_string());
    }
    if let Some(packaged_engine) = rust_override {
        cmd.env("AGENT_MANAGER_ENGINE_RS", packaged_engine);
    } else if production {
        return Err(EngineError::new("installation_incomplete", "发行包缺少 Rust 引擎运行体"));
    }
    let state = root.to_path_buf();
    cmd.env("AGENT_MANAGER_LOG_PREFIX", state.join("logs/agent_manager_sync"));
    cmd.env("AGENT_MANAGER_LATEST_LOG_POINTER", state.join("logs/agent_manager_sync.latest"));
    cmd.env("AGENT_MANAGER_LEGACY_LOG_PATH", state.join("logs/agent_manager_sync.log"));
    cmd.env("AGENT_MANAGER_LOCK_FILE", state.join("state/agent_manager.lock"));
    cmd.env("AGENT_MANAGER_SNAPSHOT_DIR", state.join("snapshots"));
    cmd.env("AGENT_MANAGER_UPSTREAM_CACHE", state.join("cache/upstream.json"));
    cmd.env("AGENT_MANAGER_PROFILE_CACHE", state.join("cache/profile"));
    for directory in [state.join("logs"), state.join("state"), state.join("snapshots"), state.join("cache/profile")] {
        std::fs::create_dir_all(&directory).map_err(|err| EngineError::new("data_dir_unavailable", format!("无法创建运行目录 {}: {err}", directory.display())))?;
    }

    let child = cmd.spawn().map_err(|err| {
        EngineError::new("spawn_failed", format!("无法启动 manage.sh: {err}"))
    })?;
    let pid = child.id();

    // wait_with_output 会消费 child；把它移进工作线程，主线程只做带超时的等待。
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });

    let output: io::Result<std::process::Output> = match rx.recv_timeout(engine_timeout()) {
        Ok(result) => result,
        Err(mpsc::RecvTimeoutError::Timeout) => {
            terminate_process(pid);
            let _ = rx.recv();
            return Err(EngineError::new(
                "timeout",
                format!("引擎命令 '{command}' 超过 {}s 未完成，已终止", engine_timeout().as_secs()),
            ));
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            return Err(EngineError::new("internal", "引擎等待线程异常断开"));
        }
    };

    let output = output.map_err(|err| EngineError::new("spawn_failed", format!("读取引擎输出失败: {err}")))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    parse_envelope(&stdout).map_err(|err| {
        err.with_process(output.status.code(), &stderr)
    })
}

// ---------- Tauri command ----------

#[tauri::command]
pub async fn engine_invoke(app: tauri::AppHandle, command: String, args: Option<Vec<String>>, run_id: Option<String>) -> Result<Envelope, EngineError> {
    let args = args.unwrap_or_default();
    let app_handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let layout = resolve_runtime_layout(&app_handle)?;
        let preferences = crate::settings::current_preferences(&app_handle).map_err(|message| EngineError::new("settings_invalid", message))?;
        let data_dir = standalone_data_dir(&app_handle)?;
        run_engine_full(&layout.root, layout.python.as_deref(), layout.rust_engine.as_deref(), layout.production, &command, &args, run_id.as_deref(), Some(&preferences), Some(&data_dir))
    })
    .await
    .map_err(|err| EngineError::new("internal", format!("bridge 任务调度失败: {err}")))?
}

// ---------- 测试 ----------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    const VALID_ENVELOPE: &str = r#"{
        "contract": "agent-manager/engine/v1",
        "run_id": "20260905T103000Z-a1b2c3",
        "command": "mcp.status",
        "mode": {"dry_run": false, "offline": false},
        "lock": {"held": false, "pid": null},
        "log_file": null,
        "ok": true,
        "data": {"domain": "mcp", "elements": []},
        "warnings": [],
        "errors": []
    }"#;

    const ERROR_ENVELOPE: &str = r#"{
        "contract": "agent-manager/engine/v1",
        "run_id": "20260905T103000Z-a1b2c3",
        "command": "system.audit",
        "mode": {"dry_run": false, "offline": false},
        "lock": {"held": false, "pid": null},
        "log_file": null,
        "ok": false,
        "data": {},
        "warnings": [],
        "errors": [{"code": "AUDIT_ERRORS", "element": null, "message": "doctor 报告 6 个错误"}]
    }"#;

    fn temp_dir(tag: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "agent-manager-bridge-test-{}-{}-{tag}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn write_manage_sh(root: &Path, body: &str) {
        fs::write(root.join("manage.sh"), format!("#!/usr/bin/env bash\n{body}\n")).expect("write manage.sh");
    }

    // ---- parse_envelope ----

    #[test]
    fn parse_valid_envelope() {
        let envelope = parse_envelope(VALID_ENVELOPE).expect("valid envelope parses");
        assert!(envelope.ok);
        assert_eq!(envelope.command, "mcp.status");
        assert!(!envelope.mode.offline);
        assert!(envelope.errors.is_empty());
    }

    #[test]
    fn parse_envelope_with_errors_is_still_ok_result() {
        // 退出码 1 / errors 非空仍是合法 envelope，由前端渲染 ErrorPanel。
        let envelope = parse_envelope(ERROR_ENVELOPE).expect("error envelope parses");
        assert!(!envelope.ok);
        assert_eq!(envelope.errors[0].code, "AUDIT_ERRORS");
        assert_eq!(envelope.errors[0].element, None);
    }

    #[test]
    fn parse_rejects_wrong_contract_id() {
        let bad = VALID_ENVELOPE.replace("agent-manager/engine/v1", "agent-manager/engine/v9");
        let err = parse_envelope(&bad).expect_err("contract mismatch must fail");
        assert_eq!(err.kind, "contract_mismatch");
    }

    #[test]
    fn parse_rejects_non_object_data() {
        let bad = VALID_ENVELOPE.replace(r#""data": {"domain": "mcp", "elements": []}"#, r#""data": []"#);
        let err = parse_envelope(&bad).expect_err("non-object data must fail");
        assert_eq!(err.kind, "contract_violation");
    }

    #[test]
    fn parse_rejects_garbage() {
        let err = parse_envelope("not json at all").expect_err("garbage must fail");
        assert_eq!(err.kind, "invalid_output");
    }

    // ---- validate_command ----

    #[test]
    fn command_whitelist_allows_contract_surface() {
        for command in [
            "agents:status", "agents:plan", "agents:apply", "mcp:status", "mcp:plan", "mcp:apply",
            "skills:status", "skills:apply", "rules:plan", "rules:apply", "hooks:plan", "hooks:apply",
            "tools:status", "tools:apply", "aliases:plan", "aliases:apply", "secrets:status",
            "system:audit", "system:refresh-upstream", "system:converge",
            "snapshots:list", "snapshots:restore", "runs:list", "runs:get",
            "policy:get", "policy:set", "policy:delete", "profile:status", "profile:pull", "profile:push",
            "db:status", "db:migrate",
        ] {
            validate_command(command).unwrap_or_else(|err| panic!("{command} should pass: {}", err.message));
        }
    }

    #[test]
    fn command_whitelist_rejects_non_contract_commands() {
        for command in [
            "agents:update",   // manage.sh 管线命令，不是契约命令
            "secrets:plan",    // secrets 只读
            "secrets:apply",
            "policy:drop",     // policy 只有 get/set/delete
            "db:drop",
            "profile:init",
            "runs:delete",
            "update",
            "mcp:",
            ":status",
            "mcp:status:extra",
            "mcp:status --offline", // 注入尝试
        ] {
            assert!(validate_command(command).is_err(), "{command} should be rejected");
        }
    }

    // ---- validate_args（命令感知白名单） ----

    #[test]
    fn args_allow_only_flag_with_element_id() {
        validate_args("system:refresh-upstream", &[]).unwrap();
        validate_args("system:refresh-upstream", &["--only".into(), "mcp:grok-search".into()]).unwrap();
        validate_args("system:refresh-upstream", &["--only".into(), "skill:agent-skills/find-skills".into()]).unwrap();
        // 范围控制贯穿 plan 与 apply
        validate_args("mcp:plan", &["--only".into(), "mcp:grok-search".into()]).unwrap();
        validate_args("mcp:apply", &["--only".into(), "mcp:grok-search".into(), "--dry-run".into()]).unwrap();
        validate_args("agents:apply", &["--only".into(), "agents:codex".into(), "--allow-uninstall".into()]).unwrap();
        validate_args("skills:plan", &["--only".into(), "skill:agent-skills/find-skills".into(), "--agent".into(), "codex".into()]).unwrap();
        validate_args("skills:apply", &["--only".into(), "skill:agent-skills/find-skills".into(), "--agent".into(), "codex".into()]).unwrap();
    }

    #[test]
    fn args_reject_flags_outside_whitelist_and_shellish_values() {
        for (command, args) in [
            ("mcp:status", vec!["--config".to_string(), "/etc/passwd".to_string()]),
            ("system:refresh-upstream", vec!["--only".to_string()]),
            ("system:refresh-upstream", vec!["--only".to_string(), "-rf".to_string()]),
            ("mcp:status", vec!["; rm -rf /".to_string()]),
            ("mcp:status", vec!["$(reboot)".to_string()]),
            ("mcp:status", vec!["mcp:status --offline".to_string()]),
            // 命令感知的边界：--allow-uninstall 只允许 agents:apply
            ("mcp:apply", vec!["--allow-uninstall".to_string()]),
            // --agent 只允许 skills 域
            ("mcp:plan", vec!["--agent".to_string(), "codex".to_string()]),
            ("agents:apply", vec!["--agent".to_string(), "codex".to_string()]),
            // 只读命令一律无参数
            ("policy:get", vec!["--dry-run".to_string()]),
            ("runs:list", vec!["--run".to_string(), "r1".to_string()]),
        ] {
            assert!(validate_args(command, &args).is_err(), "{command} {args:?} should be rejected");
        }
    }

    #[test]
    fn args_apply_dry_run_and_agents_allow_uninstall() {
        validate_args("mcp:apply", &["--dry-run".into()]).unwrap();
        validate_args("agents:apply", &["--allow-uninstall".into(), "--dry-run".into()]).unwrap();
        validate_args("system:converge", &["--dry-run".into()]).unwrap();
        validate_args("profile:push", &["--dry-run".into()]).unwrap();
    }

    #[test]
    fn args_agents_explicit_action_whitelist() {
        for action in ["install", "upgrade", "uninstall"] {
            validate_args("agents:plan", &["--only".into(), "agents:codex".into(), "--action".into(), action.into()]).unwrap();
            validate_args("agents:apply", &["--only".into(), "agents:codex".into(), "--action".into(), action.into()]).unwrap();
        }
        validate_args(
            "agents:apply",
            &["--only".into(), "agents:codex".into(), "--action".into(), "uninstall".into(), "--allow-uninstall".into()],
        )
        .unwrap();
        for (command, args) in [
            // --action 只允许 agents plan/apply
            ("mcp:plan", vec!["--action".to_string(), "install".to_string()]),
            ("agents:status", vec!["--action".to_string(), "install".to_string()]),
            ("skills:apply", vec!["--action".to_string(), "install".to_string()]),
            // 动作值严格三选一，不允许 reinstall / 注入
            ("agents:plan", vec!["--action".to_string(), "reinstall".to_string()]),
            ("agents:apply", vec!["--action".to_string(), "install; rm -rf /".to_string()]),
            ("agents:apply", vec!["--action".to_string(), "INSTALL".to_string()]),
        ] {
            assert!(validate_args(command, &args).is_err(), "{command} {args:?} should be rejected");
        }
    }

    #[test]
    fn args_runs_get_and_restore_run_id_validation() {
        validate_args("runs:get", &["--run".into(), "gui-20260906T010203Z-ab12cd".into(), "--lines".into(), "200".into()]).unwrap();
        validate_args("snapshots:restore", &["--run".into(), "20260905103000_12345".into()]).unwrap();
        for bad in [
            vec!["--run".to_string(), "../etc".to_string()],
            vec!["--run".to_string(), "a b".to_string()],
            vec!["--lines".to_string(), "abc".to_string()],
            vec!["--lines".to_string(), "100001".to_string()],
        ] {
            assert!(validate_args("runs:get", &bad).is_err(), "runs:get {bad:?} should be rejected");
        }
    }

    #[test]
    fn args_policy_set_layer_key_value_validation() {
        validate_args(
            "policy:set",
            &["--layer".into(), "synced".into(), "--key".into(), "skills.repo.skills.name.agents.claude-code".into(), "--value".into(), "true".into()],
        )
        .unwrap();
        validate_args("policy:set", &["--layer".into(), "local".into(), "--key".into(), "mcp.grok-search.enabled".into(), "--value".into(), "{\"a\": 1}".into()]).unwrap();
        for bad in [
            vec!["--layer".to_string(), "baseline".to_string()], // baseline 只读
            vec!["--key".to_string(), "a..b".to_string()],
            vec!["--key".to_string(), "a b".to_string()],
            vec!["--value".to_string(), "{not json".to_string()],
            vec!["--value".to_string(), "".to_string()],
        ] {
            assert!(validate_args("policy:set", &bad).is_err(), "policy:set {bad:?} should be rejected");
        }
    }

    #[test]
    fn args_policy_delete_layer_and_key() {
        validate_args("policy:delete", &["--layer".into(), "local".into(), "--key".into(), "mcp.definitions.demo".into()]).unwrap();
        for bad in [
            vec!["--layer".to_string(), "baseline".to_string(), "--key".to_string(), "a.b".to_string()],
            vec!["--layer".to_string(), "local".to_string(), "--key".to_string(), "a..b".to_string()],
            vec!["--value".to_string(), "1".to_string()], // delete 无 --value
        ] {
            assert!(validate_args("policy:delete", &bad).is_err(), "policy:delete {bad:?} should be rejected");
        }
    }

    #[test]
    fn args_source_edit_only_allows_key_json_and_dry_run() {
        validate_command("sources:get").unwrap();
        validate_command("sources:set").unwrap();
        validate_args("sources:get", &["--key".into(), "rules".into()]).unwrap();
        validate_args("sources:set", &["--key".into(), "rules.shared".into(), "--value".into(), "{}".into(), "--dry-run".into()]).unwrap();
        assert!(validate_args("sources:set", &["--key".into(), "../../secrets".into()]).is_err());
        assert!(validate_args("sources:set", &["--def-file".into(), "elsewhere".into()]).is_err());
    }

    #[test]
    fn args_profile_direction_controls() {
        validate_args("profile:pull", &[]).unwrap();
        validate_args("profile:pull", &["--accept-remote".into()]).unwrap();
        validate_args("profile:pull", &["--accept-remote".into(), "--only".into(), "mcp".into()]).unwrap();
        validate_args("profile:push", &["--force".into()]).unwrap();
        validate_args("profile:push", &["--dry-run".into(), "--force".into()]).unwrap();
        // 方向参数不串命令：--accept-remote/--only 不许进 push，--force 不许进 pull
        for (command, args) in [
            ("profile:push", vec!["--accept-remote".to_string()]),
            ("profile:push", vec!["--only".to_string(), "mcp".to_string()]),
            ("profile:pull", vec!["--force".to_string()]),
            ("profile:pull", vec!["--only".to_string(), "bad key".to_string()]),
            ("profile:status", vec!["--force".to_string()]),
        ] {
            assert!(validate_args(command, &args).is_err(), "{command} {args:?} should be rejected");
        }
    }

    #[test]
    fn run_id_charset_validation() {
        validate_run_id("gui-20260906T010203Z-ab12cd").unwrap();
        validate_run_id("20260905103000_12345").unwrap();
        for bad in ["", "with space", "slash/inside", "-leading-dash", "$(evil)", "中文标识"] {
            assert!(validate_run_id(bad).is_err(), "{bad} should be rejected");
        }
    }

    // ---- resolve_root ----

    #[test]
    fn resolve_root_prefers_env_override() {
        let root = temp_dir("env-root");
        write_manage_sh(&root, "exit 0");
        let resolved = resolve_root_from(Some(root.to_str().unwrap()), None, Path::new("/nonexistent/src-tauri"))
            .expect("env root resolves");
        assert_eq!(resolved, root);
    }

    #[test]
    fn resolve_root_env_override_must_contain_manage_sh() {
        let root = temp_dir("bad-env-root");
        let err = resolve_root_from(Some(root.to_str().unwrap()), None, Path::new("/nonexistent/src-tauri"))
            .expect_err("missing manage.sh must fail");
        assert_eq!(err.kind, "root_not_found");
    }

    #[test]
    fn resolve_root_probes_exe_ancestors() {
        let root = temp_dir("exe-root");
        write_manage_sh(&root, "exit 0");
        let exe = root.join("Contents/MacOS/agent-manager-app");
        fs::create_dir_all(exe.parent().unwrap()).unwrap();
        fs::write(&exe, b"").unwrap();
        let resolved = resolve_root_from(None, Some(&exe), Path::new("/nonexistent/src-tauri"))
            .expect("ancestor probe resolves");
        assert_eq!(resolved, root);
    }

    #[test]
    fn resolve_root_dev_fallback_is_app_parent() {
        let root = temp_dir("dev-root");
        write_manage_sh(&root, "exit 0");
        let manifest_dir = root.join("app/src-tauri");
        fs::create_dir_all(&manifest_dir).unwrap();
        let resolved = resolve_root_from(None, None, &manifest_dir).expect("dev fallback resolves");
        assert_eq!(resolved, root);
    }

    #[test]
    fn resolve_root_reports_failure() {
        let err = resolve_root_from(None, None, Path::new("/nonexistent/src-tauri"))
            .expect_err("nothing to resolve must fail");
        assert_eq!(err.kind, "root_not_found");
    }

    #[test]
    fn standalone_resource_copy_preserves_user_state_and_rejects_bad_paths() {
        let resource = temp_dir("standalone-resource");
        let data = temp_dir("standalone-data");
        fs::create_dir_all(resource.join("runtime/python/bin")).unwrap();
        fs::create_dir_all(resource.join("bin")).unwrap();
        fs::write(resource.join("runtime/python/bin/python3"), b"python").unwrap();
        fs::write(resource.join("bin/agent-manager-engine"), b"rust").unwrap();
        fs::set_permissions(resource.join("runtime/python/bin/python3"), fs::Permissions::from_mode(0o755)).unwrap();
        fs::set_permissions(resource.join("bin/agent-manager-engine"), fs::Permissions::from_mode(0o755)).unwrap();
        fs::write(resource.join("manage.sh"), b"v1").unwrap();
        fs::write(resource.join("standalone-manifest.json"), br#"{"schema":1,"managed_files":["manage.sh"],"managed_content_sha256":"hash-v1"}"#).unwrap();
        let root = copy_standalone_resources(&resource, &data).unwrap();
        fs::create_dir_all(root.join("config")).unwrap();
        fs::write(root.join("config/manager.local.json"), b"user overlay").unwrap();
        fs::write(resource.join("manage.sh"), b"v2").unwrap();
        fs::write(resource.join("standalone-manifest.json"), br#"{"schema":1,"managed_files":["manage.sh"],"managed_content_sha256":"hash-v2"}"#).unwrap();
        copy_standalone_resources(&resource, &data).unwrap();
        assert_eq!(fs::read(root.join("manage.sh")).unwrap(), b"v2");
        assert_eq!(fs::read(root.join("config/manager.local.json")).unwrap(), b"user overlay");
        fs::write(resource.join("standalone-manifest.json"), br#"{"schema":1,"managed_files":["/escape"]}"#).unwrap();
        assert_eq!(copy_standalone_resources(&resource, &data).unwrap_err().kind, "installation_incomplete");
    }

    #[test]
    fn standalone_upgrade_preserves_edited_source_but_updates_programs() {
        let resource = temp_dir("source-resource");
        let data = temp_dir("source-data");
        fs::create_dir_all(resource.join("runtime/python/bin")).unwrap();
        fs::create_dir_all(resource.join("bin")).unwrap();
        fs::create_dir_all(resource.join("config")).unwrap();
        for relative in ["runtime/python/bin/python3", "bin/agent-manager-engine"] {
            let path = resource.join(relative);
            fs::write(&path, b"executable").unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        fs::write(resource.join("manage.sh"), b"v1").unwrap();
        fs::write(resource.join("config/claude-hooks.json"), br#"{"hooks":{}}"#).unwrap();
        fs::write(resource.join("standalone-manifest.json"), br#"{"schema":1,"managed_files":["manage.sh","config/claude-hooks.json"],"managed_content_sha256":"v1"}"#).unwrap();
        copy_standalone_resources(&resource, &data).unwrap();
        fs::write(data.join("config/manager.local.json"), br#"{"source_edits":["config/claude-hooks.json","manage.sh"]}"#).unwrap();
        fs::write(data.join("config/claude-hooks.json"), b"user definition").unwrap();
        fs::write(resource.join("manage.sh"), b"v2").unwrap();
        fs::write(resource.join("standalone-manifest.json"), br#"{"schema":1,"managed_files":["manage.sh","config/claude-hooks.json"],"managed_content_sha256":"v2"}"#).unwrap();
        copy_standalone_resources(&resource, &data).unwrap();
        assert_eq!(fs::read(data.join("config/claude-hooks.json")).unwrap(), b"user definition");
        assert_eq!(fs::read(data.join("manage.sh")).unwrap(), b"v2");
        let _ = fs::remove_dir_all(resource);
        let _ = fs::remove_dir_all(data);
    }

    #[test]
    fn standalone_same_manifest_does_not_rewrite_seed() {
        let resource = temp_dir("standalone-mtime-resource");
        let data = temp_dir("standalone-mtime-data");
        fs::create_dir_all(resource.join("runtime/python/bin")).unwrap();
        fs::create_dir_all(resource.join("bin")).unwrap();
        for (path, bytes) in [("runtime/python/bin/python3", b"python".as_slice()), ("bin/agent-manager-engine", b"rust".as_slice())] {
            fs::write(resource.join(path), bytes).unwrap();
            fs::set_permissions(resource.join(path), fs::Permissions::from_mode(0o755)).unwrap();
        }
        fs::write(resource.join("manage.sh"), b"v1").unwrap();
        fs::write(resource.join("standalone-manifest.json"), br#"{"schema":1,"managed_files":["manage.sh"],"managed_content_sha256":"hash-v1"}"#).unwrap();
        let root = copy_standalone_resources(&resource, &data).unwrap();
        let before = fs::metadata(root.join("manage.sh")).unwrap().modified().unwrap();
        copy_standalone_resources(&resource, &data).unwrap();
        assert_eq!(before, fs::metadata(root.join("manage.sh")).unwrap().modified().unwrap());
    }

    #[test]
    fn standalone_missing_runtime_does_not_create_data_root() {
        let resource = temp_dir("standalone-no-runtime");
        let data = temp_dir("standalone-no-runtime-data").join("new-userdata/engine");
        fs::create_dir_all(resource.join("bin")).unwrap();
        fs::create_dir_all(resource.join("runtime/python/bin")).unwrap();
        fs::write(resource.join("bin/agent-manager-engine"), b"rust").unwrap();
        fs::set_permissions(resource.join("bin/agent-manager-engine"), fs::Permissions::from_mode(0o755)).unwrap();
        fs::write(resource.join("manage.sh"), b"v1").unwrap();
        fs::write(resource.join("standalone-manifest.json"), br#"{"schema":1,"managed_files":["manage.sh"],"managed_content_sha256":"hash"}"#).unwrap();
        assert_eq!(copy_standalone_resources(&resource, &data).unwrap_err().kind, "installation_incomplete");
        assert!(!data.join("manage.sh").exists());
        fs::write(resource.join("runtime/python/bin/python3"), b"python").unwrap();
        assert_eq!(copy_standalone_resources(&resource, &data).unwrap_err().kind, "installation_incomplete");
    }

    #[test]
    fn fake_script_bridge_receives_runtime_paths() {
        let root = temp_dir("standalone production root with spaces");
        fs::create_dir_all(root.join("runtime/python/bin")).unwrap();
        fs::create_dir_all(root.join("bin")).unwrap();
        fs::write(root.join("runtime/python/bin/python3"), b"python").unwrap();
        fs::write(root.join("bin/agent-manager-engine"), b"rust").unwrap();
        fs::set_permissions(root.join("runtime/python/bin/python3"), fs::Permissions::from_mode(0o755)).unwrap();
        fs::set_permissions(root.join("bin/agent-manager-engine"), fs::Permissions::from_mode(0o755)).unwrap();
        let expected_python = root.join("runtime/python/bin/python3");
        let expected_rust = root.join("bin/agent-manager-engine");
        let body = format!("[ \"$AGENT_MANAGER_PYTHON_CMD\" = '{}' ] || exit 3\n[ \"$AGENT_MANAGER_ENGINE_RS\" = '{}' ] || exit 4\ncat <<'EOF'\n{VALID_ENVELOPE}\nEOF", expected_python.display(), expected_rust.display());
        write_manage_sh(&root, &body);
        let result = run_engine_with_layout(&root, Some(&root.join("runtime/python/bin/python3")), Some(&root.join("bin/agent-manager-engine")), true, "mcp:status", &[], Some("production-space-01"));
        assert!(result.is_ok(), "production bridge failed: {:?}", result.err());
    }

    #[test]
    fn standalone_concurrent_initialization_is_serialized() {
        let resource = temp_dir("standalone-concurrent-resource");
        let data = temp_dir("standalone-concurrent-data").join("engine");
        fs::create_dir_all(resource.join("runtime/python/bin")).unwrap();
        fs::create_dir_all(resource.join("bin")).unwrap();
        for (path, bytes) in [("runtime/python/bin/python3", b"python".as_slice()), ("bin/agent-manager-engine", b"rust".as_slice())] {
            fs::write(resource.join(path), bytes).unwrap();
            fs::set_permissions(resource.join(path), fs::Permissions::from_mode(0o755)).unwrap();
        }
        fs::write(resource.join("manage.sh"), b"v1").unwrap();
        fs::write(resource.join("standalone-manifest.json"), br#"{"schema":1,"managed_files":["manage.sh"],"managed_content_sha256":"hash"}"#).unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        let mut joins = Vec::new();
        for _ in 0..8 {
            let barrier = barrier.clone(); let resource = resource.clone(); let data = data.clone();
            joins.push(std::thread::spawn(move || { barrier.wait(); copy_standalone_resources(&resource, &data).unwrap(); }));
        }
        for join in joins { join.join().unwrap(); }
        assert_eq!(fs::read(data.join("manage.sh")).unwrap(), b"v1");
    }

    #[derive(Debug, PartialEq, Eq)]
    struct ResourceSnapshotItem {
        rel_path: PathBuf,
        is_dir: bool,
        is_symlink: bool,
        #[cfg(unix)]
        mode: u32,
        #[cfg(not(unix))]
        readonly: bool,
        link_target: Option<PathBuf>,
        content: Option<Vec<u8>>,
    }

    fn collect_tree_snapshots(dir: &Path) -> Vec<ResourceSnapshotItem> {
        let mut results = Vec::new();
        fn scan(current: &Path, base: &Path, acc: &mut Vec<ResourceSnapshotItem>) {
            let entries = fs::read_dir(current).expect("read_dir must succeed");
            let mut entries: Vec<_> = entries.map(|e| e.expect("entry must succeed")).collect();
            entries.sort_by_key(|e| e.path());
            for entry in entries {
                let path = entry.path();
                let rel = path.strip_prefix(base).unwrap().to_path_buf();
                let symlink_meta = fs::symlink_metadata(&path).expect("symlink_metadata must succeed");
                let is_symlink = symlink_meta.file_type().is_symlink();
                let is_dir = symlink_meta.is_dir();
                #[cfg(unix)]
                let mode = symlink_meta.permissions().mode();
                #[cfg(not(unix))]
                let readonly = symlink_meta.permissions().readonly();

                if is_symlink {
                    let link_target = fs::read_link(&path).expect("read_link must succeed");
                    acc.push(ResourceSnapshotItem {
                        rel_path: rel,
                        is_dir: false,
                        is_symlink: true,
                        #[cfg(unix)]
                        mode,
                        #[cfg(not(unix))]
                        readonly,
                        link_target: Some(link_target),
                        content: None,
                    });
                } else if is_dir {
                    acc.push(ResourceSnapshotItem {
                        rel_path: rel,
                        is_dir: true,
                        is_symlink: false,
                        #[cfg(unix)]
                        mode,
                        #[cfg(not(unix))]
                        readonly,
                        link_target: None,
                        content: None,
                    });
                    scan(&path, base, acc);
                } else if path.is_file() {
                    let bytes = fs::read(&path).expect("read file must succeed");
                    acc.push(ResourceSnapshotItem {
                        rel_path: rel,
                        is_dir: false,
                        is_symlink: false,
                        #[cfg(unix)]
                        mode,
                        #[cfg(not(unix))]
                        readonly,
                        link_target: None,
                        content: Some(bytes),
                    });
                }
            }
        }
        scan(dir, dir, &mut results);
        results
    }

    fn count_matches(dir: &Path, pattern: &str) -> usize {
        let mut count = 0;
        fn scan(current: &Path, pattern: &str, count: &mut usize) {
            if let Ok(entries) = fs::read_dir(current) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        if path.file_name().and_then(|n| n.to_str()).is_some_and(|n| n == pattern) {
                            *count += 1;
                        }
                        scan(&path, pattern, count);
                    } else if path.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.ends_with(pattern)) {
                        *count += 1;
                    }
                }
            }
        }
        scan(dir, pattern, &mut count);
        count
    }

    static TEST_SERIAL_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    #[test]
    #[ignore = "real packaged resources; run explicitly on the worker-built resource tree"]
    fn real_packaged_resources_status_and_plan() {
        let _guard = TEST_SERIAL_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap_or_else(|e| e.into_inner());
        let resource = std::env::var_os("AGENT_MANAGER_TEST_RESOURCE_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/engine"));
        let sandbox = temp_dir("pkg-status-plan-sandbox");
        let home = sandbox.join("home");
        let data = sandbox.join("data/engine");
        fs::create_dir_all(&home).unwrap();

        let old_home = std::env::var_os("HOME");
        let old_cf = std::env::var_os("CFFIXED_USER_HOME");
        let old_pyc = std::env::var_os("PYTHONDONTWRITEBYTECODE");
        std::env::remove_var("PYTHONDONTWRITEBYTECODE");
        std::env::set_var("HOME", &home);
        std::env::set_var("CFFIXED_USER_HOME", &home);

        assert!(resource.join("manage.sh").is_file());
        assert!(resource.join("runtime/python/bin/python3").is_file());
        assert!(resource.join("bin/agent-manager-engine").is_file());

        let before_snapshots = collect_tree_snapshots(&resource);
        assert_eq!(count_matches(&resource, ".pyc"), 0);
        assert_eq!(count_matches(&resource, ".env"), 0);

        copy_standalone_resources(&resource, &data).expect("real package initializes");
        let python = resource.join("runtime/python/bin/python3");
        let rust = resource.join("bin/agent-manager-engine");

        let status = run_engine_with_layout(&data, Some(&python), Some(&rust), true, "mcp:status", &[], Some("pkg-status-01"))
            .expect("packaged status process");
        assert!(status.ok);
        assert_eq!(status.command, "mcp.status");
        assert_eq!(status.contract, CONTRACT_ID);

        let plan = run_engine_with_layout(&data, Some(&python), Some(&rust), true, "mcp:plan", &[], Some("pkg-plan-01"))
            .expect("packaged plan process");
        assert!(plan.ok);
        assert_eq!(plan.command, "mcp.plan");

        let runs = run_engine_with_layout(&data, Some(&python), Some(&rust), true, "runs:list", &[], Some("pkg-runs-01"))
            .expect("packaged runs:list process");
        assert!(runs.ok);

        let snaps = run_engine_with_layout(&data, Some(&python), Some(&rust), true, "snapshots:list", &[], Some("pkg-snaps-01"))
            .expect("packaged snapshots:list process");
        assert!(snaps.ok);

        assert_eq!(count_matches(&resource, ".pyc"), 0, "no pyc in resources after status/plan");
        assert_eq!(count_matches(&data, ".pyc"), 0, "no pyc in data root after status/plan");

        let after_snapshots = collect_tree_snapshots(&resource);
        assert_eq!(before_snapshots, after_snapshots, "package resources must be identical (mode, symlinks, bytes)");

        if let Some(h) = old_home { std::env::set_var("HOME", h); } else { std::env::remove_var("HOME"); }
        if let Some(cf) = old_cf { std::env::set_var("CFFIXED_USER_HOME", cf); } else { std::env::remove_var("CFFIXED_USER_HOME"); }
        if let Some(pyc) = old_pyc { std::env::set_var("PYTHONDONTWRITEBYTECODE", pyc); }
    }

    #[test]
    #[ignore = "real packaged resources; run explicitly on the worker-built resource tree"]
    fn real_packaged_resources_selective_apply_and_isolation() {
        let _guard = TEST_SERIAL_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap_or_else(|e| e.into_inner());
        let resource = std::env::var_os("AGENT_MANAGER_TEST_RESOURCE_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/engine"));
        let sandbox = temp_dir("pkg-selective-apply-sandbox");
        let home = sandbox.join("home");
        let data = sandbox.join("data/engine");
        fs::create_dir_all(&home).unwrap();

        let old_home = std::env::var_os("HOME");
        let old_cf = std::env::var_os("CFFIXED_USER_HOME");
        let old_pyc = std::env::var_os("PYTHONDONTWRITEBYTECODE");
        let old_path = std::env::var_os("PATH");
        std::env::remove_var("PYTHONDONTWRITEBYTECODE");
        std::env::set_var("HOME", &home);
        std::env::set_var("CFFIXED_USER_HOME", &home);

        // Isolated fake bin with business command runners (node, npm, npx, git, curl) and fake agent executors (claude)
        // Ensure test relies ONLY on fake_bin and /usr/bin:/bin, without leaking host Node/npm/npx/git or system Python.
        let fake_bin = sandbox.join("fake_bin");
        fs::create_dir_all(&fake_bin).unwrap();
        for cmd_name in ["claude", "node", "npm", "npx", "git", "curl"] {
            let script = "#!/bin/sh\nexit 0\n";
            let script_path = fake_bin.join(cmd_name);
            fs::write(&script_path, script).unwrap();
            #[cfg(unix)]
            fs::set_permissions(&script_path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let local_claude = home.join(".local/bin/claude");
        fs::create_dir_all(local_claude.parent().unwrap()).unwrap();
        fs::write(&local_claude, "#!/bin/sh\nexit 0\n").unwrap();
        #[cfg(unix)]
        fs::set_permissions(&local_claude, fs::Permissions::from_mode(0o755)).unwrap();

        let test_paths = vec![fake_bin.clone(), PathBuf::from("/usr/bin"), PathBuf::from("/bin")];
        let test_path = std::env::join_paths(test_paths).unwrap();
        std::env::set_var("PATH", &test_path);

        let before_snapshots = collect_tree_snapshots(&resource);

        copy_standalone_resources(&resource, &data).expect("real package initializes");
        let python = resource.join("runtime/python/bin/python3");
        let rust = resource.join("bin/agent-manager-engine");

        let claude_config = home.join(".claude.json");
        let initial_json = serde_json::json!({
            "mcpServers": {
                "YOUR_SERVER_A": {
                    "command": "npx",
                    "args": ["-y", "YOUR_PKG_A", "--old-a"],
                    "env": {"_MANAGED_BY": "agent-manager"}
                },
                "YOUR_SERVER_B": {
                    "command": "echo",
                    "args": ["--old-b"],
                    "env": {"_MANAGED_BY": "agent-manager"}
                }
            }
        });
        fs::write(&claude_config, serde_json::to_string_pretty(&initial_json).unwrap()).unwrap();
        let initial_bytes = fs::read(&claude_config).unwrap();

        let mcp_cfg = serde_json::json!({
            "servers": {
                "YOUR_SERVER_A": {
                    "command": "npx",
                    "args": ["-y", "YOUR_PKG_A", "--new-a"],
                    "env": {},
                    "agents": ["claude-code"]
                },
                "YOUR_SERVER_B": {
                    "command": "echo",
                    "args": ["--new-b"],
                    "env": {},
                    "agents": ["claude-code"]
                }
            },
            "agent_paths": {
                "claude-code": "~/.claude.json"
            }
        });
        fs::write(data.join("config/mcp.json"), serde_json::to_string_pretty(&mcp_cfg).unwrap()).unwrap();

        let local_cfg = serde_json::json!({
            "mcp": {
                "YOUR_SERVER_A": {"enabled": true},
                "YOUR_SERVER_B": {"enabled": true}
            }
        });
        fs::write(data.join("config/manager.local.json"), serde_json::to_string_pretty(&local_cfg).unwrap()).unwrap();

        // 1. Dryrun with --only: zero writes, full target byte identity
        let dryrun = run_engine_with_layout(
            &data, Some(&python), Some(&rust), true,
            "mcp:apply",
            &["--dry-run".into(), "--only".into(), "mcp:YOUR_SERVER_A".into()],
            Some("pkg-dryrun-01")
        ).expect("mcp:apply dryrun");
        assert!(dryrun.ok);
        assert!(dryrun.mode.dry_run);
        assert_eq!(fs::read(&claude_config).unwrap(), initial_bytes, "dry-run must leave target file byte-for-byte identical");

        // 2. Reject illegal and cross-domain/unknown scopes before write, full target byte identity
        // 2a. Bridge-level rejection of illegal element_id characters (e.g. spaces/shell chars)
        let bad_char = run_engine_with_layout(
            &data, Some(&python), Some(&rust), true,
            "mcp:apply",
            &["--only".into(), "bad scope with spaces".into()],
            Some("pkg-bad-char-01")
        );
        assert!(bad_char.is_err(), "illegal element_id characters must be rejected by bridge before spawn");
        assert_eq!(fs::read(&claude_config).unwrap(), initial_bytes, "rejected illegal element ID must leave target file byte-for-byte identical");

        // 2b. Engine-level rejection of cross-domain scope
        let cross_domain = run_engine_with_layout(
            &data, Some(&python), Some(&rust), true,
            "mcp:apply",
            &["--only".into(), "tools:some_tool".into()],
            Some("pkg-cross-scope-01")
        ).expect("cross domain returns error envelope");
        assert!(!cross_domain.ok);
        assert_eq!(cross_domain.errors[0].code, "CROSS_DOMAIN_SCOPE");
        assert_eq!(fs::read(&claude_config).unwrap(), initial_bytes, "cross domain scope rejection must leave target file byte-for-byte identical");

        // 2c. Engine-level rejection of unknown nonexistent element
        let unknown_scope = run_engine_with_layout(
            &data, Some(&python), Some(&rust), true,
            "mcp:apply",
            &["--only".into(), "mcp:YOUR_NONEXISTENT_SERVER".into()],
            Some("pkg-unknown-scope-01")
        ).expect("unknown scope returns error envelope");
        assert!(!unknown_scope.ok);
        assert_eq!(unknown_scope.errors[0].code, "USAGE_ERROR");
        assert_eq!(fs::read(&claude_config).unwrap(), initial_bytes, "unknown scope rejection must leave target file byte-for-byte identical");

        // 3. Selective apply: only A updated, full B object preserved
        let apply = run_engine_with_layout(
            &data, Some(&python), Some(&rust), true,
            "mcp:apply",
            &["--only".into(), "mcp:YOUR_SERVER_A".into()],
            Some("pkg-apply-a-01")
        ).expect("mcp:apply only A");
        assert!(apply.ok);
        let target_after_apply: serde_json::Value = serde_json::from_slice(&fs::read(&claude_config).unwrap()).unwrap();
        assert_eq!(target_after_apply["mcpServers"]["YOUR_SERVER_A"]["args"], serde_json::json!(["-y", "YOUR_PKG_A", "--new-a"]), "A must be updated");
        assert_eq!(target_after_apply["mcpServers"]["YOUR_SERVER_B"], initial_json["mcpServers"]["YOUR_SERVER_B"], "full B server object must be exactly preserved when only applying A");

        // 4. Lifecycle execution check: tools:apply calling tool_a while tool_b lifecycle has 0 calls
        let tool_lifecycle_log = sandbox.join("tool_lifecycle.log");
        let tool_a_ready = sandbox.join("tool_a_ready");
        let tool_b_ready = sandbox.join("tool_b_ready");
        let tools_path = data.join("config/tools.json");
        let mut tools_cfg: serde_json::Value = if tools_path.exists() {
            serde_json::from_slice(&fs::read(&tools_path).unwrap()).unwrap()
        } else {
            serde_json::json!({"tools": {}})
        };
        tools_cfg["tools"]["tool_a"] = serde_json::json!({
            "ready_check": format!("app:{}", tool_a_ready.display()),
            "install": format!("sh -c 'echo tool_a >> \"{}\" && touch \"{}\"'", tool_lifecycle_log.display(), tool_a_ready.display())
        });
        tools_cfg["tools"]["tool_b"] = serde_json::json!({
            "ready_check": format!("app:{}", tool_b_ready.display()),
            "install": format!("sh -c 'echo tool_b >> \"{}\" && touch \"{}\"'", tool_lifecycle_log.display(), tool_b_ready.display())
        });
        fs::write(&tools_path, serde_json::to_string_pretty(&tools_cfg).unwrap()).unwrap();
        let mut local_with_tools = local_cfg.clone();
        local_with_tools["tools"] = serde_json::json!({
            "tool_a": {"enabled": true},
            "tool_b": {"enabled": true}
        });
        fs::write(data.join("config/manager.local.json"), serde_json::to_string_pretty(&local_with_tools).unwrap()).unwrap();

        let tool_apply = run_engine_with_layout(
            &data, Some(&python), Some(&rust), true,
            "tools:apply",
            &["--only".into(), "tools:tool_a".into()],
            Some("pkg-tool-apply-a-01")
        ).expect("tools:apply only tool_a");
        assert!(tool_apply.ok, "tools:apply should succeed: {:?}", tool_apply.errors);
        assert!(tool_a_ready.is_file(), "tool_a install must have executed and created ready marker");
        assert!(!tool_b_ready.exists(), "tool_b install must not execute when only tool_a is selected");
        let tool_log_content = fs::read_to_string(&tool_lifecycle_log).unwrap_or_default();
        assert!(tool_log_content.contains("tool_a"), "tool_a lifecycle must be called");
        assert!(!tool_log_content.contains("tool_b"), "tool_b lifecycle must have 0 calls");

        // 5. Selective apply log readability
        let log_file = data.join("logs/agent_manager_sync_pkg-apply-a-01.log");
        assert!(log_file.is_file(), "sync log must exist: {}", log_file.display());
        let log_text = fs::read_to_string(&log_file).unwrap();
        assert!(!log_text.is_empty(), "sync log must not be empty");

        // 6. Snapshot restore recovery: returns claude_config to exact pre-apply bytes
        let restore = run_engine_with_layout(
            &data, Some(&python), Some(&rust), true,
            "snapshots:restore",
            &["--run".into(), "pkg-apply-a-01".into()],
            Some("pkg-restore-01")
        ).expect("snapshots:restore");
        assert!(restore.ok);
        assert_eq!(fs::read(&claude_config).unwrap(), initial_bytes, "restore returns target file to exact pre-apply bytes");

        // 7. Offline network gate with fake git/curl/npm logging plus refusing network-grade profile:pull
        let net_log = sandbox.join("network_calls.log");
        for cmd_name in ["git", "curl", "npm"] {
            let script = format!("#!/bin/sh\necho \"{} $@\" >> \"{}\"\nexit 1\n", cmd_name, net_log.display());
            let script_path = fake_bin.join(cmd_name);
            fs::write(&script_path, script).unwrap();
            #[cfg(unix)]
            fs::set_permissions(&script_path, fs::Permissions::from_mode(0o755)).unwrap();
        }

        let prefs_offline = crate::settings::SettingsPreferences { offline: true, default_dry_run: false, upstream_ttl_sec: 3600 };
        let offline_plan = run_engine_with_preferences(
            &data, Some(&python), Some(&rust), true,
            "mcp:plan",
            &[],
            Some("pkg-offline-01"),
            Some(&prefs_offline)
        ).expect("offline plan");
        assert!(offline_plan.ok);
        assert!(offline_plan.mode.offline, "offline mode must be active");

        // Profile sync with configured repo URL: profile:pull must be refused under offline
        let mut local_with_profile = local_with_tools.clone();
        local_with_profile["profile_sync"] = serde_json::json!({
            "repo_url": "git@YOUR_HOST:YOUR_PROFILE_REPO.git"
        });
        fs::write(data.join("config/manager.local.json"), serde_json::to_string_pretty(&local_with_profile).unwrap()).unwrap();

        let pull_offline = run_engine_with_preferences(
            &data, Some(&python), Some(&rust), true,
            "profile:pull",
            &[],
            Some("pkg-offline-pull-01"),
            Some(&prefs_offline)
        ).expect("profile:pull envelope");
        assert!(!pull_offline.ok, "network-grade profile:pull must be refused in offline mode");
        assert_eq!(pull_offline.errors[0].code, "OFFLINE_NETWORK_DENIED");
        assert!(!net_log.exists() || fs::read_to_string(&net_log).unwrap().is_empty(), "offline operations must make zero network calls to git/curl/npm");

        for cmd_name in ["git", "curl", "npm"] {
            let script_path = fake_bin.join(cmd_name);
            fs::write(&script_path, "#!/bin/sh\nexit 0\n").unwrap();
            #[cfg(unix)]
            fs::set_permissions(&script_path, fs::Permissions::from_mode(0o755)).unwrap();
        }

        // 8. TTL testing: real fresh vs stale difference via UPSTREAM_PROBE_STALE warning
        let now_epoch = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
        let checked_time_60s_ago = {
            let ts = now_epoch - 60;
            let output = std::process::Command::new(&python)
                .arg("-B")
                .args(["-c", &format!("import datetime; print(datetime.datetime.fromtimestamp({ts}, datetime.timezone.utc).strftime('%Y-%m-%dT%H:%M:%SZ'))")])
                .output().expect("format timestamp via packaged python");
            assert!(output.status.success(), "timestamp generator via packaged python must succeed: {}", String::from_utf8_lossy(&output.stderr));
            String::from_utf8_lossy(&output.stdout).trim().to_string()
        };
        let upstream_cache_content = serde_json::json!({
            "entries": {
                "mcp:YOUR_SERVER_A": {
                    "state": "latest",
                    "current": "1.0.0",
                    "upstream": "1.0.0",
                    "checked_at": checked_time_60s_ago
                }
            }
        });
        fs::create_dir_all(data.join("cache")).unwrap();
        fs::write(data.join("cache/upstream.json"), serde_json::to_string_pretty(&upstream_cache_content).unwrap()).unwrap();

        // Fresh TTL (3600s): cache age is 60s <= 3600s, no UPSTREAM_PROBE_STALE warning
        let prefs_fresh = crate::settings::SettingsPreferences { offline: true, default_dry_run: false, upstream_ttl_sec: 3600 };
        let status_fresh = run_engine_with_preferences(
            &data, Some(&python), Some(&rust), true,
            "mcp:status",
            &[],
            Some("pkg-ttl-fresh-01"),
            Some(&prefs_fresh)
        ).expect("fresh status");
        assert!(status_fresh.ok);
        assert!(!status_fresh.warnings.iter().any(|w| w.code == "UPSTREAM_PROBE_STALE"), "fresh cache entry must not produce UPSTREAM_PROBE_STALE warning");

        // Stale TTL (10s): cache age is 60s > 10s, UPSTREAM_PROBE_STALE warning emitted
        let prefs_stale = crate::settings::SettingsPreferences { offline: true, default_dry_run: false, upstream_ttl_sec: 10 };
        let status_stale = run_engine_with_preferences(
            &data, Some(&python), Some(&rust), true,
            "mcp:status",
            &[],
            Some("pkg-ttl-stale-01"),
            Some(&prefs_stale)
        ).expect("stale status");
        assert!(status_stale.ok);
        assert!(status_stale.warnings.iter().any(|w| w.code == "UPSTREAM_PROBE_STALE"), "stale cache entry must produce UPSTREAM_PROBE_STALE warning");

        // 9. Assert zero pyc generated and package immutability (mode, symlinks, byte content)
        assert_eq!(count_matches(&resource, ".pyc"), 0, "no pyc in resources after test");
        assert_eq!(count_matches(&data, ".pyc"), 0, "no pyc in data root after test");
        let after_snapshots = collect_tree_snapshots(&resource);
        assert_eq!(before_snapshots, after_snapshots, "package resources byte-for-byte, mode-for-mode, link-for-link identical");

        if let Some(h) = old_home { std::env::set_var("HOME", h); } else { std::env::remove_var("HOME"); }
        if let Some(cf) = old_cf { std::env::set_var("CFFIXED_USER_HOME", cf); } else { std::env::remove_var("CFFIXED_USER_HOME"); }
        if let Some(pyc) = old_pyc { std::env::set_var("PYTHONDONTWRITEBYTECODE", pyc); }
        if let Some(p) = old_path { std::env::set_var("PATH", p); } else { std::env::remove_var("PATH"); }
    }

    #[test]
    #[ignore = "real packaged resources; run explicitly on the worker-built resource tree"]
    fn real_packaged_resources_idempotency_and_upgrade() {
        let _guard = TEST_SERIAL_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap_or_else(|e| e.into_inner());
        let resource = std::env::var_os("AGENT_MANAGER_TEST_RESOURCE_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/engine"));
        let sandbox = temp_dir("pkg-upgrade-sandbox");
        let home = sandbox.join("home");
        let data = sandbox.join("data/engine");
        fs::create_dir_all(&home).unwrap();

        let old_home = std::env::var_os("HOME");
        let old_cf = std::env::var_os("CFFIXED_USER_HOME");
        std::env::set_var("HOME", &home);
        std::env::set_var("CFFIXED_USER_HOME", &home);

        copy_standalone_resources(&resource, &data).expect("first init");
        let manage_mtime_1 = fs::metadata(data.join("manage.sh")).unwrap().modified().unwrap();

        copy_standalone_resources(&resource, &data).expect("restart copy");
        let manage_mtime_2 = fs::metadata(data.join("manage.sh")).unwrap().modified().unwrap();
        assert_eq!(manage_mtime_1, manage_mtime_2, "restart must not rewrite seed files");

        // 1. User configurations and profile base (per agent-profile/sync_profile.py:122, base is config/manager.synced.base.json alongside manager.synced.json)
        fs::write(data.join("config/manager.local.json"), br#"{"source_edits":["config/claude-hooks.json"],"user_local":"saved"}"#).unwrap();
        fs::write(data.join("config/manager.synced.json"), br#"{"user_synced":"saved"}"#).unwrap();
        fs::write(data.join("config/claude-hooks.json"), br#"{"user_custom_hooks":true}"#).unwrap();
        fs::write(data.join("config/manager.synced.base.json"), br#"{"synced_base_key":"YOUR_SYNCED_BASE_VAL"}"#).unwrap();

        // 2. Caches: upstream MCP cache, GitBackend cache (cache/profile/repo per sync_profile.py:172), tool & skill caches, legacy profile base
        fs::create_dir_all(data.join("cache/tools")).unwrap();
        fs::write(data.join("cache/tools/cache.json"), br#"{"tool_cache":"saved"}"#).unwrap();
        fs::create_dir_all(data.join("cache/skills")).unwrap();
        fs::write(data.join("cache/skills/cache.json"), br#"{"skill_cache":"saved"}"#).unwrap();
        fs::create_dir_all(data.join("cache/profile/repo")).unwrap();
        fs::write(data.join("cache/profile/repo/HEAD"), b"ref: refs/heads/YOUR_BRANCH\n").unwrap();
        fs::write(data.join("cache/profile/repo/sentinel.txt"), b"YOUR_GIT_BACKEND_CACHE_SENTINEL").unwrap();
        fs::write(data.join("cache/profile/synced.base.json"), br#"{"profile_base":"saved"}"#).unwrap(); // preserved for backward-compatibility test
        fs::write(data.join("cache/upstream.json"), br#"{"cached_upstream":"YOUR_UPSTREAM_CACHED_VALUE"}"#).unwrap();
        fs::write(data.join("cache/profile/base.json"), br#"{"cached_profile":"saved"}"#).unwrap();

        // 3. Real working directories in data root:
        //    - MCP working dir: agent-mcp/<server> per src/engine_v1.py:1809
        //    - Tool working dir: agent-tool/<name> per src/manage_runtime_probe.py:595
        //    - Skill working dir: agent-skill/repositories/<name> per src/engine_v1.py:2132
        fs::create_dir_all(data.join("agent-mcp/YOUR_MCP_SERVER")).unwrap();
        fs::write(data.join("agent-mcp/YOUR_MCP_SERVER/sentinel.txt"), b"YOUR_MCP_WORK_DIR_SENTINEL").unwrap();
        fs::create_dir_all(data.join("agent-tool/YOUR_TOOL")).unwrap();
        fs::write(data.join("agent-tool/YOUR_TOOL/sentinel.txt"), b"YOUR_TOOL_WORK_DIR_SENTINEL").unwrap();
        let skill_target_in_data = data.join("agent-skill/repositories/YOUR_SKILL_REPO/skills/YOUR_SKILL/SKILL.md");
        fs::create_dir_all(skill_target_in_data.parent().unwrap()).unwrap();
        fs::write(&skill_target_in_data, b"---\nname: YOUR_SKILL\ndescription: YOUR_SKILL sentinel\n---\nYOUR_SKILL_CONTENT\n").unwrap();

        // 4. Assistant skill symlink in isolated HOME pointing to data repository:
        //    Assistants in user HOME link to the stable skill cache in data root (home -> data)
        let home_assistant_skill = home.join(".claude/skills/YOUR_SKILL/SKILL.md");
        fs::create_dir_all(home_assistant_skill.parent().unwrap()).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&skill_target_in_data, &home_assistant_skill).unwrap();

        // Also retain legacy link (data -> home) to preserve existing assertion
        let target_skill = home.join("my_existing_skill");
        fs::create_dir_all(&target_skill).unwrap();
        fs::create_dir_all(data.join("agent-skill")).unwrap();
        let link_path = data.join("agent-skill/my_existing_skill");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target_skill, &link_path).unwrap();

        // 5. User logs and snapshots
        fs::create_dir_all(data.join("logs")).unwrap();
        fs::write(data.join("logs/user_session.log"), b"audit log").unwrap();
        fs::create_dir_all(data.join("snapshots/user_snap")).unwrap();
        fs::write(data.join("snapshots/user_snap/manifest.json"), br#"{"snapshot":"saved"}"#).unwrap();

        let upgrade_resource = sandbox.join("upgrade_resource");
        fs::create_dir_all(upgrade_resource.join("runtime")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(resource.join("runtime/python"), upgrade_resource.join("runtime/python")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(resource.join("bin"), upgrade_resource.join("bin")).unwrap();

        let manifest_bytes = fs::read(resource.join("standalone-manifest.json")).unwrap();
        let manifest: StandaloneManifest = serde_json::from_slice(&manifest_bytes).unwrap();
        for rel in &manifest.managed_files {
            let src = resource.join(rel);
            let dst = upgrade_resource.join(rel);
            if let Some(parent) = dst.parent() { fs::create_dir_all(parent).unwrap(); }
            fs::copy(&src, &dst).unwrap();
        }

        let mut manage_content = fs::read(upgrade_resource.join("manage.sh")).unwrap();
        manage_content.extend_from_slice(b"\n# UPGRADE_MARKER_V2\n");
        fs::write(upgrade_resource.join("manage.sh"), &manage_content).unwrap();

        let mut manifest_val: serde_json::Value = serde_json::from_slice(&manifest_bytes).unwrap();
        manifest_val["managed_content_sha256"] = serde_json::json!("simulated_upgrade_sha256_v2");
        fs::write(upgrade_resource.join("standalone-manifest.json"), serde_json::to_string_pretty(&manifest_val).unwrap()).unwrap();

        copy_standalone_resources(&upgrade_resource, &data).expect("upgrade copy");

        let updated_manage = fs::read_to_string(data.join("manage.sh")).unwrap();
        assert!(updated_manage.contains("UPGRADE_MARKER_V2"), "manage.sh must be updated by upgrade");

        // Run engine in upgraded data directory to verify post-upgrade execution
        let python = resource.join("runtime/python/bin/python3");
        let rust = resource.join("bin/agent-manager-engine");
        let status = run_engine_with_layout(&data, Some(&python), Some(&rust), true, "mcp:status", &[], Some("pkg-upgrade-status-01"))
            .expect("engine runs successfully in upgraded data directory");
        assert!(status.ok);
        assert_eq!(status.command, "mcp.status");

        // Verify all sentinels, caches, base config, and working directories are byte-for-byte preserved after upgrade and status run
        assert_eq!(fs::read(data.join("config/manager.local.json")).unwrap(), br#"{"source_edits":["config/claude-hooks.json"],"user_local":"saved"}"#);
        assert_eq!(fs::read(data.join("config/manager.synced.json")).unwrap(), br#"{"user_synced":"saved"}"#);
        assert_eq!(fs::read(data.join("config/claude-hooks.json")).unwrap(), br#"{"user_custom_hooks":true}"#, "source_edits edited file must be preserved");
        assert_eq!(fs::read(data.join("config/manager.synced.base.json")).unwrap(), br#"{"synced_base_key":"YOUR_SYNCED_BASE_VAL"}"#, "config/manager.synced.base.json must be preserved");
        assert_eq!(fs::read(data.join("cache/tools/cache.json")).unwrap(), br#"{"tool_cache":"saved"}"#);
        assert_eq!(fs::read(data.join("cache/skills/cache.json")).unwrap(), br#"{"skill_cache":"saved"}"#);
        assert_eq!(fs::read(data.join("cache/profile/synced.base.json")).unwrap(), br#"{"profile_base":"saved"}"#);
        assert_eq!(fs::read(data.join("cache/upstream.json")).unwrap(), br#"{"cached_upstream":"YOUR_UPSTREAM_CACHED_VALUE"}"#);
        assert_eq!(fs::read(data.join("cache/profile/base.json")).unwrap(), br#"{"cached_profile":"saved"}"#);
        assert_eq!(fs::read(data.join("cache/profile/repo/HEAD")).unwrap(), b"ref: refs/heads/YOUR_BRANCH\n", "GitBackend repo cache HEAD must be preserved");
        assert_eq!(fs::read(data.join("cache/profile/repo/sentinel.txt")).unwrap(), b"YOUR_GIT_BACKEND_CACHE_SENTINEL", "GitBackend repo cache sentinel must be preserved");
        assert_eq!(fs::read(data.join("agent-mcp/YOUR_MCP_SERVER/sentinel.txt")).unwrap(), b"YOUR_MCP_WORK_DIR_SENTINEL", "real MCP working directory must be preserved");
        assert_eq!(fs::read(data.join("agent-tool/YOUR_TOOL/sentinel.txt")).unwrap(), b"YOUR_TOOL_WORK_DIR_SENTINEL", "real Tool working directory must be preserved");
        assert_eq!(fs::read(&skill_target_in_data).unwrap(), b"---\nname: YOUR_SKILL\ndescription: YOUR_SKILL sentinel\n---\nYOUR_SKILL_CONTENT\n", "real Skill repository content must be preserved");

        // Verify HOME assistant skill symlink: links home -> data, target preserved, content readable
        #[cfg(unix)]
        {
            assert!(home_assistant_skill.is_symlink(), "assistant skill link in HOME must be a symlink");
            assert_eq!(fs::read_link(&home_assistant_skill).unwrap(), skill_target_in_data, "assistant skill link must point to stable data repository");
            assert_eq!(fs::read(&home_assistant_skill).unwrap(), b"---\nname: YOUR_SKILL\ndescription: YOUR_SKILL sentinel\n---\nYOUR_SKILL_CONTENT\n", "assistant skill content must be readable through symlink");
        }

        assert_eq!(fs::read(data.join("logs/user_session.log")).unwrap(), b"audit log");
        assert_eq!(fs::read(data.join("snapshots/user_snap/manifest.json")).unwrap(), br#"{"snapshot":"saved"}"#);
        #[cfg(unix)]
        assert!(link_path.is_symlink(), "existing skill link must be preserved");

        if let Some(h) = old_home { std::env::set_var("HOME", h); } else { std::env::remove_var("HOME"); }
        if let Some(cf) = old_cf { std::env::set_var("CFFIXED_USER_HOME", cf); } else { std::env::remove_var("CFFIXED_USER_HOME"); }
    }

    // ---- run_engine（假 manage.sh 脚本端到端） ----

    #[test]
    fn run_engine_parses_envelope_from_manage_sh_stdout() {
        let root = temp_dir("run-ok");
        write_manage_sh(&root, &format!("cat <<'EOF'\n{VALID_ENVELOPE}\nEOF"));
        let envelope = run_engine(&root, "mcp:status", &[], None).expect("envelope from script");
        assert!(envelope.ok);
        assert_eq!(envelope.command, "mcp.status");
    }

    #[test]
    fn run_engine_accepts_error_envelope_with_nonzero_exit() {
        let root = temp_dir("run-errors");
        write_manage_sh(&root, &format!("cat <<'EOF'\n{ERROR_ENVELOPE}\nEOF\nexit 1"));
        let envelope = run_engine(&root, "system:audit", &[], None).expect("exit 1 with envelope is still Ok");
        assert!(!envelope.ok);
        assert_eq!(envelope.errors[0].code, "AUDIT_ERRORS");
    }

    #[test]
    fn run_engine_reports_invalid_output_with_exit_code_and_stderr() {
        let root = temp_dir("run-garbage");
        write_manage_sh(&root, "echo 'boom: config missing' >&2\nexit 2");
        let err = run_engine(&root, "mcp:status", &[], None).expect_err("garbage stdout must fail");
        assert_eq!(err.kind, "invalid_output");
        assert_eq!(err.exit_code, Some(2));
        assert!(err.stderr.as_deref().unwrap_or_default().contains("boom: config missing"));
    }

    #[test]
    fn run_engine_rejects_command_outside_whitelist_before_spawn() {
        let root = temp_dir("run-reject");
        write_manage_sh(&root, "echo SHOULD_NOT_RUN");
        let err = run_engine(&root, "policy:drop", &[], None).expect_err("non-contract command must fail");
        assert_eq!(err.kind, "unknown_command");
    }

    #[test]
    fn run_engine_reports_missing_manage_sh() {
        let root = temp_dir("run-missing");
        let err = run_engine(&root, "mcp:status", &[], None).expect_err("missing script must fail");
        assert_eq!(err.kind, "root_not_found");
    }

    #[test]
    fn run_engine_passes_run_id_via_env() {
        // 沙盒证明透传：假 manage.sh 把 $AGENT_MANAGER_RUN_ID 回显到 envelope.run_id。
        let root = temp_dir("run-id-env");
        let script = r#"cat <<EOF
{"contract":"agent-manager/engine/v1","run_id":"${AGENT_MANAGER_RUN_ID:-none}","command":"mcp.status","mode":{"dry_run":false,"offline":false},"lock":{"held":false,"pid":null},"log_file":null,"ok":true,"data":{},"warnings":[],"errors":[]}
EOF"#;
        write_manage_sh(&root, script);
        let envelope = run_engine(&root, "mcp:status", &[], Some("gui-testrun-01")).expect("envelope");
        assert_eq!(envelope.run_id, "gui-testrun-01");
        // 非法 run id 在 spawn 前被拒
        let err = run_engine(&root, "mcp:status", &[], Some("bad id")).expect_err("bad run id must fail");
        assert_eq!(err.kind, "invalid_args");
    }

    #[test]
    fn settings_preferences_reach_child_without_forcing_dry_run() {
        let root = temp_dir("settings-child");
        let body = format!("[ \"$AGENT_MANAGER_UPSTREAM_TTL_SEC\" = 17 ] || exit 7\n[ \"$3\" = --offline ] || exit 8\nfor arg in \"$@\"; do [ \"$arg\" = --dry-run ] && exit 9; done\ncat <<'EOF'\n{VALID_ENVELOPE}\nEOF");
        write_manage_sh(&root, &body);
        let prefs = crate::settings::SettingsPreferences { offline: true, default_dry_run: true, upstream_ttl_sec: 17 };
        let result = run_engine_with_preferences(&root, None, None, false, "mcp:status", &[], Some("settings-child-01"), Some(&prefs)).unwrap();
        assert!(result.ok);
        assert!(!result.mode.dry_run, "defaultDryRun only affects UI confirmation");
    }

    // ---- 真实联调冒烟（手动运行） ----
    //
    // AGENT_MANAGER_ROOT=/path/to/agent-manager cargo test -- --ignored --nocapture
    // 走完整 manage.sh 路径（三层策略合并 + 透传），断言 mcp:status 返回 ok:true。

    #[test]
    #[ignore = "real engine smoke; requires AGENT_MANAGER_ROOT"]
    fn real_manage_sh_mcp_status_returns_ok() {
        let root = std::env::var("AGENT_MANAGER_ROOT").expect("AGENT_MANAGER_ROOT must be set");
        // 指定 run_id：证明 manage.sh 的 AGENT_MANAGER_RUN_ID 透传（RUN_ID 外部指定）生效。
        let envelope = run_engine(Path::new(&root), "mcp:status", &[], Some("gui-smoke-p2c-01"))
            .expect("real mcp:status envelope");
        println!(
            "smoke: command={} ok={} elements={} contract={} run_id={}",
            envelope.command,
            envelope.ok,
            envelope.data["elements"].as_array().map_or(-1, |els| els.len() as i64),
            envelope.contract,
            envelope.run_id,
        );
        assert_eq!(envelope.contract, CONTRACT_ID);
        assert_eq!(envelope.command, "mcp.status");
        assert!(envelope.ok, "mcp:status should report ok:true, errors: {:?}", envelope.errors);
        assert_eq!(envelope.run_id, "gui-smoke-p2c-01", "manage.sh 应透传外部指定的 RUN_ID");
    }
}
