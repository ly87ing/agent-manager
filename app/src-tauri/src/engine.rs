//! Rust sidecar 桥：把前端的引擎调用转发给 `<root>/manage.sh <command> --json`，
//! 并把 stdout 上的 Contract v1 envelope 反序列化、校验基本形状后返回给前端。
//!
//! 设计要点（docs/goal-rust-gui.md / docs/GUI-ARCHITECTURE-PROPOSAL.md §6.1）：
//! - 开发期不调 engine_v1.py 本体，而是经 manage.sh，由它完成三层策略合并与 env 覆盖。
//! - 引擎契约约定：envelope 永远写 stdout（退出码 0/1/2 都可能伴随合法 envelope），
//!   人类可读文本只走 stderr。因此先解析 stdout，解析失败才落到结构化错误。
//! - 桥只允许契约命令面内的只读/刷新命令，args 仅放行 `--only <element-id>`。

use serde::{Deserialize, Serialize};
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

pub const CONTRACT_ID: &str = "agent-manager/engine/v1";
const DEFAULT_TIMEOUT_SEC: u64 = 600;
const STDERR_SUMMARY_LIMIT: usize = 2000;

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
const STATUS_PLAN_DOMAINS: &[&str] = &["agents", "mcp", "skills", "rules", "hooks", "tools", "aliases"];
/// 附加参数白名单：`--only <element-id>`（system:refresh-upstream 单元素刷新）。
const ALLOWED_FLAGS: &[&str] = &["--only"];

pub fn validate_command(command: &str) -> Result<(), EngineError> {
    let rejected = || {
        EngineError::new(
            "unknown_command",
            format!("命令 '{command}' 不在契约命令面内（允许 <domain>:status / <domain>:plan / system:audit / system:refresh-upstream）"),
        )
    };
    let Some((domain, verb)) = command.split_once(':') else {
        return Err(rejected());
    };
    if domain.is_empty() || verb.is_empty() || verb.contains(':') {
        return Err(rejected());
    }
    let allowed = match (domain, verb) {
        ("system", "audit") | ("system", "refresh-upstream") => true,
        ("secrets", "status") => true,
        (d, "status" | "plan") => STATUS_PLAN_DOMAINS.contains(&d),
        _ => false,
    };
    if allowed { Ok(()) } else { Err(rejected()) }
}

fn validate_arg_value(value: &str) -> Result<(), EngineError> {
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
        Err(EngineError::new("invalid_args", format!("非法参数值 '{value}'")))
    }
}

