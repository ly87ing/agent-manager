//! Rust 原生 agents 域。接入冻结公共接口：crate::native::{Context, Request}、crate::store。
//!
//! 语义：
//! - 定义唯一来源 = store::effective_policy 的 agents.definitions；
//!   内置定义直接消费 store::BUILTIN_POLICY_JSON，不在此处复制。
//! - 显式动作仅 install/upgrade/uninstall；status 只对已存在二进制执行固定 `--version` 探测。
//! - 命令只在真实 apply 执行；plan、apply --dry-run、取消零执行零写。
//! - apply 输出对齐前端 ApplyResultData {domain, dry_run, steps, snapshot, skipped}；
//!   命令 exit 0 后必须探测到安装/卸载结果，否则报错而非谎报成功。

use anyhow::{anyhow, bail, Result};
use serde_json::{json, Map, Value};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::native::{Context, Request};
use crate::store;

const APPLY_TIMEOUT: Duration = Duration::from_secs(600);

// ---------- 定义 ----------

/// 单条命令的来源（status 明确展示）。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Source {
    Builtin,
    Custom,
}

impl Source {
    fn as_str(self) -> &'static str {
        match self {
            Source::Builtin => "builtin",
            Source::Custom => "custom",
        }
    }
}

#[derive(Default, Clone)]
struct CommandSet {
    install: Option<(String, Source)>,
    upgrade: Option<(String, Source)>,
    uninstall: Option<(String, Source)>,
}

struct AgentDef {
    id: String,
    display_name: String,
    bin: String,
    commands: CommandSet,
    skills_dir: Option<String>,
    mcp_config: Option<String>,
    origin: Source,
    enabled: bool,
    spec: Value,
    preferred_provider: String,
    providers: Map<String, Value>,
}

fn command_slot<'a>(
    set: &'a mut CommandSet,
    key: &str,
) -> Option<&'a mut Option<(String, Source)>> {
    match key {
        "install" => Some(&mut set.install),
        "upgrade" => Some(&mut set.upgrade),
        "uninstall" => Some(&mut set.uninstall),
        _ => None,
    }
}

fn provider_command(
    cfg: &Map<String, Value>,
    key: &str,
    source: Source,
) -> Result<Option<(String, Source)>> {
    match cfg.get(key) {
        None => Ok(None),
        Some(Value::String(command)) => Ok(Some((command.clone(), source))),
        Some(Value::Null) => Ok(None),
        Some(value) => bail!("命令 '{}' 必须是字符串或 null，得到 {}", key, value),
    }
}

fn strip_internal_markers(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .filter(|(key, _)| key.as_str() != "_replace" && key.as_str() != "_delete")
                .map(|(key, value)| (key.clone(), strip_internal_markers(value)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(strip_internal_markers).collect()),
        _ => value.clone(),
    }
}

fn parse_definitions(policy: &Value, builtin: &Value) -> Result<Vec<AgentDef>> {
    let definitions = policy
        .get("agents")
        .and_then(|v| v.get("definitions"))
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("agents.definitions 必须是对象"))?;
    let builtin_defs = builtin
        .get("agents")
        .and_then(|v| v.get("definitions"))
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("内置 agents.definitions 必须是对象"))?;
    let mut defs = Vec::new();
    let agent_policy = policy.get("agents").and_then(Value::as_object);
    for (id, raw) in definitions {
        let raw = raw
            .as_object()
            .ok_or_else(|| anyhow!("agents.definitions 中 '{}' 的定义必须是对象", id))?;
        if raw.get("_delete") == Some(&Value::Bool(true)) {
            continue;
        }
        let builtin_raw = builtin_defs.get(id).and_then(Value::as_object);
        let display_name = raw
            .get("display_name")
            .and_then(Value::as_str)
            .unwrap_or(id)
            .to_string();
        let bin = raw
            .get("bin")
            .and_then(Value::as_str)
            .unwrap_or(id)
            .to_string();
        let skills_dir = raw
            .get("skills_dir")
            .and_then(Value::as_str)
            .map(str::to_string);
        let mcp_config = raw
            .get("mcp_config")
            .and_then(|v| v.get("path").or(Some(v)))
            .and_then(Value::as_str)
            .map(str::to_string);
        let providers = raw
            .get("providers")
            .and_then(Value::as_object)
            .ok_or_else(|| anyhow!("agents.definitions 中 '{}' 缺少 providers", id))?;
        let preferred = raw
            .get("preferred_provider")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("agents.definitions 中 '{}' 缺少 preferred_provider", id))?;
        let cfg = providers
            .get(preferred)
            .and_then(Value::as_object)
            .ok_or_else(|| {
                anyhow!(
                    "agents.definitions 中 '{}' 的 preferred_provider 未在 providers 声明",
                    id
                )
            })?;
        let builtin_cfg = builtin_raw
            .and_then(|v| v.get("providers"))
            .and_then(|v| v.get(preferred))
            .and_then(Value::as_object);
        let mut commands = CommandSet::default();
        for key in ["install", "upgrade", "uninstall"] {
            let source = if builtin_cfg.and_then(|b| b.get(key)) == cfg.get(key) {
                Source::Builtin
            } else {
                Source::Custom
            };
            if let Some(slot) = command_slot(&mut commands, key) {
                *slot = provider_command(cfg, key, source)?;
            }
        }
        let origin = if builtin_raw == Some(raw) {
            Source::Builtin
        } else {
            Source::Custom
        };
        let enabled = agent_policy
            .and_then(|agents| agents.get(id))
            .and_then(|value| value.get("enabled"))
            .and_then(Value::as_bool)
            .unwrap_or(true);
        defs.push(AgentDef {
            id: id.clone(),
            display_name,
            bin,
            commands,
            skills_dir,
            mcp_config,
            origin,
            enabled,
            spec: strip_internal_markers(&Value::Object(raw.clone())),
            preferred_provider: preferred.to_string(),
            providers: providers.clone(),
        });
    }
    Ok(defs)
}

