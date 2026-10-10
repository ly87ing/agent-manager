//! Native dispatcher: one entry point shared by the CLI and the Tauri bridge.
//!
//! Domain ownership: core (this module) implements policy/db/runs. Domain
//! modules are delivered by their owners as `crate::native_<domain>::handle`
//! with signature `fn handle(&Context, &Request) -> anyhow::Result<Value>`;
//! until registered they answer DOMAIN_NOT_IMPLEMENTED (never fake success).

use std::fs::OpenOptions;
use std::path::PathBuf;

use anyhow::Result;
use fs2::FileExt;
use rusqlite::TransactionBehavior;
use serde_json::{json, Map, Value};

use crate::envelope::{diagnostic, Diagnostic, Envelope, Lock, Mode};
use crate::store;

/// Frozen fields (design review): data_dir = app data dir (native-manager.db
/// lives here); home_dir = user home for domain targets.
pub struct Context {
    pub data_dir: PathBuf,
    pub home_dir: PathBuf,
}

/// Frozen fields: command is "<domain>:<verb>" (a single '.' is also accepted),
/// args is the raw flag list (["--only", "rules:x", "--offline", ...]),
/// run_id is provided by the caller (GUI run id or CLI-generated).
pub struct Request {
    pub command: String,
    pub args: Vec<String>,
    pub run_id: String,
}

impl Request {
    pub fn flag(&self, name: &str) -> bool {
        self.args.iter().any(|arg| arg == name)
    }

    pub fn value(&self, name: &str) -> Option<&str> {
        let mut iter = self.args.iter();
        while let Some(arg) = iter.next() {
            if arg == name {
                return iter.next().map(String::as_str);
            }
        }
        None
    }

    pub fn values(&self, name: &str) -> Vec<&str> {
        let mut out = Vec::new();
        let mut iter = self.args.iter();
        while let Some(arg) = iter.next() {
            if arg == name {
                if let Some(value) = iter.next() {
                    out.push(value.as_str());
                }
            }
        }
        out
    }
}

const KNOWN_DOMAINS: &[&str] = &[
    "usage", "agents", "mcp", "skills", "rules", "aliases", "profiles", "secrets", "profile",
    "programs", "policy", "system", "runs", "db", "logs", "config", "accounts",
];

fn mode_of(request: &Request) -> Mode {
    Mode {
        offline: request.flag("--offline"),
    }
}

fn lock_from_env() -> Lock {
    if std::env::var("AGENT_MANAGER_LOCK_HELD").ok().as_deref() == Some("true") {
        let pid = std::env::var("AGENT_MANAGER_LOCK_PID")
            .ok()
            .and_then(|raw| raw.parse::<i64>().ok())
            .unwrap_or_else(|| std::process::id() as i64);
        Lock {
            held: true,
            pid: Some(pid),
        }
    } else {
        Lock {
            held: false,
            pid: None,
        }
    }
}

struct ApplyLock {
    _file: std::fs::File,
}

/// 锁被占用时最多再等这么久才判定为并发。
///
/// 同一进程里别的线程启动子进程时，子进程在 exec 之前握着这把锁所在文件描述符的副本，
/// 刚释放的锁会被它短暂占住（flock 挂在打开的文件描述上）。这个窗口很短，等它过去即可；
/// 真正在执行的写动作远比这长，照样拒绝。
const APPLY_LOCK_GRACE: std::time::Duration = std::time::Duration::from_millis(500);

