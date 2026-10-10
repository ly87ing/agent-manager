//! Rust 原生 agents 域。接入冻结公共接口：crate::native::{Context, Request}、crate::store。
//!
//! 语义：
//! - 定义唯一来源 = store::effective_policy 的 agents.definitions；
//!   内置定义直接消费 store::BUILTIN_POLICY_JSON，不在此处复制。
//! - 显式动作仅 install/upgrade/uninstall；status 只对已存在二进制执行固定 `--version` 探测。
//! - 命令只在真实 apply 执行；plan 与 --dry-run 已退役，apply 一律真实执行并持有全局锁。
//! - apply 输出为 {domain, steps, skipped}；
//!   命令 exit 0 后必须探测到安装/卸载结果，否则报错而非谎报成功。

use crate::platform::Canon;
use anyhow::{anyhow, bail, Result};
use serde_json::{json, Map, Value};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use crate::native::{Context, Request};
use crate::store;

const APPLY_TIMEOUT: Duration = Duration::from_secs(600);

// ---------- 定义 ----------

/// 单条命令的来源（status 明确展示）。
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Source {
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
pub(crate) struct CommandSet {
    pub(crate) install: Option<(String, Source)>,
    upgrade: Option<(String, Source)>,
    uninstall: Option<(String, Source)>,
}

pub(crate) struct AgentDef {
    pub(crate) id: String,
    display_name: String,
    pub(crate) bin: String,
    pub(crate) commands: CommandSet,
    skills_dir: Option<String>,
    mcp_config: Option<String>,
    origin: Source,
    pub(crate) enabled: bool,
    spec: Value,
    preferred_provider: String,
    providers: Map<String, Value>,
    /// 查最新版去哪个 npm 包看。
    ///
    /// 官方安装器装的助手（curl install.sh）没有包名可推，但它们在 npm 上发的是同一份
    /// 版本号，显式声明一下就能查到新版本；不声明的助手不参与探测，不瞎猜。
    upstream_npm: Option<String>,
}

impl AgentDef {
    /// 显式声明的上游探测包名。
    pub(crate) fn upstream_npm(&self) -> Option<&str> {
        self.upstream_npm.as_deref()
    }

    /// 定义原文。别的域（如运行参数）从这里读自己那一段，不另开一套加载。
    pub(crate) fn spec_ref(&self) -> &Value {
        &self.spec
    }

    /// 定义里写的客户端配置文件位置。Codex 的运行参数与 MCP 写的是同一个
    /// `config.toml`，实例各有各的 CODEX_HOME，照定义走才不会张冠李戴。
    pub(crate) fn config_path(&self) -> Option<&str> {
        self.mcp_config.as_deref()
    }
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

pub(crate) fn strip_internal_markers(value: &Value) -> Value {
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
        // 实例（带 profile 段）借用基础助手的软件，自己没有安装渠道：界面建的实例定义不带
        // providers，缺省按空渠道处理，否则一个实例就会让整个 agents:status 失败。
        let no_providers = Map::new();
        let instance_without_providers =
            raw.get("profile").is_some() && raw.get("providers").is_none();
        let providers = if instance_without_providers {
            &no_providers
        } else {
            raw.get("providers")
                .and_then(Value::as_object)
                .ok_or_else(|| anyhow!("agents.definitions 中 '{}' 缺少 providers", id))?
        };
        let preferred = if instance_without_providers {
            "native"
        } else {
            raw.get("preferred_provider")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("agents.definitions 中 '{}' 缺少 preferred_provider", id))?
        };
        let cfg = if instance_without_providers {
            &no_providers
        } else {
            providers
                .get(preferred)
                .and_then(Value::as_object)
                .ok_or_else(|| {
                    anyhow!(
                        "agents.definitions 中 '{}' 的 preferred_provider 未在 providers 声明",
                        id
                    )
                })?
        };
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
            upstream_npm: raw
                .get("upstream_npm")
                .and_then(Value::as_str)
                .map(str::to_string),
        });
    }
    Ok(defs)
}

pub(crate) fn load_definitions(ctx: &Context) -> Result<Vec<AgentDef>> {
    let policy = store::effective_policy(ctx)?;
    parse_definitions(&policy, &store::builtin_policy())
}

// ---------- 只读探测 ----------