pub fn validate_args(args: &[String]) -> Result<(), EngineError> {
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        if arg.starts_with('-') {
            if !ALLOWED_FLAGS.contains(&arg.as_str()) {
                return Err(EngineError::new("invalid_args", format!("不允许的参数 '{arg}'")));
            }
            index += 1;
            let Some(value) = args.get(index) else {
                return Err(EngineError::new("invalid_args", format!("参数 '{arg}' 缺少取值")));
            };
            validate_arg_value(value)?;
        } else {
            validate_arg_value(arg)?;
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
pub fn run_engine(root: &Path, command: &str, args: &[String]) -> Result<Envelope, EngineError> {
    validate_command(command)?;
    validate_args(args)?;

    let script = root.join("manage.sh");
    if !script.is_file() {
        return Err(EngineError::new(
            "root_not_found",
            format!("manage.sh 不存在: {}", script.display()),
        ));
    }

    let mut cmd = Command::new("bash");
    cmd.arg(&script)
        .arg(command)
        .arg("--json")
        .args(args)
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

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
pub async fn engine_invoke(command: String, args: Option<Vec<String>>) -> Result<Envelope, EngineError> {
    let args = args.unwrap_or_default();
    tauri::async_runtime::spawn_blocking(move || {
        let root = resolve_root()?;
        run_engine(&root, &command, &args)
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
            "agents:status", "agents:plan", "mcp:status", "mcp:plan", "skills:status",
            "rules:plan", "hooks:plan", "tools:status", "aliases:plan", "secrets:status",
            "system:audit", "system:refresh-upstream",
        ] {
            validate_command(command).unwrap_or_else(|err| panic!("{command} should pass: {}", err.message));
        }
    }

    #[test]
    fn command_whitelist_rejects_non_contract_commands() {
        for command in [
            "system:converge", // Phase 2 才存在
            "agents:update",   // manage.sh 管线命令，不是契约命令
            "secrets:plan",    // secrets 只读
            "update",
            "mcp:",
            ":status",
            "mcp:status:extra",
            "mcp:status --offline", // 注入尝试
        ] {
            assert!(validate_command(command).is_err(), "{command} should be rejected");
        }
    }

    // ---- validate_args ----

    #[test]
    fn args_allow_only_flag_with_element_id() {
        validate_args(&[]).unwrap();
        validate_args(&["--only".into(), "mcp:grok-search".into()]).unwrap();
        validate_args(&["skill:agent-skills/find-skills".into()]).unwrap();
    }

    #[test]
    fn args_reject_flags_outside_whitelist_and_shellish_values() {
        for args in [
            vec!["--config".to_string(), "/etc/passwd".to_string()],
            vec!["--only".to_string()],
            vec!["--only".to_string(), "-rf".to_string()],
            vec!["; rm -rf /".to_string()],
            vec!["$(reboot)".to_string()],
            vec!["mcp:status --offline".to_string()],
        ] {
            assert!(validate_args(&args).is_err(), "{args:?} should be rejected");
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

    // ---- run_engine（假 manage.sh 脚本端到端） ----

    #[test]
    fn run_engine_parses_envelope_from_manage_sh_stdout() {
        let root = temp_dir("run-ok");
        write_manage_sh(&root, &format!("cat <<'EOF'\n{VALID_ENVELOPE}\nEOF"));
        let envelope = run_engine(&root, "mcp:status", &[]).expect("envelope from script");
        assert!(envelope.ok);
        assert_eq!(envelope.command, "mcp.status");
    }

    #[test]
    fn run_engine_accepts_error_envelope_with_nonzero_exit() {
        let root = temp_dir("run-errors");
        write_manage_sh(&root, &format!("cat <<'EOF'\n{ERROR_ENVELOPE}\nEOF\nexit 1"));
        let envelope = run_engine(&root, "system:audit", &[]).expect("exit 1 with envelope is still Ok");
        assert!(!envelope.ok);
        assert_eq!(envelope.errors[0].code, "AUDIT_ERRORS");
    }

    #[test]
    fn run_engine_reports_invalid_output_with_exit_code_and_stderr() {
        let root = temp_dir("run-garbage");
        write_manage_sh(&root, "echo 'boom: config missing' >&2\nexit 2");
        let err = run_engine(&root, "mcp:status", &[]).expect_err("garbage stdout must fail");
        assert_eq!(err.kind, "invalid_output");
        assert_eq!(err.exit_code, Some(2));
        assert!(err.stderr.as_deref().unwrap_or_default().contains("boom: config missing"));
    }

    #[test]
    fn run_engine_rejects_command_outside_whitelist_before_spawn() {
        let root = temp_dir("run-reject");
        write_manage_sh(&root, "echo SHOULD_NOT_RUN");
        let err = run_engine(&root, "system:converge", &[]).expect_err("non-contract command must fail");
        assert_eq!(err.kind, "unknown_command");
    }

    #[test]
    fn run_engine_reports_missing_manage_sh() {
        let root = temp_dir("run-missing");
        let err = run_engine(&root, "mcp:status", &[]).expect_err("missing script must fail");
        assert_eq!(err.kind, "root_not_found");
    }

    // ---- 真实联调冒烟（手动运行） ----
    //
    // AGENT_MANAGER_ROOT=/path/to/agent-manager cargo test -- --ignored --nocapture
    // 走完整 manage.sh 路径（三层策略合并 + 透传），断言 mcp:status 返回 ok:true。

    #[test]
    #[ignore = "real engine smoke; requires AGENT_MANAGER_ROOT"]
    fn real_manage_sh_mcp_status_returns_ok() {
        let root = std::env::var("AGENT_MANAGER_ROOT").expect("AGENT_MANAGER_ROOT must be set");
        let envelope = run_engine(Path::new(&root), "mcp:status", &[]).expect("real mcp:status envelope");
        println!(
            "smoke: command={} ok={} elements={} contract={}",
            envelope.command,
            envelope.ok,
            envelope.data["elements"].as_array().map_or(-1, |els| els.len() as i64),
            envelope.contract,
        );
        assert_eq!(envelope.contract, CONTRACT_ID);
        assert_eq!(envelope.command, "mcp.status");
        assert!(envelope.ok, "mcp:status should report ok:true, errors: {:?}", envelope.errors);
    }
}