fn acquire_apply_lock(ctx: &Context, domain: &str) -> Result<ApplyLock> {
    std::fs::create_dir_all(&ctx.data_dir)?;
    let path = ctx.data_dir.join("agent-manager.apply.lock");
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(path)?;
    let deadline = std::time::Instant::now() + APPLY_LOCK_GRACE;
    loop {
        match file.try_lock_exclusive() {
            Ok(()) => return Ok(ApplyLock { _file: file }),
            Err(err)
                if err.raw_os_error() == fs2::lock_contended_error().raw_os_error()
                    && std::time::Instant::now() < deadline =>
            {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Err(err) => {
                return Err(anyhow::anyhow!(
                    "{domain} apply 正在执行，拒绝并发动作：{err}"
                ))
            }
        }
    }
}

/// The single native entry point. Pure with respect to process state: no exit
/// codes here (CLI maps errors to exit codes); status never writes the DB.
pub fn execute(ctx: &Context, request: &Request) -> Envelope {
    // 每次执行重新读取本机保存的凭据取值。必须在这里、而不是在 dispatch 里：
    // apply 的 preflight 在取锁前就跑了，比 dispatch 更早要用到取值。
    // 桌面端一个进程会连续执行很多次命令，用户刚填的值要当场生效。
    crate::secret_values::refresh(ctx);
    // system:audit 叶子特判：DispatchResult 无法表达“有效 data + errors”，
    // 且 audit 永不取 apply lock、永不写库，直接在叶子路径组装 envelope。
    if request.command.replace('.', ":") == "system:audit" {
        let outcome = crate::native_system::run(ctx, request);
        let sanitize = |items: Vec<Diagnostic>| -> Vec<Diagnostic> {
            items
                .into_iter()
                .map(|mut d| {
                    d.message = crate::sanitize::sanitize_diagnostic_message(&d.message);
                    d
                })
                .collect()
        };
        let envelope = Envelope::new(
            request.command.replacen(':', ".", 1),
            mode_of(request),
            lock_from_env(),
            request.run_id.clone(),
            outcome.data,
            sanitize(outcome.warnings),
            sanitize(outcome.errors),
        );
        return envelope;
    }
    let is_agent_apply = request.command.replace('.', ":") == "agents:apply";
    // 纳入管理会移动账号目录里的内容并建链接：与 skills:apply 同样取排他锁
    let is_skill_apply = matches!(
        request.command.replace('.', ":").as_str(),
        "skills:apply" | "skills:adopt"
    );
    let is_mcp_apply = request.command.replace('.', ":") == "mcp:apply";
    let is_rule_apply = request.command.replace('.', ":") == "rules:apply";
    let is_alias_apply = request.command.replace('.', ":") == "aliases:apply";
    let is_profile_apply = matches!(
        request.command.replace('.', ":").as_str(),
        "profiles:apply" | "profiles:apply-definition" | "profiles:create"
    );
    let is_profile_seed = request.command.replace('.', ":") == "profiles:seed";
    let is_profile_shared_apply = request.command.replace('.', ":") == "profiles:shared-apply";
    // 运行参数写助手配置文件，与别的写动作同样要排他；校验在 configure 内部做，
    // 这里只负责取锁。
    let is_agent_configure = matches!(
        request.command.replace('.', ":").as_str(),
        "agents:configure"
    );
    // 程序：安装、升级、回退、纳入会动数据目录与用户的全局安装，取排他锁
    let is_program_apply = request.command.replace('.', ":") == "programs:apply";
    let is_secret_write = matches!(
        request.command.replace('.', ":").as_str(),
        "secrets:set" | "secrets:source" | "secrets:migrate-legacy"
    );
    let (needs_lock, domain_label, diag_code) = if is_agent_apply {
        match crate::native_agents::preflight_apply(ctx, request) {
            Ok(()) => (true, "Agent", "AGENT_ERROR"),
            Err(err) => {
                let env = Envelope::new(
                    request.command.replacen(':', ".", 1),
                    mode_of(request),
                    Lock {
                        held: false,
                        pid: None,
                    },
                    request.run_id.clone(),
                    Value::Object(Map::new()),
                    vec![],
                    vec![diagnostic(
                        "AGENT_ERROR",
                        None,
                        crate::sanitize::sanitize_diagnostic_message(&format!("{err:#}")),
                    )],
                );
                return env;
            }
        }
    } else if is_skill_apply {
        match crate::native_skills::preflight_apply(ctx, request) {
            Ok(()) => (true, "Skill", "SKILL_ERROR"),
            Err(err) => {
                let env = Envelope::new(
                    request.command.replacen(':', ".", 1),
                    mode_of(request),
                    Lock {
                        held: false,
                        pid: None,
                    },
                    request.run_id.clone(),
                    Value::Object(Map::new()),
                    vec![],
                    vec![diagnostic(
                        "SKILL_ERROR",
                        None,
                        crate::sanitize::sanitize_diagnostic_message(&format!("{err:#}")),
                    )],
                );
                return env;
            }
        }
    } else if is_mcp_apply {
        match crate::native_mcp::preflight_apply(ctx, request) {
            Ok(()) => (true, "MCP", "MCP_ERROR"),
            Err(err) => {
                let env = Envelope::new(
                    request.command.replacen(':', ".", 1),
                    mode_of(request),
                    Lock {
                        held: false,
                        pid: None,
                    },
                    request.run_id.clone(),
                    Value::Object(Map::new()),
                    vec![],
                    vec![diagnostic(
                        "MCP_ERROR",
                        None,
                        crate::sanitize::sanitize_diagnostic_message(&format!("{err:#}")),
                    )],
                );
                return env;
            }
        }
    } else if is_rule_apply {
        match crate::native_rules::preflight_apply(ctx, request) {
            Ok(()) => (true, "Rules", "RULE_ERROR"),
            Err(err) => {
                let env = Envelope::new(
                    request.command.replacen(':', ".", 1),
                    mode_of(request),
                    Lock {
                        held: false,
                        pid: None,
                    },
                    request.run_id.clone(),
                    Value::Object(Map::new()),
                    vec![],
                    vec![diagnostic(
                        "RULE_ERROR",
                        None,
                        crate::sanitize::sanitize_diagnostic_message(&format!("{err:#}")),
                    )],
                );
                return env;
            }
        }
    } else if is_alias_apply {
        match crate::native_aliases::preflight_apply(ctx, request) {
            Ok(()) => (true, "Aliases", "ALIAS_ERROR"),
            Err(err) => {
                let env = Envelope::new(
                    request.command.replacen(':', ".", 1),
                    mode_of(request),
                    Lock {
                        held: false,
                        pid: None,
                    },
                    request.run_id.clone(),
                    Value::Object(Map::new()),
                    vec![],
                    vec![diagnostic(
                        crate::native_aliases::error_code(&err),
                        None,
                        crate::sanitize::sanitize_diagnostic_message(&format!("{err:#}")),
                    )],
                );
                return env;
            }
        }
    } else if is_profile_apply {
        match crate::native_profiles::preflight_apply(ctx, request) {
            Ok(()) => (true, "Profiles", "PROFILE_ERROR"),
            Err(err) => {
                let env = Envelope::new(
                    request.command.replacen(':', ".", 1),
                    mode_of(request),
                    Lock {
                        held: false,
                        pid: None,
                    },
                    request.run_id.clone(),
                    Value::Object(Map::new()),
                    vec![],
                    vec![diagnostic(
                        "PROFILE_ERROR",
                        None,
                        crate::sanitize::sanitize_diagnostic_message(&format!("{err:#}")),
                    )],
                );
                return env;
            }
        }
    } else if is_profile_seed || is_profile_shared_apply {
        let preflight = if is_profile_seed {
            crate::native_profiles::preflight_seed(ctx, request)
        } else {
            crate::native_profiles::preflight_shared(ctx, request)
        };
        match preflight {
            Ok(()) => (true, "Profiles", "PROFILE_ERROR"),
            Err(err) => {
                let env = Envelope::new(
                    request.command.replacen(':', ".", 1),
                    mode_of(request),
                    Lock {
                        held: false,
                        pid: None,
                    },
                    request.run_id.clone(),
                    Value::Object(Map::new()),
                    vec![],
                    vec![diagnostic(
                        "PROFILE_ERROR",
                        None,
                        crate::sanitize::sanitize_diagnostic_message(&format!("{err:#}")),
                    )],
                );
                return env;
            }
        }
    } else if is_agent_configure {
        (true, "Agent", "AGENT_ERROR")
    } else if is_secret_write {
        (true, "Secrets", "SECRET_ERROR")
    } else if is_program_apply {
        (true, "Programs", "PROGRAM_ERROR")
    } else {
        (false, "", "")
    };
    let apply_lock = if needs_lock {
        match acquire_apply_lock(ctx, domain_label) {
            Ok(lock) => Some(lock),
            Err(err) => {
                let env = Envelope::new(
                    request.command.replacen(':', ".", 1),
                    mode_of(request),
                    Lock {
                        held: false,
                        pid: None,
                    },
                    request.run_id.clone(),
                    Value::Object(Map::new()),
                    vec![],
                    vec![diagnostic(
                        diag_code,
                        None,
                        crate::sanitize::sanitize_diagnostic_message(&format!("{err:#}")),
                    )],
                );
                return env;
            }
        }
    } else {
        None
    };
    let lock = if apply_lock.is_some() {
        Lock {
            held: true,
            pid: Some(std::process::id() as i64),
        }
    } else {
        lock_from_env()
    };
    let envelope = match dispatch(ctx, request) {
        Ok((data, warnings)) => {
            let sanitized_warnings = warnings
                .into_iter()
                .map(|mut w| {
                    w.message = crate::sanitize::sanitize_diagnostic_message(&w.message);
                    w
                })
                .collect();
            Envelope::new(
                request.command.replacen(':', ".", 1),
                mode_of(request),
                lock,
                request.run_id.clone(),
                data,
                sanitized_warnings,
                vec![],
            )
        }
        Err(DispatchError { code, message }) => Envelope::new(
            request.command.replacen(':', ".", 1),
            mode_of(request),
            lock,
            request.run_id.clone(),
            Value::Object(Map::new()),
            vec![],
            vec![diagnostic(
                code,
                None,
                crate::sanitize::sanitize_diagnostic_message(&message),
            )],
        ),
    };
    envelope
}

pub(crate) struct DispatchError {
    pub(crate) code: &'static str,
    pub(crate) message: String,
}

impl DispatchError {
    pub(crate) fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl From<anyhow::Error> for DispatchError {
    fn from(err: anyhow::Error) -> Self {
        DispatchError::new("INTERNAL", format!("{err:#}"))
    }
}

impl From<rusqlite::Error> for DispatchError {
    fn from(err: rusqlite::Error) -> Self {
        DispatchError::new("INTERNAL", err.to_string())
    }
}

pub(crate) type DispatchResult = Result<(Value, Vec<Diagnostic>), DispatchError>;

fn dispatch(ctx: &Context, request: &Request) -> DispatchResult {
    let dotted = request.command.replacen(':', ".", 1);
    let mut parts = dotted.splitn(2, '.');
    let domain = parts.next().unwrap_or("");
    let verb = parts.next().unwrap_or("");
    if domain.is_empty() || verb.is_empty() {
        return Err(DispatchError::new(
            "USAGE_ERROR",
            format!("命令 '{}' 不是 <domain>:<verb> 形式", request.command),
        ));
    }
    if !KNOWN_DOMAINS.contains(&domain) {
        return Err(DispatchError::new(
            "UNKNOWN_DOMAIN",
            format!(
                "未知域 '{}'（已知域: {}）",
                domain,
                KNOWN_DOMAINS.join(", ")
            ),
        ));
    }
    match (domain, verb) {
        ("accounts", "command") => crate::native_account_context::command_request(ctx, request)
            .map(|data| (data, vec![]))
            .map_err(|err| DispatchError::new("ACCOUNT_ISOLATION_ERROR", format!("{err:#}"))),
        ("agents", "status" | "apply") => crate::native_agents::handle(ctx, request)
            .map(|data| (data, vec![]))
            .map_err(|err| DispatchError::new("AGENT_ERROR", format!("{err:#}"))),
        // 用量：status 只读已聚合的结果，scan 才去读会话文件并写库
        ("usage", "status" | "scan" | "clear") => {
            crate::native_usage::handle(ctx, request).map(|d| (d, vec![]))
        }
        // 最近会话：只读会话文件开头，不写库
        ("usage", "sessions") => {
            crate::native_sessions::sessions(ctx, request).map(|d| (d, vec![]))
        }
        // 运行参数落到助手自己的配置文件；与装卸升级分开，那条走 shell 命令，这条写文件
        ("agents", "configure") => {
            crate::native_agent_settings::apply(ctx, request).map(|data| (data, vec![]))
        }
        ("agents", "preview-runtime") => {
            crate::native_agent_settings::preview_runtime(ctx, request).map(|data| (data, vec![]))
        }
        // 官方账号登录状态：用官方命令查一次，只读零写；一次要一两秒，只在界面上点它时才调
        ("agents", "auth-status") => crate::native_auth::status_command(ctx, request)
            .map(|data| (data, vec![]))
            .map_err(|err| DispatchError::new("AGENT_ERROR", format!("{err:#}"))),
        (
            "skills",
            "status" | "apply" | "fork" | "versions" | "preview" | "adopt-preview" | "adopt",
        ) => crate::native_skills::handle(ctx, request)
            .map(|data| (data, vec![]))
            .map_err(|err| DispatchError::new("SKILL_ERROR", format!("{err:#}"))),
        ("mcp", "status" | "preview" | "apply" | "check") => {
            crate::native_mcp::dispatch(ctx, request)
                .map(|(data, notes)| {
                    let warnings = notes
                        .into_iter()
                        .map(|note| diagnostic("MCP_NOTICE", None, note))
                        .collect();
                    (data, warnings)
                })
                .map_err(|err| DispatchError::new("MCP_ERROR", format!("{err:#}")))
        }
        // 规则以实际文件为准：apply 写文件（取锁），模板保存/删除只写库
        ("rules", "status" | "apply" | "save-template" | "delete-template") => {
            crate::native_rules::handle(ctx, request)
                .map(|data| (data, vec![]))
                .map_err(|err| DispatchError::new("RULE_ERROR", format!("{err:#}")))
        }
        (
            "profiles",
            "status" | "preview" | "apply" | "apply-definition" | "create" | "adopt" | "seed"
            | "shared-status" | "shared-apply",
        ) => crate::native_profiles::handle(ctx, request)
            .map(|data| (data, vec![]))
            .map_err(|err| DispatchError::new("PROFILE_ERROR", format!("{err:#}"))),
        // 别名以 rc 文件受管区块的实际行为准：apply 写文件（取锁），丢弃旧版保存的别名只写库
        ("aliases", "status" | "preview" | "apply" | "discard-legacy") => {
            crate::native_aliases::handle(ctx, request)
                .map(|data| (data, vec![]))
                .map_err(|err| {
                    DispatchError::new(crate::native_aliases::error_code(&err), format!("{err:#}"))
                })
        }
        ("secrets", "status" | "set" | "adopt" | "source" | "migrate-legacy") => {
            crate::native_secrets::handle(ctx, request)
                .map(|data| (data, vec![]))
                .map_err(|err| DispatchError::new("SECRET_ERROR", format!("{err:#}")))
        }
        // 程序：技能与 MCP 依赖的程序（智能体本身不管）；status 只读零写零网络
        ("programs", "status" | "preview" | "apply" | "declare") => {
            crate::native_programs::handle(ctx, request)
                .map(|data| (data, vec![]))
                .map_err(|err| DispatchError::new("PROGRAM_ERROR", format!("{err:#}")))
        }
        ("policy", "get") => policy_get(ctx),
        ("policy", "set") => policy_set(ctx, request),
        ("policy", "delete") => policy_delete(ctx, request),
        ("db", "status") => db_status(ctx),
        ("db", "migrate") => db_migrate(ctx, request),
        ("runs", "list") => crate::native_runs::runs_list(ctx),
        ("runs", "get") => crate::native_runs::runs_get(ctx, request),
        // 长操作的进度：只读系统临时目录里的进度文件，没有进行中的操作时为 null
        ("runs", "progress") => match request.value("--run") {
            Some(run) => Ok((json!({"progress": crate::progress::read(run)}), vec![])),
            None => Err(DispatchError::new(
                "USAGE_ERROR",
                "runs:progress 需要 --run <运行编号>".to_string(),
            )),
        },
        ("runs", "clear") => crate::native_runs::runs_clear(ctx),
        ("config", "export" | "import") => crate::native_config::handle(ctx, request)
            .map(|data| (data, vec![]))
            .map_err(|err| DispatchError::new("CONFIG_ERROR", format!("{err:#}"))),
        ("system", "discover") => crate::native_discover::handle(ctx, request)
            .map(|data| (data, vec![]))
            .map_err(|err| DispatchError::new("DISCOVER_ERROR", format!("{err:#}"))),
        ("system", "refresh-upstream") => crate::native_upstream::handle(ctx, request),
        // 已退役或从未进入命令面的动词：已知域 + 未知动词。
        _ => Err(DispatchError::new(
            "UNKNOWN_VERB",
            format!("未知动词 '{verb}'（域 '{domain}' 的命令面内没有该动词）"),
        )),
    }
}

// ---------------------------------------------------------------------------
// policy domain
// ---------------------------------------------------------------------------

fn validate_key(key: &str) -> Result<Vec<&str>, DispatchError> {
    if key.is_empty() {
        return Err(DispatchError::new("USAGE_ERROR", "非法 key 路径：不能为空"));
    }
    let segments: Vec<&str> = key.split('.').collect();
    let re = regex_lite_match();
    for segment in &segments {
        if segment.is_empty() || !re(segment) {
            return Err(DispatchError::new(
                "USAGE_ERROR",
                format!("非法 key 路径段（点号分隔，每段只允许字母/数字/_/-）: {key}"),
            ));
        }
    }
    Ok(segments)
}

/// 严格参数面校验：args 只允许出现 `recognized` 中的 `--flag <value>` 对；
/// 任何其他参数（已退役的 --dry-run、未知 flag、位置参数、缺值的悬挂
/// flag）一律在任何写之前以 USAGE_ERROR 拒绝。
fn reject_unrecognized_args(
    request: &Request,
    recognized: &[&str],
    command: &str,
) -> Result<(), DispatchError> {
    let mut iter = request.args.iter();
    while let Some(arg) = iter.next() {
        if recognized.contains(&arg.as_str()) {
            if iter.next().is_none() {
                return Err(DispatchError::new(
                    "USAGE_ERROR",
                    format!("{command} 参数 {arg} 缺少值"),
                ));
            }
            continue;
        }
        return Err(DispatchError::new(
            "USAGE_ERROR",
            format!("{command} 不支持参数 '{arg}'"),
        ));
    }
    Ok(())
}

/// 无 regex 依赖的段校验（等价于 ^[A-Za-z0-9_-]+$）。
fn regex_lite_match() -> impl Fn(&str) -> bool {
    |segment: &str| {
        segment
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    }
}

/// Deep-merge `patch` into `target` honoring `_replace: true` (replace the
/// subtree instead of merging) and `_delete: true` (remove the key), matching
/// the overlay semantics the GUI already sends.
pub fn deep_merge_overlay(target: &mut Value, patch: &Value) {
    match (target, patch) {
        (Value::Object(target_map), Value::Object(patch_map)) => {
            if patch_map.get("_replace") == Some(&Value::Bool(true)) {
                let mut replacement = patch_map.clone();
                replacement.remove("_replace");
                *target_map = replacement;
                return;
            }
            for (key, value) in patch_map {
                if key == "_replace" {
                    continue;
                }
                if value
                    == &Value::Object(Map::from_iter([("_delete".to_string(), Value::Bool(true))]))
                {
                    target_map.remove(key);
                    continue;
                }
                deep_merge_overlay(target_map.entry(key.clone()).or_insert(Value::Null), value);
            }
        }
        (target, patch) => *target = patch.clone(),
    }
}

fn set_dot_path(root: &mut Value, segments: &[&str], value: Value) -> Result<(), DispatchError> {
    let mut cursor = root;
    for segment in &segments[..segments.len() - 1] {
        if !cursor.is_object() {
            return Err(DispatchError::new(
                "POLICY_SET_INVALID",
                format!("配置项路径 '{segment}' 的父级不是对象，拒绝写入子字段"),
            ));
        }
        cursor = cursor
            .as_object_mut()
            .expect("checked above")
            .entry(segment.to_string())
            .or_insert_with(|| Value::Object(Map::new()));
    }
    if !cursor.is_object() {
        return Err(DispatchError::new(
            "POLICY_SET_INVALID",
            "配置项不是对象，拒绝写入子字段",
        ));
    }
    let leaf = segments[segments.len() - 1];
    let map = cursor.as_object_mut().expect("checked above");
    match map.get_mut(leaf) {
        Some(existing) => deep_merge_overlay(existing, &value),
        None => {
            // 新键直接落值（_replace/_delete 包裹的新子树按原样存储语义不展开）
            map.insert(leaf.to_string(), value);
        }
    }
    Ok(())
}

fn delete_dot_path(root: &mut Value, segments: &[&str]) -> Result<(), DispatchError> {
    let mut cursor = root;
    for segment in &segments[..segments.len() - 1] {
        let next = cursor.as_object_mut().and_then(|map| map.get_mut(*segment));
        match next {
            Some(value) => cursor = value,
            None => {
                return Err(DispatchError::new(
                    "POLICY_KEY_NOT_FOUND",
                    format!("键不存在: {}", segments.join(".")),
                ))
            }
        }
    }
    let map = cursor.as_object_mut().ok_or_else(|| {
        DispatchError::new(
            "POLICY_KEY_NOT_FOUND",
            format!("键不存在: {}", segments.join(".")),
        )
    })?;
    if map.remove(segments[segments.len() - 1]).is_none() {
        return Err(DispatchError::new(
            "POLICY_KEY_NOT_FOUND",
            format!("键不存在: {}", segments.join(".")),
        ));
    }
    Ok(())
}

/// 地址里内嵌账号密码的那段（`scheme://user:pass@host` 的 `user:pass`）的位置；没有时为 `None`。
fn url_userinfo_end(url: &str) -> Option<(usize, usize)> {
    let start = url.find("://")? + 3;
    let rest = &url[start..];
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let at = rest[..authority_end].rfind('@')?;
    Some((start, start + at))
}

/// 地址里是否内嵌了账号密码。这部分是凭据：不存库、不展示。
pub(crate) fn url_has_userinfo(url: &str) -> bool {
    url_userinfo_end(url).is_some()
}

/// 展示地址时把内嵌的账号密码换成 `***`，其余原样。
pub(crate) fn redact_url_userinfo(url: &str) -> String {
    match url_userinfo_end(url) {
        Some((start, end)) => format!("{}***{}", &url[..start], &url[end..]),
        None => url.to_string(),
    }
}

/// 变量名、请求头或配置键看起来是否承载凭据。
///
/// 取值格式检测（`reject_secret_text`）只认得常见厂商前缀，自定义格式的令牌会漏过；
/// 凡是要落库的环境变量或请求头字面量，名字像凭据就按凭据处理，不看取值长什么样。
///
/// 按整类匹配（参照 CC Switch 的 `is_sensitive_config_key`）：SECRET、PASSWORD、CREDENTIAL
/// 这类词出现在哪都算；TOKEN、KEY 必须是独立的词（按 `_`、`-`、`.` 与驼峰分词），所以
/// `MAX_OUTPUT_TOKENS` 这类长度参数不算；PAT、PASS、PWD、AUTH 只在带前缀的末尾才算——
/// `GITHUB_PAT`、`DB_PASS`、`X-Auth` 算，shell 的 `PWD` 与 `preferred_auth_method` 不算。
pub(crate) fn credential_like_name(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    if [
        "SECRET",
        "PASSWORD",
        "PASSWD",
        "PASSPHRASE",
        "CREDENTIAL",
        "AUTHORIZATION",
        "APIKEY",
        "APITOKEN",
        "ACCESSKEY",
    ]
    .iter()
    .any(|needle| upper.contains(needle))
    {
        return true;
    }
    let words = name_words(name);
    words
        .iter()
        .any(|word| matches!(word.as_str(), "TOKEN" | "KEY" | "KEYS" | "CREDS"))
        || (words.len() > 1
            && matches!(
                words.last().map(String::as_str),
                Some("PAT" | "PASS" | "PWD" | "AUTH")
            ))
}

/// 按 `_`、`-`、`.` 与驼峰边界拆成大写单词：`apiKeyHelper` → API KEY HELPER，`APIKey` → API KEY。
fn name_words(name: &str) -> Vec<String> {
    let chars: Vec<char> = name.chars().collect();
    let mut words = Vec::new();
    let mut word = String::new();
    for (index, &c) in chars.iter().enumerate() {
        if !c.is_ascii_alphanumeric() {
            if !word.is_empty() {
                words.push(std::mem::take(&mut word));
            }
            continue;
        }
        let camel = c.is_ascii_uppercase()
            && index > 0
            && (chars[index - 1].is_ascii_lowercase()
                || chars[index - 1].is_ascii_digit()
                || (chars[index - 1].is_ascii_uppercase()
                    && chars.get(index + 1).is_some_and(|n| n.is_ascii_lowercase())));
        if camel && !word.is_empty() {
            words.push(std::mem::take(&mut word));
        }
        word.push(c.to_ascii_uppercase());
    }
    if !word.is_empty() {
        words.push(word);
    }
    words
}

pub(crate) fn reject_secret_text(text: &str) -> Option<&'static str> {
    use std::sync::OnceLock;
    static RULES: OnceLock<Vec<(&'static str, regex::Regex)>> = OnceLock::new();
    let rules = RULES.get_or_init(|| {
        vec![
            (
                "openai_like_key",
                regex::Regex::new(r"\bsk-(?:proj-|ant-)?[A-Za-z0-9_-]{20,}\b").unwrap(),
            ),
            (
                "github_token",
                regex::Regex::new(
                    r"\b(?:gh[pousr]_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,})\b",
                )
                .unwrap(),
            ),
            (
                "google_api_key",
                regex::Regex::new(r"\bAIza[0-9A-Za-z_-]{20,}\b").unwrap(),
            ),
            (
                "aws_access_key",
                regex::Regex::new(r"\bAKIA[0-9A-Z]{16}\b").unwrap(),
            ),
            (
                "private_key_block",
                regex::Regex::new(r"-----BEGIN (?:RSA |EC |DSA |OPENSSH |PGP )?PRIVATE KEY-----")
                    .unwrap(),
            ),
            (
                "augment_session_token",
                regex::Regex::new(r#""accessToken"\s*:\s*"ace_[A-Za-z0-9_-]{12,}""#).unwrap(),
            ),
        ]
    });
    rules
        .iter()
        .find_map(|(kind, rule)| rule.is_match(text).then_some(*kind))
}

/// 整树凭据校验（配置备份恢复时复用同一道闸门）。
pub(crate) fn reject_secrets_value(value: &Value) -> anyhow::Result<()> {
    reject_secrets("", value).map_err(|err| anyhow::anyhow!("{}", err.message))
}

fn reject_secrets(key: &str, value: &Value) -> Result<(), DispatchError> {
    if let Some(kind) = reject_secret_text(key) {
        return Err(DispatchError::new(
            "SECRET_INPUT_REJECTED",
            format!("拒绝写入疑似凭据: {kind}"),
        ));
    }
    match value {
        Value::String(text) => reject_secret_text(text).map_or(Ok(()), |kind| {
            Err(DispatchError::new(
                "SECRET_INPUT_REJECTED",
                format!("拒绝写入疑似凭据: {kind}"),
            ))
        }),
        Value::Object(map) => {
            for (child, item) in map {
                reject_secrets(child, item)?;
            }
            Ok(())
        }
        Value::Array(items) => {
            for item in items {
                reject_secrets(key, item)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn policy_data(
    layer: Option<&str>,
    key: Option<&str>,
    value: Option<Value>,
    path: Option<String>,
    effective: Option<Value>,
    attribution: Option<Value>,
) -> Value {
    json!({
        "layers": Value::Null,
        "effective": effective,
        "attribution": attribution,
        "layer": layer,
        "key": key,
        "value": value,
        "path": path,
    })
}

fn policy_get(ctx: &Context) -> DispatchResult {
    let db = store::db_path(ctx);
    let exists = db.is_file();
    let effective = store::effective_policy(ctx).map_err(DispatchError::from)?;
    let attribution: Value = effective
        .as_object()
        .map(|map| {
            map.keys()
                .map(|key| {
                    (
                        key.clone(),
                        Value::String(if exists {
                            "sqlite".into()
                        } else {
                            "builtin".into()
                        }),
                    )
                })
                .collect::<Map<String, Value>>()
        })
        .map(Value::Object)
        .unwrap_or(Value::Null);
    let mut layers = Map::new();
    layers.insert(
        "sqlite".into(),
        if exists {
            effective.clone()
        } else {
            Value::Null
        },
    );
    let mut data = policy_data(
        None,
        None,
        None,
        Some(db.display().to_string()),
        Some(effective),
        Some(attribution),
    );
    data.as_object_mut()
        .expect("object")
        .insert("layers".into(), Value::Object(layers));
    Ok((data, vec![]))
}

/// MCP 定义候选校验（policy:set 保存期）。env/headers 占位符与 agents 允许列表
/// 对 merge 后候选策略树中的定义生效；当本次写入的值是完整定义对象时
/// （`mcp.definitions` 整表中的条目，或 `mcp.definitions.<name>` 整条定义），
/// 对 merge 后的完整定义追加完整形态校验（远程 url 白名单、stdio/远程字段互斥，
/// 错误消息不回显 url/正文）。`_delete` 删除型 patch 与更深层子字段 patch
/// （deep-merge 语义，如 mcp.definitions.<name>.startup_timeout_sec）不触发
/// 形态校验，保持既有粒度。
fn validate_mcp_policy_candidate(
    key: &str,
    value: &Value,
    policy: &Value,
) -> Result<(), DispatchError> {
    let prefix = "mcp.definitions";
    if key != prefix && !key.starts_with("mcp.definitions.") {
        return Ok(());
    }
    let Some(mcp) = policy.get("mcp").and_then(Value::as_object) else {
        return Ok(());
    };
    let Some(defs) = mcp.get("definitions").and_then(Value::as_object) else {
        return Ok(());
    };

    fn validate_common(def: &Value) -> Result<(), DispatchError> {
        if let Err(e) = crate::native_mcp::policy_view::validate_definition_env(def) {
            return Err(DispatchError::new("USAGE_ERROR", e.to_string()));
        }
        if let Err(e) = crate::native_mcp::policy_view::validate_definition_headers(def) {
            return Err(DispatchError::new("USAGE_ERROR", e.to_string()));
        }
        if let Err(e) = crate::native_mcp::policy_view::allowed_agents(def) {
            return Err(DispatchError::new("USAGE_ERROR", e.to_string()));
        }
        Ok(())
    }

    fn validate_shape(server: &str, def: &Value) -> Result<(), DispatchError> {
        // _delete 删除型 patch：仅标记删除，不校验形态
        if def.get("_delete").and_then(Value::as_bool) == Some(true) {
            return Ok(());
        }
        if let Err(e) = crate::native_mcp::policy_view::validate_definition(def, server) {
            return Err(DispatchError::new("USAGE_ERROR", e.to_string()));
        }
        Ok(())
    }

    if key == prefix {
        // 整表写入：形态校验只覆盖本次值中的完整定义对象，历史遗留定义不牵连
        if let Value::Object(incoming) = value {
            for name in incoming.keys() {
                if name.starts_with('_') {
                    continue;
                }
                if let Some(def) = defs.get(name) {
                    validate_shape(name, def)?;
                }
            }
        }
        for def in defs.values() {
            validate_common(def)?;
        }
        return Ok(());
    }

    if let Some(rest) = key.strip_prefix("mcp.definitions.") {
        let server_name = rest.split('.').next().unwrap_or("");
        if !server_name.is_empty() {
            if let Some(def) = defs.get(server_name) {
                validate_common(def)?;
                // 仅 mcp.definitions.<name> 整条位置且写入值为对象时跑形态校验；
                // 子字段 patch（更深 key）不触发
                if !rest.contains('.') && value.is_object() {
                    validate_shape(server_name, def)?;
                }
            }
        }
    }
    Ok(())
}

fn check_expected_agent_definition(
    policy: &Value,
    segments: &[&str],
    expected: &Value,
) -> Result<(), DispatchError> {
    let current = segments
        .iter()
        .try_fold(policy, |value, segment| value.get(*segment));
    let visible = match current {
        Some(value) if value.get("_delete") == Some(&Value::Bool(true)) => Value::Null,
        Some(value) => crate::native_agents::strip_internal_markers(value),
        None => Value::Null,
    };
    if &visible != expected {
        return Err(DispatchError::new(
            "POLICY_CONFLICT",
            "Agent 定义在表单打开后发生变化，未保存；请刷新后重试",
        ));
    }
    Ok(())
}

/// Account incarnations are local registration metadata, never credentials. Only
/// an explicit registration/update initializes a legacy profile; queries stay read-only.
pub(crate) fn prepare_account_generations(
    before: &Value,
    after: &mut Value,
    imported: bool,
    explicit_key: Option<&str>,
) -> anyhow::Result<()> {
    let old = before
        .pointer("/agents/definitions")
        .and_then(Value::as_object);
    let Some(definitions) = after
        .pointer_mut("/agents/definitions")
        .and_then(Value::as_object_mut)
    else {
        return Ok(());
    };
    for (id, raw) in definitions {
        let previous = old.and_then(|definitions| definitions.get(id));
        let explicitly_saved = imported
            || explicit_key.is_some_and(|key| {
                key == "agents"
                    || key == "agents.definitions"
                    || key == format!("agents.definitions.{id}")
                    || key.starts_with(&format!("agents.definitions.{id}."))
            });
        let initialize_legacy = explicitly_saved && raw.get("account_generation").is_none();
        let independent = raw.get("profile").is_some()
            || (raw.get("mcp_config").is_some()
                && !matches!(id.as_str(), "claude-code" | "codex" | "opencode"));
        if (previous == Some(raw) && !initialize_legacy)
            || !independent
            || raw.get("_delete") == Some(&Value::Bool(true))
        {
            continue;
        }
        let registered = previous.filter(|previous| {
            (previous.get("profile").is_some() || previous.get("mcp_config").is_some())
                && previous.get("profile").is_some() == raw.get("profile").is_some()
                && previous.get("_delete") != Some(&Value::Bool(true))
        });
        let generation = registered.and_then(|raw| raw.get("account_generation"));
        let inherited_stale = registered.is_none()
            && raw.get("account_generation")
                == previous.and_then(|raw| raw.get("account_generation"));
        if !imported && raw.get("account_generation") != generation && !inherited_stale {
            anyhow::bail!("账号代际由本工具维护，拒绝更改或提交自定义代际；请刷新定义后重试");
        }
        let generation = match generation {
            Some(generation) => generation.clone(),
            None => Value::String(store::new_account_generation()?),
        };
        raw.as_object_mut()
            .ok_or_else(|| anyhow::anyhow!("账号定义必须是对象"))?
            .insert("account_generation".into(), generation);
    }
    Ok(())
}

pub(crate) fn validate_account_policy_change(
    ctx: &Context,
    before: &Value,
    after: &Value,
) -> anyhow::Result<()> {
    let old = before
        .pointer("/agents/definitions")
        .and_then(Value::as_object);
    if let Some(definitions) = after
        .pointer("/agents/definitions")
        .and_then(Value::as_object)
    {
        for (id, raw) in definitions {
            if old.and_then(|definitions| definitions.get(id)) == Some(raw)
                || raw.get("_delete") == Some(&Value::Bool(true))
            {
                continue;
            }
            if let Some(mcp) = raw.get("mcp_config") {
                let path = mcp
                    .as_str()
                    .or_else(|| mcp.get("path").and_then(Value::as_str))
                    .filter(|path| !path.is_empty())
                    .ok_or_else(|| anyhow::anyhow!("账号 MCP 配置路径为空或格式不正确"))?;
                let path = crate::native_mcp::targets::expand_home(path, &ctx.home_dir);
                crate::native_profiles::paths::validate_account_resource_path(
                    ctx, after, id, &path,
                )?;
            }
            if raw.get("profile").is_none() {
                if raw.get("mcp_config").is_some() {
                    let names = crate::native_secrets::account_reference_names(ctx, id, after)?;
                    if !names.is_empty() {
                        let owners =
                            crate::native_secrets::account_secret_owners_in_policy(ctx, after)?;
                        for name in names {
                            if owners
                                .get(&name.to_ascii_uppercase())
                                .is_some_and(|ids| ids.iter().any(|owner| owner != id))
                            {
                                anyhow::bail!(
                                    "账号 MCP 密钥变量属于另一账号或已退役登记，拒绝保存定义"
                                );
                            }
                        }
                    }
                }
                continue;
            }
            let enabled = raw.get("enabled").and_then(Value::as_bool).unwrap_or(true);
            let def = crate::native_profiles::model::parse_definition(id, raw, enabled)?;
            crate::native_profiles::paths::config_dir_in_policy(ctx, &def, after)?;
            crate::native_profiles::model::validate_secret_ownership_in_policy(ctx, &def, after)?;
        }
    }
    Ok(())
}

fn changed_account_ids(before: &Value, after: &Value) -> std::collections::BTreeSet<String> {
    let old = before
        .pointer("/agents/definitions")
        .and_then(Value::as_object);
    let new = after
        .pointer("/agents/definitions")
        .and_then(Value::as_object);
    old.into_iter()
        .flat_map(|defs| defs.keys())
        .chain(new.into_iter().flat_map(|defs| defs.keys()))
        .filter(|id| old.and_then(|defs| defs.get(*id)) != new.and_then(|defs| defs.get(*id)))
        .cloned()
        .collect()
}

/// Reserve the old reference graph before replacing or deleting its definition.
/// The caller includes both reservation and registration in one SQL transaction.
pub(crate) fn reserve_changed_account_references(
    ctx: &Context,
    conn: &rusqlite::Connection,
    before: &Value,
    after: &Value,
) -> anyhow::Result<()> {
    for id in changed_account_ids(before, after) {
        if before
            .pointer("/agents/definitions")
            .and_then(|defs| defs.get(&id))
            .is_some_and(|raw| raw.get("_delete") != Some(&Value::Bool(true)))
        {
            crate::native_secrets::reserve_account_references(ctx, conn, &id, before, Some(after))?;
        }
    }
    Ok(())
}

pub(crate) fn claim_changed_account_references(
    ctx: &Context,
    conn: &rusqlite::Connection,
    before: &Value,
    after: &Value,
) -> anyhow::Result<()> {
    for id in changed_account_ids(before, after) {
        if after
            .pointer("/agents/definitions")
            .and_then(|defs| defs.get(&id))
            .is_some_and(|raw| raw.get("_delete") != Some(&Value::Bool(true)))
        {
            let names = crate::native_secrets::account_reference_names(ctx, &id, after)?;
            crate::native_secrets::claim_variable_ownership(ctx, conn, &id, &names)?;
        }
    }
    Ok(())
}

fn policy_set(ctx: &Context, request: &Request) -> DispatchResult {
    reject_unrecognized_args(
        request,
        &["--layer", "--key", "--value", "--expect-definition"],
        "policy:set",
    )?;
    if request.value("--layer") != Some("local") {
        return Err(DispatchError::new(
            "USAGE_ERROR",
            "policy:set 仅支持 --layer local",
        ));
    }
    let key = request.value("--key").ok_or_else(|| {
        DispatchError::new(
            "USAGE_ERROR",
            "policy:set 需要 --key <dot.path> --value <json>",
        )
    })?;
    let raw_value = request.value("--value").ok_or_else(|| {
        DispatchError::new(
            "USAGE_ERROR",
            "policy:set 需要 --key <dot.path> --value <json>",
        )
    })?;
    let value: Value = serde_json::from_str(raw_value).map_err(|err| {
        DispatchError::new("USAGE_ERROR", format!("--value 不是合法 JSON: {err}"))
    })?;
    let segments = validate_key(key)?;
    reject_secrets(key, &value)?;
    let expected_definition = request
        .value("--expect-definition")
        .map(|raw| {
            serde_json::from_str::<Value>(raw).map_err(|err| {
                DispatchError::new(
                    "USAGE_ERROR",
                    format!("--expect-definition 不是合法 JSON: {err}"),
                )
            })
        })
        .transpose()?;
    if let Some(expected) = &expected_definition {
        if segments.len() != 3
            || segments[0] != "agents"
            || segments[1] != "definitions"
            || (!expected.is_object() && !expected.is_null())
        {
            return Err(DispatchError::new(
                "USAGE_ERROR",
                "--expect-definition 只支持 agents.definitions.<id> 的对象或 null 基准",
            ));
        }
    }

    let before_policy = store::effective_policy(ctx).map_err(DispatchError::from)?;
    let mut candidate_policy = before_policy.clone();
    if let Some(expected) = &expected_definition {
        check_expected_agent_definition(&candidate_policy, &segments, expected)?;
    }
    set_dot_path(&mut candidate_policy, &segments, value.clone())?;
    prepare_account_generations(&before_policy, &mut candidate_policy, false, Some(key))
        .map_err(|err| DispatchError::new("ACCOUNT_ISOLATION_ERROR", format!("{err:#}")))?;
    validate_mcp_policy_candidate(key, &value, &candidate_policy)?;
    validate_account_policy_change(ctx, &before_policy, &candidate_policy)
        .map_err(|err| DispatchError::new("ACCOUNT_ISOLATION_ERROR", format!("{err:#}")))?;
    let _account_locks = crate::native_account_context::lock_policy_account_changes(
        ctx,
        &before_policy,
        &candidate_policy,
    )
    .map_err(DispatchError::from)?;

    // 首次明确保存即初始化库并固化内置默认；同连接同事务完成读-改-写。
    let mut conn = store::open_write(ctx).map_err(DispatchError::from)?;
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(DispatchError::from)?;
    let mut policy = store::policy_from_connection(&tx).map_err(DispatchError::from)?;
    if !store::policy_matches_write_snapshot(&tx, &policy, &before_policy)
        .map_err(DispatchError::from)?
    {
        return Err(DispatchError::new(
            "POLICY_CONFLICT",
            "账号登记在预检后发生变化，未保存；请刷新后重试",
        ));
    }
    let before = policy.clone();
    if let Some(expected) = &expected_definition {
        check_expected_agent_definition(&policy, &segments, expected)?;
    }
    set_dot_path(&mut policy, &segments, value.clone())?;
    prepare_account_generations(&before, &mut policy, false, Some(key))
        .map_err(|err| DispatchError::new("ACCOUNT_ISOLATION_ERROR", format!("{err:#}")))?;
    validate_mcp_policy_candidate(key, &value, &policy)?;
    validate_account_policy_change(ctx, &before, &policy)
        .map_err(|err| DispatchError::new("ACCOUNT_ISOLATION_ERROR", format!("{err:#}")))?;
    reserve_changed_account_references(ctx, &tx, &before, &policy)
        .map_err(|err| DispatchError::new("ACCOUNT_ISOLATION_ERROR", format!("{err:#}")))?;
    store::write_policy(&tx, &policy).map_err(DispatchError::from)?;
    claim_changed_account_references(ctx, &tx, &before, &policy)
        .map_err(|err| DispatchError::new("ACCOUNT_ISOLATION_ERROR", format!("{err:#}")))?;
    // MCP 定义不再留在策略里：同一事务里转成「未分配的已保存定义」（mcp_saved）。
    if key == "mcp" || key.starts_with("mcp.") {
        let scans = crate::native_mcp::inventory::scan_targets_checked(ctx, &policy);
        crate::native_mcp::legacy::migrate(&tx, &scans, Some(&request.run_id))
            .map_err(|err| DispatchError::new("USAGE_ERROR", format!("{err:#}")))?;
    }
    store::record_operation(
        &tx,
        Some(&request.run_id),
        "policy:set",
        Some(key),
        Some(&json!({ "key": key, "layer": "local" })),
    )
    .map_err(DispatchError::from)?;
    tx.commit().map_err(DispatchError::from)?;
    let data = policy_data(
        Some("sqlite"),
        Some(key),
        Some(value),
        Some(store::db_path(ctx).display().to_string()),
        None,
        None,
    );
    Ok((data, vec![]))
}

fn policy_delete(ctx: &Context, request: &Request) -> DispatchResult {
    reject_unrecognized_args(request, &["--layer", "--key"], "policy:delete")?;
    if request.value("--layer") != Some("local") {
        return Err(DispatchError::new(
            "USAGE_ERROR",
            "policy:delete 仅支持 --layer local",
        ));
    }
    let key = request
        .value("--key")
        .ok_or_else(|| DispatchError::new("USAGE_ERROR", "policy:delete 需要 --key <dot.path>"))?;
    let segments = validate_key(key)?;
    let before = store::effective_policy(ctx).map_err(DispatchError::from)?;
    let mut after = before.clone();
    delete_dot_path(&mut after, &segments)?;
    prepare_account_generations(&before, &mut after, false, None)
        .map_err(|err| DispatchError::new("ACCOUNT_ISOLATION_ERROR", format!("{err:#}")))?;
    let _account_locks =
        crate::native_account_context::lock_policy_account_changes(ctx, &before, &after)
            .map_err(DispatchError::from)?;
    let mut conn = match store::open_readonly(ctx).map_err(DispatchError::from)? {
        // 无库即无键：删除不建库
        None => {
            return Err(DispatchError::new(
                "POLICY_KEY_NOT_FOUND",
                format!("键不存在: {key}"),
            ))
        }
        Some(_) => store::open_write(ctx).map_err(DispatchError::from)?,
    };
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(DispatchError::from)?;
    let mut policy = store::policy_from_connection(&tx).map_err(DispatchError::from)?;
    if !store::policy_matches_write_snapshot(&tx, &policy, &before).map_err(DispatchError::from)? {
        return Err(DispatchError::new(
            "POLICY_CONFLICT",
            "账号登记在预检后发生变化，未删除；请刷新后重试",
        ));
    }
    delete_dot_path(&mut policy, &segments)?;
    reserve_changed_account_references(ctx, &tx, &before, &policy)
        .map_err(|err| DispatchError::new("ACCOUNT_ISOLATION_ERROR", format!("{err:#}")))?;
    store::write_policy(&tx, &policy).map_err(DispatchError::from)?;
    store::record_operation(&tx, Some(&request.run_id), "policy:delete", Some(key), None)
        .map_err(DispatchError::from)?;
    tx.commit().map_err(DispatchError::from)?;
    let data = policy_data(
        Some("sqlite"),
        Some(key),
        None,
        Some(store::db_path(ctx).display().to_string()),
        None,
        None,
    );
    Ok((data, vec![]))
}

// ---------------------------------------------------------------------------
// db domain
// ---------------------------------------------------------------------------

fn db_status(ctx: &Context) -> DispatchResult {
    let path = store::db_path(ctx);
    let exists = path.is_file();
    let mut meta = Map::new();
    let mut ready = false;
    match store::open_readonly(ctx) {
        Ok(Some(conn)) => {
            ready = true;
            if let Ok(mut stmt) = conn.prepare("SELECT key, value FROM schema_meta") {
                let rows = stmt.query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                });
                if let Ok(rows) = rows {
                    for row in rows.flatten() {
                        meta.insert(row.0, Value::String(row.1));
                    }
                }
            }
            let count = |sql: &str| {
                conn.query_row(sql, [], |row| row.get::<_, i64>(0))
                    .unwrap_or(0)
            };
            meta.insert(
                "counts".into(),
                json!({
                    "policies": count("SELECT COUNT(*) FROM policy_entries"),
                    "operations": count("SELECT COUNT(*) FROM operation_logs"),
                }),
            );
        }
        Ok(None) => {}
        Err(err) => {
            meta.insert("error".into(), Value::String(format!("{err:#}")));
        }
    }
    let data = json!({
        "engine": "native",
        "active": exists,
        "ready": ready,
        "data_dir": ctx.data_dir.display().to_string(),
        "db_path": path.display().to_string(),
        "exists": exists,
        "meta": Value::Object(meta),
    });
    Ok((data, vec![]))
}

fn db_migrate(ctx: &Context, request: &Request) -> DispatchResult {
    reject_unrecognized_args(request, &[], "db:migrate")?;
    store::open_write(ctx).map_err(DispatchError::from)?;
    db_status(ctx)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_ctx(name: &str) -> (std::path::PathBuf, Context) {
        let root =
            std::env::temp_dir().join(format!("am-native-core-{}-{}", std::process::id(), name));
        let _ = std::fs::remove_dir_all(&root);
        (
            root.clone(),
            Context {
                data_dir: root.join("data"),
                home_dir: root.join("home"),
            },
        )
    }

    fn request(command: &str, args: &[&str]) -> Request {
        Request {
            command: command.to_string(),
            args: args.iter().map(|s| s.to_string()).collect(),
            run_id: "run-test".to_string(),
        }
    }

    #[test]
    fn apply_lock_waits_out_a_briefly_held_handle_but_refuses_a_real_holder() {
        let (root, ctx) = temp_ctx("apply-lock-grace");
        std::fs::create_dir_all(&ctx.data_dir).unwrap();
        let path = ctx.data_dir.join("agent-manager.apply.lock");
        let open = || {
            OpenOptions::new()
                .create(true)
                .read(true)
                .write(true)
                .open(&path)
                .unwrap()
        };
        // 像 fork 出来、还没 exec 的子进程那样短暂占着：稍后释放，取锁应等到它
        let brief = open();
        brief.lock_exclusive().unwrap();
        let releaser = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(100));
            drop(brief);
        });
        let lock = acquire_apply_lock(&ctx, "Test");
        releaser.join().unwrap();
        assert!(lock.is_ok(), "短暂占用应等过去");
        drop(lock);

        // 真在执行的写动作一直占着：宽限期过后拒绝
        let holder = open();
        holder.lock_exclusive().unwrap();
        let started = std::time::Instant::now();
        let err = acquire_apply_lock(&ctx, "Test").err().expect("应拒绝");
        assert!(started.elapsed() >= APPLY_LOCK_GRACE);
        assert!(format!("{err}").contains("拒绝并发动作"), "{err}");
        drop(holder);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn url_userinfo_is_detected_and_redacted() {
        assert!(url_has_userinfo("https://u:p@gw.example.com/v1"));
        assert!(url_has_userinfo("http://token@10.0.0.5:8080"));
        assert!(!url_has_userinfo("https://gw.example.com/v1/users/@me"));
        assert!(!url_has_userinfo("https://gw.example.com/v1?x=a@b"));
        assert_eq!(
            redact_url_userinfo("https://u:p@gw.example.com/v1?q=1"),
            "https://***@gw.example.com/v1?q=1"
        );
        assert_eq!(
            redact_url_userinfo("https://gw.example.com/a@b"),
            "https://gw.example.com/a@b"
        );
    }

    #[test]
    fn credential_names_match_by_word_class() {
        for name in [
            "GITHUB_TOKEN",
            "ANTHROPIC_AUTH_TOKEN",
            "OPENAI_API_KEY",
            "API_KEYS",
            "apiKey",
            "accessToken",
            "APIKey",
            "S3Key",
            "clientSecret",
            "CLIENTSECRET",
            "AWS_SECRET_ACCESS_KEY",
            "GOOGLE_APPLICATION_CREDENTIALS",
            "SERVICE_CREDS",
            "Authorization",
            "X-Api-Key",
            "X-Auth",
            "OPENAI_APIKEY",
            "VOLC_ACCESSKEY",
            "GITHUB_PAT",
            "DB_PASS",
            "DB_PWD",
            "MY_PASSWD",
            "experimental_bearer_token",
        ] {
            assert!(credential_like_name(name), "{name} 应算凭据");
        }
        for name in [
            "CLAUDE_CODE_MAX_OUTPUT_TOKENS",
            "MAX_THINKING_TOKENS",
            "maxOutputTokens",
            "preferred_auth_method",
            "PWD",
            "OLDPWD",
            "PATH",
            "PASS_THROUGH",
            "KEYBINDINGS",
            "NODE_ENV",
        ] {
            assert!(!credential_like_name(name), "{name} 不应算凭据");
        }
    }

    #[test]
    fn github_fine_grained_pat_value_is_recognised() {
        assert_eq!(
            reject_secret_text("github_pat_11ABCDEFG0123456789_abcdefghijklmnop"),
            Some("github_token")
        );
    }

    #[test]
    fn six_domain_plan_is_unknown_verb() {
        let (root, context) = temp_ctx("plan-unknown");
        for command in [
            "agents:plan",
            "skills:plan",
            "mcp:plan",
            "rules:plan",
            "aliases:plan",
            "skills.plan", // 点号形式同样归一化为 UNKNOWN_VERB
        ] {
            let env = execute(&context, &request(command, &[]));
            assert!(!env.ok, "{command} 不应成功");
            assert_eq!(
                env.errors[0].code, "UNKNOWN_VERB",
                "{command} 应报 UNKNOWN_VERB"
            );
        }
        assert!(
            !context.data_dir.exists(),
            "plan 拒绝不得创建数据目录/库/日志"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn policy_set_delete_and_db_migrate_reject_dry_run_before_any_write() {
        let (root, context) = temp_ctx("dry-run-reject");
        let db = store::db_path(&context);
        for (command, args) in [
            (
                "policy:set",
                &[
                    "--layer",
                    "local",
                    "--key",
                    "agents.x",
                    "--value",
                    "1",
                    "--dry-run",
                ][..],
            ),
            (
                "policy:delete",
                &["--layer", "local", "--key", "agents.x", "--dry-run"][..],
            ),
            ("db:migrate", &["--dry-run"][..]),
        ] {
            let env = execute(&context, &request(command, args));
            assert!(!env.ok, "{command} --dry-run 不应成功");
            assert_eq!(
                env.errors[0].code, "USAGE_ERROR",
                "{command} --dry-run 应以 USAGE_ERROR 拒绝"
            );
            assert!(
                !db.exists(),
                "{command} --dry-run 被拒绝后不得创建 native-manager.db"
            );
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn policy_and_db_reject_unrecognized_args() {
        let (root, context) = temp_ctx("unknown-args");
        let db = store::db_path(&context);
        for (command, args) in [
            (
                "policy:set",
                &[
                    "--layer", "local", "--key", "agents.x", "--value", "1", "--bogus",
                ][..],
            ),
            (
                "policy:delete",
                &["--layer", "local", "--key", "agents.x", "--bogus"][..],
            ),
            ("db:migrate", &["--bogus"][..]),
            ("db:migrate", &["positional"][..]),
        ] {
            let env = execute(&context, &request(command, args));
            assert!(!env.ok, "{command} {args:?} 不应成功");
            assert_eq!(env.errors[0].code, "USAGE_ERROR");
            assert!(!db.exists(), "{command} {args:?} 被拒绝后不得建库");
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn rejected_policy_set_dry_run_leaves_existing_db_untouched() {
        let (root, context) = temp_ctx("reject-zero-write");
        // 先建库，再验证被拒绝的 --dry-run 请求对库零写。
        let conn = store::open_write(&context).unwrap();
        drop(conn);
        let db = store::db_path(&context);
        let bytes_before = std::fs::read(&db).unwrap();

        let env = execute(
            &context,
            &request(
                "policy:set",
                &[
                    "--layer",
                    "local",
                    "--key",
                    "agents.x",
                    "--value",
                    "1",
                    "--dry-run",
                ],
            ),
        );
        assert!(!env.ok);
        assert_eq!(env.errors[0].code, "USAGE_ERROR");
        let bytes_after = std::fs::read(&db).unwrap();
        assert_eq!(bytes_before, bytes_after, "拒绝必须先于任何写");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn policy_set_happy_path_still_writes() {
        let (root, context) = temp_ctx("happy");
        let env = execute(
            &context,
            &request(
                "policy:set",
                &[
                    "--layer",
                    "local",
                    "--key",
                    "agents.claude-code.enabled",
                    "--value",
                    "false",
                ],
            ),
        );
        assert!(env.ok, "{:?}", env.errors);
        assert!(store::db_path(&context).exists());
        let policy = store::effective_policy(&context).unwrap();
        assert_eq!(
            policy["agents"]["claude-code"]["enabled"],
            Value::Bool(false)
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn db_migrate_without_args_initializes_store() {
        let (root, context) = temp_ctx("migrate");
        let env = execute(&context, &request("db:migrate", &[]));
        assert!(env.ok, "{:?}", env.errors);
        assert!(store::db_path(&context).exists());
        let _ = std::fs::remove_dir_all(root);
    }
}