/// PATH 之外还要查的常见安装目录：官方安装器、npm/volta/bun 全局目录与 Homebrew。
/// 桌面应用从访达启动时 PATH 只有系统目录，不补这些就找不到用户装好的助手
/// （CC Switch 等同类工具查的也是这一组）。
pub(crate) fn fallback_bin_dirs(home_dir: &Path) -> Vec<PathBuf> {
    if cfg!(windows) {
        // Windows：官方安装器（~\.local\bin\claude.exe）、npm 全局（%APPDATA%\npm）、
        // Volta、Bun 与 Scoop 的命令目录
        let appdata = std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home_dir.join("AppData").join("Roaming"));
        let local = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home_dir.join("AppData").join("Local"));
        return vec![
            home_dir.join(".local").join("bin"),
            // Codex 官方 install.ps1 的入口
            local
                .join("Programs")
                .join("OpenAI")
                .join("Codex")
                .join("bin"),
            appdata.join("npm"),
            local.join("Volta").join("bin"),
            home_dir.join(".bun").join("bin"),
            home_dir.join("scoop").join("shims"),
        ];
    }
    vec![
        home_dir.join(".local").join("bin"),
        home_dir.join(".npm-global").join("bin"),
        home_dir.join(".volta").join("bin"),
        home_dir.join(".bun").join("bin"),
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
        // Linux 上的 Homebrew
        PathBuf::from("/home/linuxbrew/.linuxbrew/bin"),
    ]
}

/// 执行已解析出的命令时用的 PATH：命令所在目录放最前，再接继承的 PATH。npm 装的助手是
/// node 脚本，node 一般与它在同一目录；桌面应用继承的 PATH 里没有这个目录。
pub(crate) fn path_for_command(bin_path: &Path) -> std::ffi::OsString {
    let mut dirs: Vec<PathBuf> = bin_path
        .parent()
        .map(Path::to_path_buf)
        .into_iter()
        .collect();
    if let Some(paths) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&paths));
    }
    std::env::join_paths(dirs).unwrap_or_default()
}

/// 解析 bin：先 PATH，再 `fallback_bin_dirs`（home_dir 注入，测试替身）。
pub(crate) fn resolve_bin(home_dir: &Path, bin: &str) -> Option<PathBuf> {
    if bin.is_empty() {
        return None;
    }
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).collect())
        .unwrap_or_default();
    dirs.extend(fallback_bin_dirs(home_dir));
    // Windows 上按 PATHEXT 补扩展名（claude → claude.cmd / claude.exe）
    dirs.into_iter()
        .flat_map(|dir| crate::platform::command_candidates(&dir, bin))
        .find(|p| p.is_file())
}

struct CmdOutput {
    exit_code: i32,
}

#[cfg(unix)]
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