fn load_definitions(ctx: &Context) -> Result<Vec<AgentDef>> {
    let policy = store::effective_policy(ctx)?;
    parse_definitions(&policy, &store::builtin_policy())
}

// ---------- 只读探测 ----------

/// PATH 与 ~/.local/bin 下解析 bin（home_dir 注入，测试替身）。
fn resolve_bin(home_dir: &Path, bin: &str) -> Option<PathBuf> {
    if bin.is_empty() {
        return None;
    }
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).collect())
        .unwrap_or_default();
    dirs.push(home_dir.join(".local").join("bin"));
    dirs.into_iter()
        .map(|dir| dir.join(bin))
        .find(|p| p.is_file())
}

struct CmdOutput {
    exit_code: i32,
}

/// 持续读出管道且不保存内容，防大输出撑爆管道或把命令输出带入 envelope。
/// 读完经 channel 交还；shell 已退出但孙进程占用管道时主流程 recv_timeout 兜底，不无限 join。
fn spawn_drain_reader(mut pipe: impl Read + Send + 'static) -> std::sync::mpsc::Receiver<()> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = vec![0u8; 4096];
        while let Ok(n) = pipe.read(&mut buf) {
            if n == 0 {
                break;
            }
        }
        let _ = tx.send(());
    });
    rx
}

fn spawn_capture_reader(
    mut pipe: impl Read + Send + 'static,
    limit: usize,
) -> std::sync::mpsc::Receiver<Vec<u8>> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = [0u8; 256];
        let mut captured = Vec::with_capacity(limit);
        while let Ok(n) = pipe.read(&mut buf) {
            if n == 0 {
                break;
            }
            let remaining = limit.saturating_sub(captured.len());
            captured.extend_from_slice(&buf[..n.min(remaining)]);
        }
        let _ = tx.send(captured);
    });
    rx
}

/// 等待子进程并并发有界读取输出；读取端 recv_timeout 兜底（孙进程持管道不阻塞主流程）。
fn wait_bounded(
    mut child: std::process::Child,
    timeout: Duration,
    action: &str,
) -> Result<CmdOutput> {
    let stdout_rx = spawn_drain_reader(child.stdout.take().expect("已 piped"));
    let stderr_rx = spawn_drain_reader(child.stderr.take().expect("已 piped"));
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|e| anyhow!("等待 Agent 动作失败（{}）：{}", action, e))?
        {
            break status;
        }
        if start.elapsed() > timeout {
            terminate_child(&mut child);
            let _ = child.wait();
            bail!(
                "Agent 动作执行超时（{} 秒上限），已终止（{}）",
                timeout.as_secs(),
                action
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let grace = Duration::from_secs(2);
    let _ = stdout_rx.recv_timeout(grace);
    let _ = stderr_rx.recv_timeout(grace);
    Ok(CmdOutput {
        exit_code: status.code().unwrap_or(-1),
    })
}

/// unix：子进程独立进程组（std process_group(0)），超时经平台 kill API 终止整个进程组，
/// 不只 kill shell 留下安装器，也不依赖 PATH 里的外部 kill 命令。
#[cfg(unix)]
fn terminate_child(child: &mut std::process::Child) {
    extern "C" {
        fn kill(pid: i32, sig: i32) -> i32;
    }
    const SIGKILL: i32 = 9;
    let pgid = child.id() as i32; // process_group(0) 使 pgid == pid
                                  // 先整组；组不存在（已退出）时退回直接子进程
    if unsafe { kill(-pgid, SIGKILL) } != 0 {
        let _ = child.kill();
    }
}

#[cfg(not(unix))]
fn terminate_child(child: &mut std::process::Child) {
    let _ = child.kill();
}

/// 经 sh -c 执行（POSIX），HOME 注入 home_dir。非 unix 平台明确拒绝而非编译失败。
#[cfg(unix)]
fn run_shell(home_dir: &Path, cmd: &str, timeout: Duration, action: &str) -> Result<CmdOutput> {
    use std::os::unix::process::CommandExt;
    let mut command = Command::new("sh");
    command
        .arg("-c")
        .arg(cmd)
        .env("HOME", home_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let child = command
        .spawn()
        .map_err(|e| anyhow!("无法启动 Agent 动作（{}）: {}", action, e))?;
    wait_bounded(child, timeout, action)
}

#[cfg(not(unix))]
fn run_shell(_home_dir: &Path, _cmd: &str, _timeout: Duration, action: &str) -> Result<CmdOutput> {
    bail!("agents 域动作 '{}' 在当前平台不可执行", action)
}

/// status 的版本探测：只对已解析出的实际二进制执行固定 `--version`，不执行策略中的 shell。
fn probe_version(bin_path: &Path) -> Option<String> {
    let mut command = Command::new(bin_path);
    command
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command.spawn().ok()?;
    let stdout_rx = spawn_capture_reader(child.stdout.take()?, 256);
    let stderr_rx = spawn_capture_reader(child.stderr.take()?, 256);
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait().ok()? {
            if status.code() != Some(0) {
                return None;
            }
            let stdout = stdout_rx
                .recv_timeout(Duration::from_secs(2))
                .unwrap_or_default();
            let stderr = stderr_rx
                .recv_timeout(Duration::from_secs(2))
                .unwrap_or_default();
            let text = format!(
                "{}{}",
                String::from_utf8_lossy(&stdout),
                String::from_utf8_lossy(&stderr)
            );
            return text
                .split_whitespace()
                .find(|token| token.chars().next().is_some_and(|c| c.is_ascii_digit()))
                .map(str::to_string);
        }
        if start.elapsed() > Duration::from_secs(10) {
            terminate_child(&mut child);
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

// ---------- status ----------

fn action_descriptor(id: &str) -> Value {
    let risk = match id {
        "install" | "uninstall" => "install-remove",
        _ => "network",
    };
    json!({"id": id, "risk": risk, "needs_lock": true, "offline_ok": id == "uninstall"})
}

fn command_sources(commands: &CommandSet) -> Value {
    let mut map = Map::new();
    for (key, slot) in [
        ("install", &commands.install),
        ("upgrade", &commands.upgrade),
        ("uninstall", &commands.uninstall),
    ] {
        if let Some((_, source)) = slot {
            map.insert(key.to_string(), json!(source.as_str()));
        }
    }
    Value::Object(map)
}

fn detected_provider(home_dir: &Path, bin_path: &Path) -> &'static str {
    let canonical = bin_path
        .canonicalize()
        .unwrap_or_else(|_| bin_path.to_path_buf());
    let native_dir = home_dir
        .canonicalize()
        .unwrap_or_else(|_| home_dir.to_path_buf())
        .join(".local")
        .join("bin");
    let path = canonical.to_string_lossy();
    if canonical.starts_with(native_dir) {
        "native"
    } else if canonical
        .components()
        .any(|component| component.as_os_str() == "Cellar")
        || path.starts_with("/opt/homebrew/")
        || path.contains("/homebrew/")
    {
        "brew"
    } else {
        "system"
    }
}

fn classified_provider(def: &AgentDef, provider: &str) -> &'static str {
    if provider == "brew" {
        return "unsupported";
    }
    match def.providers.get(provider).and_then(Value::as_object) {
        Some(config) if config.get("deprecated") == Some(&Value::Bool(true)) => "deprecated",
        Some(config) if config.get("managed") == Some(&Value::Bool(true)) => "managed",
        Some(_) => "system_only",
        None if provider == "system" => "system_only",
        None => "unsupported",
    }
}

fn status_element(ctx: &Context, def: &AgentDef) -> Value {
    let bin_path = resolve_bin(&ctx.home_dir, &def.bin);
    let installed = bin_path.is_some();
    let version = if installed {
        bin_path.as_deref().and_then(probe_version)
    } else {
        None
    };
    let actual_provider = bin_path
        .as_deref()
        .map(|path| detected_provider(&ctx.home_dir, path));
    let bucket = actual_provider.map(|provider| classified_provider(def, provider));
    let manageable = bucket == Some("managed");
    let state = if !def.enabled {
        "disabled"
    } else if installed && !manageable {
        "unmanaged-detected"
    } else if installed {
        "managed-ok"
    } else {
        "missing"
    };

    let mut actions = Vec::new();
    if def.enabled && !installed && def.commands.install.is_some() {
        actions.push(action_descriptor("install"));
    }
    if def.enabled && installed && manageable {
        if def.commands.upgrade.is_some() {
            actions.push(action_descriptor("upgrade"));
        }
        if def.commands.uninstall.is_some() {
            actions.push(action_descriptor("uninstall"));
        }
    }

    let mut observed = Map::new();
    observed.insert("installed".into(), json!(installed));
    observed.insert(
        "bin_path".into(),
        bin_path
            .map(|p| json!(p.display().to_string()))
            .unwrap_or(Value::Null),
    );
    observed.insert(
        "version".into(),
        version.clone().map(Value::String).unwrap_or(Value::Null),
    );
    observed.insert("definition_origin".into(), json!(def.origin.as_str()));
    observed.insert("command_sources".into(), command_sources(&def.commands));
    let mut classified = Map::new();
    for bucket_name in ["managed", "deprecated", "system_only", "unsupported"] {
        classified.insert(
            bucket_name.to_string(),
            Value::Array(
                actual_provider
                    .filter(|provider| classified_provider(def, provider) == bucket_name)
                    .map(|provider| Value::String(provider.to_string()))
                    .into_iter()
                    .collect(),
            ),
        );
    }
    observed.insert(
        "providers".into(),
        actual_provider
            .map(|provider| json!([provider]))
            .unwrap_or_else(|| json!([])),
    );
    observed.insert("classified".into(), Value::Object(classified));
    observed.insert(
        "preferred_active".into(),
        json!(actual_provider == Some(def.preferred_provider.as_str())),
    );

    json!({
        "id": format!("agents:{}", def.id),
        "domain": "agents",
        "spec": def.spec.clone(),
        "policy": {"enabled": def.enabled},
        "observed": Value::Object(observed),
        "state": state,
        "freshness": {"state": "not-applicable", "current": version.clone(), "upstream": null, "checked_at": null, "probe": "native", "cache_age_sec": null},
        "actions": actions,
    })
}

// ---------- plan / apply ----------

struct ActionRequest {
    agent_id: String,
    action: String,
    dry_run: bool,
    offline: bool,
}

fn parse_action_args(req: &Request) -> Result<ActionRequest> {
    let only = req
        .value("--only")
        .ok_or_else(|| anyhow!("agents:plan/apply 需要显式 --only agents:<id>"))?;
    let agent_id = only
        .strip_prefix("agents:")
        .ok_or_else(|| anyhow!("--only 只接受 agents:<id>，得到 '{}'", only))?
        .to_string();
    let action = req
        .value("--action")
        .ok_or_else(|| {
            anyhow!("agents:plan/apply 需要显式 --action install|upgrade|uninstall（不自动安装）")
        })?
        .to_string();
    if !["install", "upgrade", "uninstall"].contains(&action.as_str()) {
        bail!(
            "agents 域不支持动作 '{}'，可选 install/upgrade/uninstall",
            action
        );
    }
    Ok(ActionRequest {
        agent_id,
        action,
        dry_run: req.flag("--dry-run"),
        offline: req.flag("--offline"),
    })
}

fn resolve_action_command<'a>(def: &'a AgentDef, action: &str) -> Result<&'a str> {
    let slot = match action {
        "install" => &def.commands.install,
        "upgrade" => &def.commands.upgrade,
        "uninstall" => &def.commands.uninstall,
        _ => bail!("agents 域不支持动作 '{}'", action),
    };
    slot.as_ref()
        .map(|(cmd, _)| cmd.as_str())
        .ok_or_else(|| anyhow!("Agent '{}' 未定义 '{}' 命令，不伪造命令", def.id, action))
}