#[cfg(unix)]
/// 等待子进程并并发有界读取输出；读取端 recv_timeout 兜底（孙进程持管道不阻塞主流程）。
fn wait_bounded(
    mut child: std::process::Child,
    group: crate::process::Group,
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
            group.kill(&mut child);
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

/// 经 sh -c 执行（POSIX），HOME 注入 home_dir。非 unix 平台明确拒绝而非编译失败。
#[cfg(unix)]
fn run_shell(home_dir: &Path, cmd: &str, timeout: Duration, action: &str) -> Result<CmdOutput> {
    let mut command = crate::process::command("sh");
    command
        .arg("-c")
        .arg(cmd)
        .env("HOME", home_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // 独立进程组：超时时连同安装器拉起的进程一起结束
    let (child, group) = crate::process::spawn_group(&mut command)
        .map_err(|e| anyhow!("无法启动 Agent 动作（{}）: {}", action, e))?;
    wait_bounded(child, group, timeout, action)
}

#[cfg(not(unix))]
fn run_shell(_home_dir: &Path, _cmd: &str, _timeout: Duration, action: &str) -> Result<CmdOutput> {
    bail!("agents 域动作 '{}' 在当前平台不可执行", action)
}

/// status 的版本探测：只对已解析出的实际二进制执行固定 `--version`，不执行策略中的 shell。
///
/// 版本号与配置目录无关，但官方 CLI 启动时会写配置目录（`codex` 每次都在 `CODEX_HOME/tmp` 建临时
/// 目录）；查询不能改动用户目录，所以把两个配置目录变量指到系统临时目录里的专用位置。
pub(crate) fn probe_version(bin_path: &Path) -> Option<String> {
    let scratch = std::env::temp_dir().join("agent-manager-version-probe");
    let _ = std::fs::create_dir_all(&scratch);
    let mut command = crate::process::command(bin_path);
    command
        .arg("--version")
        .env("PATH", path_for_command(bin_path))
        .env("CODEX_HOME", &scratch)
        .env("CLAUDE_CONFIG_DIR", &scratch)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let (mut child, group) = crate::process::spawn_group(&mut command).ok()?;
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
            group.kill(&mut child);
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

/// 来源识别：Homebrew 按解析后的真实路径判定且优先最高——launcher 位于 ~/.local/bin
/// 也不豁免（符号链接指向 Cellar 仍是 brew）；其余 native 以 launcher（PATH /
/// ~/.local/bin 解析到的入口）所在目录判定——官方 native 安装器在 ~/.local/bin 放置的
/// 入口常是指向 ~/.local/share、~/.codex 等版本目录的符号链接，先整体 canonicalize 比
/// 目录会误判 system；普通路径兜底 system，fail-closed。
pub(crate) fn detected_provider(home_dir: &Path, bin_path: &Path) -> &'static str {
    let canonical = bin_path.canon().unwrap_or_else(|_| bin_path.to_path_buf());
    let path = canonical.to_string_lossy();
    if canonical
        .components()
        .any(|component| component.as_os_str() == "Cellar")
        || path.starts_with("/opt/homebrew/")
        || path.contains("/homebrew/")
    {
        return "brew";
    }
    let home = home_dir.canon().unwrap_or_else(|_| home_dir.to_path_buf());
    // Codex 官方独立安装器把版本放在 ~/.codex/packages/standalone 下；Windows 的 install.ps1
    // 经 %LOCALAPPDATA%\Programs\OpenAI\Codex\bin 的目录联接到达，入口不在 ~/.local/bin
    if canonical.starts_with(home.join(".codex").join("packages").join("standalone")) {
        return "native";
    }
    let native_dir = home.join(".local").join("bin");
    // 只 canonicalize launcher 的父目录，保留最终符号链接不展开
    let launcher_dir = bin_path
        .parent()
        .map(|dir| dir.canon().unwrap_or_else(|_| dir.to_path_buf()));
    if launcher_dir
        .as_deref()
        .is_some_and(|dir| dir.starts_with(&native_dir))
    {
        return "native";
    }
    "system"
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

fn status_element(
    ctx: &Context,
    def: &AgentDef,
    probes: &std::collections::BTreeMap<String, crate::native_upstream::ProbeRecord>,
) -> Value {
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
    // Keep the response shape without collecting quota or opening sessions.
    observed.insert("usage".into(), Value::Null);
    observed.insert("command_sources".into(), command_sources(&def.commands));
    // 实际检测到的安装来源：一个值说清「它是怎么装的」。是否可管由 state 表达
    // （managed 之外 → unmanaged-detected），不再另发分类桶与首选对照。
    observed.insert(
        "install_source".into(),
        actual_provider
            .map(|provider| Value::String(provider.to_string()))
            .unwrap_or(Value::Null),
    );
    observed.insert(
        "management".into(),
        crate::native_agent_adapters::management_for_bin(&def.bin),
    );
    // 实例（带 profile 段的定义）的登录状态与实际配置在它自己的配置目录里，由 profiles:status
    // 报告；这里读主目录会把主实例的值错当成实例的
    let is_instance = def.spec.get("profile").is_some();
    if let Some(kind) =
        crate::native_agent_adapters::kind_for_bin(&def.bin).filter(|_| !is_instance)
    {
        // 会压过主配置里官方账号的外部设置：只报来源与变量名，只读
        let auth_kind = match kind {
            crate::native_agent_adapters::AdapterKind::Claude => {
                crate::native_auth::AuthKind::Claude
            }
            crate::native_agent_adapters::AdapterKind::Codex => crate::native_auth::AuthKind::Codex,
        };
        observed.insert(
            "credential_overrides".into(),
            Value::Array(crate::native_auth::credential_overrides(
                &ctx.home_dir,
                auth_kind,
                &|name| std::env::var_os(name).is_some_and(|value| !value.is_empty()),
            )),
        );
        // 实际值、写入基准与是否官方账号；只读零写
        let current_config = crate::native_agent_settings::current_config_status(
            ctx,
            &def.id,
            def.config_path(),
            kind,
        );
        observed.insert("current_config".into(), current_config);
    }

    let element_id = format!("agents:{}", def.id);
    // 上游轴：仅 enabled 且首选 provider install 命令可解析出 npm 包的 agent 参与
    // 探测宇宙（与 refresh-upstream 收集逻辑共用同一分类器）；其余恒 not-applicable，
    // current 仍携带本地版本用于展示。
    let freshness = if crate::native_upstream::agent_probe_package(def).is_some() {
        crate::native_upstream::freshness_from_cache(probes, &element_id, version.as_deref())
    } else {
        json!({"state": "not-applicable", "current": version.clone(), "upstream": null, "checked_at": null, "probe": "native", "cache_age_sec": null})
    };

    json!({
        "id": element_id,
        "domain": "agents",
        "spec": def.spec.clone(),
        "policy": {"enabled": def.enabled},
        "observed": Value::Object(observed),
        "state": state,
        "freshness": freshness,
        "actions": actions,
    })
}

// ---------- apply ----------

struct ActionRequest {
    agent_id: String,
    action: String,
    offline: bool,
}

fn parse_action_args(req: &Request) -> Result<ActionRequest> {
    if req.flag("--dry-run") {
        bail!("agents:apply 已退役 --dry-run，拒绝该参数（执行前请以 agents:status 确认状态）");
    }
    let only = req
        .value("--only")
        .ok_or_else(|| anyhow!("agents:apply 需要显式 --only agents:<id>"))?;
    let agent_id = only
        .strip_prefix("agents:")
        .ok_or_else(|| anyhow!("--only 只接受 agents:<id>，得到 '{}'", only))?
        .to_string();
    let action = req
        .value("--action")
        .ok_or_else(|| {
            anyhow!("agents:apply 需要显式 --action install|upgrade|uninstall（不自动安装）")
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
            upstream_npm: def.upstream_npm.clone(),
        },
    ))
}

/// Apply preflight is read-only and runs before the process lock is created.
/// 通过完整安全预检（含 --dry-run 拒绝）后 apply 一律真实执行，调用方恒取全局锁。
pub fn preflight_apply(ctx: &Context, req: &Request) -> Result<()> {
    prepare_apply(ctx, req)?;
    Ok(())
}

fn find_def<'a>(defs: &'a [AgentDef], agent_id: &str) -> Result<&'a AgentDef> {
    defs.iter()
        .find(|def| def.id == agent_id)
        .ok_or_else(|| anyhow!("元素 'agents:{}' 在域 'agents' 中不存在", agent_id))
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
            let probes = crate::native_upstream::load_probe_cache(ctx)?;
            let elements: Vec<Value> = defs
                .iter()
                .map(|def| status_element(ctx, def, &probes))
                .collect();
            Ok(json!({"domain": "agents", "elements": elements}))
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
                "steps": [{"name": step_name, "ok": true, "exit_code": output.exit_code}],
                "skipped": [],
            }))
        }
        other => bail!("native_agents 不支持命令 '{}'", other),
    }
}