fn check_action_allowed(
    ctx: &Context,
    def: &AgentDef,
    action: &str,
    installed: bool,
) -> Result<()> {
    if !def.enabled {
        bail!("Agent '{}' 已禁用，拒绝执行动作", def.id);
    }
    if installed && matches!(action, "upgrade" | "uninstall") {
        let provider = resolve_bin(&ctx.home_dir, &def.bin)
            .as_deref()
            .map(|path| detected_provider(&ctx.home_dir, path));
        if provider.map(|name| classified_provider(def, name)) != Some("managed") {
            bail!(
                "Agent '{}' 当前安装来源不可管理，拒绝执行 {}",
                def.id,
                action
            );
        }
    }
    match action {
        "install" if installed => bail!("Agent '{}' 已安装，install 不适用", def.id),
        "upgrade" | "uninstall" if !installed => {
            bail!("Agent '{}' 未安装，{} 不适用", def.id, action)
        }
        _ => Ok(()),
    }
}

fn canonical_command(command: &str) -> String {
    command.replacen('.', ":", 1)
}

fn prepare_apply(ctx: &Context, req: &Request) -> Result<(ActionRequest, AgentDef)> {
    let defs = load_definitions(ctx)?;
    let parsed = parse_action_args(req)?;
    let def = find_def(&defs, &parsed.agent_id)?;
    let installed = resolve_bin(&ctx.home_dir, &def.bin).is_some();
    check_action_allowed(ctx, def, &parsed.action, installed)?;
    if parsed.offline && parsed.action != "uninstall" {
        bail!(
            "agents:{} 在 offline 模式下拒绝执行 {}（需要联网）",
            def.id,
            parsed.action
        );
    }
    resolve_action_command(def, &parsed.action)?;
    Ok((
        parsed,
        AgentDef {
            id: def.id.clone(),
            display_name: def.display_name.clone(),
            bin: def.bin.clone(),
            commands: def.commands.clone(),
            skills_dir: def.skills_dir.clone(),
            mcp_config: def.mcp_config.clone(),
            origin: def.origin,
            enabled: def.enabled,
            spec: def.spec.clone(),
            preferred_provider: def.preferred_provider.clone(),
            providers: def.providers.clone(),
        },
    ))
}