// ---------- discover（只读，供 system:discover 聚合） ----------

/// 产品只支持 Claude Code 与 Codex。两者通常已有内置定义；保留在候选表中，
/// 兼容旧数据库里定义被用户移除后重新发现的场景。
const KNOWN_AGENT_BINS: &[(&str, &str, &str)] = &[
    ("claude", "claude-code", "Claude Code"),
    ("codex", "codex", "Codex"),
];

/// 从全局 bin 链接反查 npm 包名：`.../lib/node_modules/<pkg>/bin/x`（scope 包取两段）。
/// 解析失败或路径不含 node_modules 即 None，不猜测。
fn npm_package_of(bin_path: &Path) -> Option<String> {
    let resolved = bin_path.canon().ok()?;
    let comps: Vec<String> = resolved
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect();
    let idx = comps.iter().rposition(|c| c == "node_modules")?;
    let first = comps.get(idx + 1)?;
    if first.starts_with('@') {
        Some(format!("{}/{}", first, comps.get(idx + 2)?))
    } else {
        Some(first.clone())
    }
}

fn tildify(path: &Path, home_dir: &Path) -> String {
    if let Ok(rel) = path.strip_prefix(home_dir) {
        return format!("~/{}", crate::platform::slash(&rel));
    }
    // 链接目标经 canonicalize 后可能落在 home 的真实路径上（如 macOS /var → /private/var），
    // 两种写法都归一到 ~，避免输出里混入解析后的绝对路径。
    if let Ok(canonical_home) = home_dir.canon() {
        if let Ok(rel) = path.strip_prefix(&canonical_home) {
            return format!("~/{}", crate::platform::slash(&rel));
        }
    }
    path.to_string_lossy().to_string()
}