/// Apply preflight is read-only and runs before the process lock is created.
/// Returns whether this is a real (non-dry-run) apply.
pub fn preflight_apply(ctx: &Context, req: &Request) -> Result<bool> {
    let (parsed, _) = prepare_apply(ctx, req)?;
    Ok(!parsed.dry_run)
}

fn find_def<'a>(defs: &'a [AgentDef], agent_id: &str) -> Result<&'a AgentDef> {
    defs.iter()
        .find(|def| def.id == agent_id)
        .ok_or_else(|| anyhow!("元素 'agents:{}' 在域 'agents' 中不存在", agent_id))
}

fn plan_data(def: &AgentDef, action: &str, cmd: &str) -> Value {
    let risk = match action {
        "upgrade" => "network",
        _ => "install-remove",
    };
    let label = match action {
        "install" => "安装",
        "upgrade" => "升级",
        "uninstall" => "卸载",
        _ => unreachable!("validated agent action"),
    };
    let mut summary = Map::new();
    for key in [
        "read",
        "write-local",
        "exec-local",
        "network",
        "install-remove",
    ] {
        summary.insert(key.to_string(), json!(if key == risk { 1 } else { 0 }));
    }
    json!({
        "domain": "agents",
        "actions": [{
            "op": "run-cmd",
            "element": format!("agents:{}", def.id),
            "target": Value::Null,
            "risk": risk,
            "before_digest": Value::Null,
            "after_digest": Value::Null,
            "backup_path": Value::Null,
            "cmd": cmd,
            "note": format!("{} {}（显式动作）", label, def.id),
        }],
        "summary": Value::Object(summary),
    })
}