/// 只读发现本机已安装、但 agents.definitions 里还没有的助手 CLI。
/// 零写：只做 PATH/`~/.local/bin` 解析与既有 `--version` 探测（有界超时），不落库、不联网。
pub(crate) fn discover(ctx: &Context) -> Result<Value> {
    let policy = store::effective_policy(ctx)?;
    let mut known_ids: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut known_bins: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    if let Some(defs) = policy
        .get("agents")
        .and_then(|v| v.get("definitions"))
        .and_then(Value::as_object)
    {
        for (id, raw) in defs {
            if raw.get("_delete") == Some(&Value::Bool(true)) {
                continue;
            }
            known_ids.insert(id.clone());
            if let Some(bin) = raw.get("bin").and_then(Value::as_str) {
                known_bins.insert(bin.to_string());
            }
        }
    }

    let mut out = Vec::new();
    for (bin, id, display) in KNOWN_AGENT_BINS {
        if known_ids.contains(*id) || known_bins.contains(*bin) {
            continue;
        }
        let Some(bin_path) = resolve_bin(&ctx.home_dir, bin) else {
            continue;
        };
        let npm_package = npm_package_of(&bin_path);
        // node_modules 命中优先于路径归类：Homebrew 的 node 装的全局包仍是 npm 包。
        let origin = match npm_package {
            Some(_) => "npm",
            None => detected_provider(&ctx.home_dir, &bin_path),
        };
        let version = probe_version(&bin_path);
        let mut providers = Map::new();
        match (origin, npm_package.as_deref()) {
            ("npm", Some(pkg)) => {
                providers.insert(
                    "npm".into(),
                    json!({
                        "managed": true,
                        "install": format!("npm install -g {pkg}"),
                        "upgrade": format!("npm install -g {pkg}"),
                        "uninstall": format!("npm uninstall -g {pkg}"),
                    }),
                );
            }
            ("native", _) => {
                providers.insert(
                    "native".into(),
                    json!({"managed": false, "detect_paths": [tildify(&bin_path, &ctx.home_dir)]}),
                );
            }
            ("brew", _) => {
                providers.insert("brew".into(), json!({"managed": false}));
            }
            _ => {
                providers.insert("system".into(), json!({"managed": false}));
            }
        }
        let managed_after_takeover = origin == "npm";
        out.push(json!({
            "id": id,
            "display_name": display,
            "bin": bin,
            "bin_path": tildify(&bin_path, &ctx.home_dir),
            "version": version.clone().map(Value::String).unwrap_or(Value::Null),
            "origin": origin,
            "npm_package": npm_package.clone().map(Value::String).unwrap_or(Value::Null),
            "manage": "register",
            "managed_after_takeover": managed_after_takeover,
            "suggested_definition": json!({
                "display_name": display,
                "bin": bin,
                "preferred_provider": origin,
                "providers": Value::Object(providers),
            }),
        }));
    }
    Ok(Value::Array(out))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// Codex 独立安装：入口在 ~/.local/bin 以外（Windows 的 Programs\OpenAI\Codex\bin），
    /// 经链接到达 ~/.codex/packages/standalone 下的版本，也是官方安装。
    #[cfg(unix)]
    #[test]
    fn codex_standalone_install_is_native_wherever_its_entry_sits() {
        let root = std::env::temp_dir().join(format!("am-provider-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let home = root.join("home");
        let release = home.join(".codex/packages/standalone/releases/0.161.0/bin");
        std::fs::create_dir_all(&release).unwrap();
        std::fs::write(release.join("codex"), "").unwrap();
        let entry_dir = root.join("Programs/OpenAI/Codex/bin");
        std::fs::create_dir_all(&entry_dir).unwrap();
        std::os::unix::fs::symlink(release.join("codex"), entry_dir.join("codex")).unwrap();
        assert_eq!(detected_provider(&home, &entry_dir.join("codex")), "native");
        // 普通目录里的同名程序仍是 system
        let other = root.join("usr/bin");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("codex"), "").unwrap();
        assert_eq!(detected_provider(&home, &other.join("codex")), "system");
        // 别的用户主目录里的独立安装不算这个账号的官方安装
        let elsewhere = root.join("other-home");
        assert_eq!(
            detected_provider(&elsewhere, &entry_dir.join("codex")),
            "system"
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