/// exit 0 后的结果核验：install/upgrade → bin 必须可解析；uninstall → bin 必须消失。
fn verify_outcome(home_dir: &Path, def: &AgentDef, action: &str) -> Result<()> {
    let installed = resolve_bin(home_dir, &def.bin).is_some();
    match action {
        "install" | "upgrade" if !installed => bail!(
            "{} {} 的命令退出码为 0，但未探测到 '{}' 可执行文件，不能报成功",
            action,
            def.id,
            def.bin
        ),
        "uninstall" if installed => bail!(
            "uninstall {} 的命令退出码为 0，但 '{}' 仍可解析，不能报成功",
            def.id,
            def.bin
        ),
        _ => Ok(()),
    }
}

fn record_apply_operation(
    ctx: &Context,
    req: &Request,
    def: &AgentDef,
    action: &str,
    ok: bool,
    exit_code: Option<i32>,
) -> Result<()> {
    let mut conn = store::open_write(ctx)?;
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    store::record_operation(
        &tx,
        Some(&req.run_id),
        &format!("agents:{action}"),
        Some(&format!("agents:{}", def.id)),
        Some(&json!({"action": action, "ok": ok, "exit_code": exit_code})),
    )?;
    tx.commit()?;
    Ok(())
}

// ---------- handle ----------

pub fn handle(ctx: &Context, req: &Request) -> Result<Value> {
    let defs = load_definitions(ctx)?;
    match canonical_command(&req.command).as_str() {
        "agents:status" => {
            let elements: Vec<Value> = defs.iter().map(|def| status_element(ctx, def)).collect();
            Ok(json!({"domain": "agents", "elements": elements}))
        }
        "agents:plan" => {
            let parsed = parse_action_args(req)?;
            let def = find_def(&defs, &parsed.agent_id)?;
            let installed = resolve_bin(&ctx.home_dir, &def.bin).is_some();
            check_action_allowed(ctx, def, &parsed.action, installed)?;
            if parsed.offline && parsed.action != "uninstall" {
                bail!(
                    "agents:{} 在 offline 模式下拒绝执行 {}（需要联网）",
                    def.id,
                    parsed.action
                );
            }
            resolve_action_command(def, &parsed.action)?;
            Ok(plan_data(
                def,
                &parsed.action,
                resolve_action_command(def, &parsed.action)?,
            ))
        }
        "agents:apply" => {
            let parsed = parse_action_args(req)?;
            let def = find_def(&defs, &parsed.agent_id)?;
            let installed = resolve_bin(&ctx.home_dir, &def.bin).is_some();
            check_action_allowed(ctx, def, &parsed.action, installed)?;
            if parsed.offline && parsed.action != "uninstall" {
                bail!(
                    "agents:{} 在 offline 模式下拒绝执行 {}（需要联网）",
                    def.id,
                    parsed.action
                );
            }
            let cmd = resolve_action_command(def, &parsed.action)?.to_string();
            if parsed.dry_run {
                return Ok(json!({
                    "domain": "agents",
                    "dry_run": true,
                    "steps": [],
                    "snapshot": Value::Null,
                    "skipped": [&parsed.agent_id],
                }));
            }
            let output = match run_shell(&ctx.home_dir, &cmd, APPLY_TIMEOUT, &parsed.action) {
                Ok(output) => output,
                Err(err) => {
                    record_apply_operation(ctx, req, def, &parsed.action, false, None).map_err(
                        |log_err| anyhow!("Agent 动作失败，且 operation log 写入失败：{log_err}"),
                    )?;
                    return Err(err);
                }
            };
            let step_name = format!("{} {}", parsed.action, def.id);
            if output.exit_code != 0 {
                record_apply_operation(
                    ctx,
                    req,
                    def,
                    &parsed.action,
                    false,
                    Some(output.exit_code),
                )?;
                bail!(
                    "{} {} 失败（退出码 {}）",
                    parsed.action,
                    def.id,
                    output.exit_code
                );
            }
            if let Err(err) = verify_outcome(&ctx.home_dir, def, &parsed.action) {
                record_apply_operation(
                    ctx,
                    req,
                    def,
                    &parsed.action,
                    false,
                    Some(output.exit_code),
                )?;
                return Err(err);
            }
            record_apply_operation(ctx, req, def, &parsed.action, true, Some(output.exit_code))?;
            Ok(json!({
                "domain": "agents",
                "dry_run": false,
                "steps": [{"name": step_name, "ok": true, "exit_code": output.exit_code}],
                "snapshot": Value::Null,
                "skipped": [],
            }))
        }
        other => bail!("native_agents 不支持命令 '{}'", other),
    }
}
