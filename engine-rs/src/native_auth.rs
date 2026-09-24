//! 助手官方账号登录的只读判断。
//!
//! 只经官方 CLI 的状态命令（`claude auth status` / `codex login status`）和官方文档说明的
//! 登录记录判断，不读取任何凭据内容、不返回账号或令牌、不写文件。认证修改只由用户在终端
//! 亲自执行官方命令完成，本工具从不代为登录或退出。

use serde_json::{json, Value};
use std::path::Path;

#[derive(Clone, Copy)]
pub(crate) enum AuthKind {
    Claude,
    Codex,
}

/// 探测时从子进程环境里去掉的变量：它们会让官方命令报告进程环境里的凭据或别的来源，
/// 而不是这个配置目录里存的登录。依据官方认证优先级：云厂商开关、两种第三方凭据、官方长期
/// 令牌、Anthropic 配置档案与工作负载身份联合（WIF）变量都排在 `/login` 之前。
const CLAUDE_PROBE_ENV_REMOVE: &[&str] = &[
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_API_KEY",
    "CLAUDE_CODE_OAUTH_TOKEN",
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_VERTEX",
    "CLAUDE_CODE_USE_FOUNDRY",
    "CLAUDE_CODE_USE_ANTHROPIC_AWS",
    "CLAUDE_CODE_USE_MANTLE",
    "ANTHROPIC_PROFILE",
    "ANTHROPIC_FEDERATION_RULE_ID",
    "ANTHROPIC_ORGANIZATION_ID",
    "ANTHROPIC_SERVICE_ACCOUNT_ID",
    "ANTHROPIC_IDENTITY_TOKEN",
    "ANTHROPIC_IDENTITY_TOKEN_FILE",
    "ANTHROPIC_WORKSPACE_ID",
    // 只认 API key、不读登录（--bare）；嵌入宿主用它让配置文件里的凭据失效
    "CLAUDE_CODE_SIMPLE",
    "CLAUDE_CODE_PROVIDER_MANAGED_BY_HOST",
    // 未公开，但若从外部继承会把登录存储指到别的目录
    "CLAUDE_SECURESTORAGE_CONFIG_DIR",
];
/// Anthropic 配置档案（`~/.config/anthropic`，WIF 参考文档）不随 `CLAUDE_CONFIG_DIR` 区分：
/// 活动档案是联合身份时，官方命令对任何配置目录都报「已登录」。探测时把档案目录指到一个
/// 不存在的位置，只看这个目录自己的登录；实测官方命令只读、不会创建它。档案本身在
/// `credential_overrides` 里如实提示。
#[cfg(unix)]
const CLAUDE_PROBE_NO_PROFILE_DIR: &str = "/nonexistent/agent-manager-probe/anthropic";
#[cfg(not(unix))]
const CLAUDE_PROBE_NO_PROFILE_DIR: &str = "Z:\\nonexistent\\agent-manager-probe\\anthropic";
/// 经官方 `--settings` 取消的凭据来源（CLI 参数优先于配置目录里的 settings.json）：目录自己
/// 声明的第三方凭据、长期令牌、云厂商开关与 apiKeyHelper 都会遮住目录里的登录，取消后官方命令
/// 报告的就是这个目录存的登录。只在这次子进程里生效，不改任何文件。
const CLAUDE_PROBE_CANCELLED_ENV: &[&str] = &[
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_API_KEY",
    "CLAUDE_CODE_OAUTH_TOKEN",
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_VERTEX",
    "CLAUDE_CODE_USE_FOUNDRY",
    "CLAUDE_CODE_USE_ANTHROPIC_AWS",
    "CLAUDE_CODE_USE_MANTLE",
];
/// Console 登录生成、由 Claude Code 自己保管的 API key 在 `auth status` 里的来源名。
const CLAUDE_CONSOLE_KEY_SOURCE: &str = "/login managed key";

fn claude_probe_settings() -> String {
    let env: serde_json::Map<String, Value> = CLAUDE_PROBE_CANCELLED_ENV
        .iter()
        .map(|name| (name.to_string(), Value::String(String::new())))
        .collect();
    json!({"apiKeyHelper": "", "env": env}).to_string()
}
const CODEX_PROBE_ENV_REMOVE: &[&str] = &["CODEX_API_KEY", "OPENAI_API_KEY", "CODEX_ACCESS_TOKEN"];
const PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

/// 官方 CLI：先找 `<HOME>/.local/bin`（官方安装器的落点），再找 PATH，最后是其他常见
/// 安装目录（桌面应用的 PATH 里没有 Homebrew、npm 全局目录）。
fn resolve_cli(home_dir: &Path, bin: &str) -> Option<std::path::PathBuf> {
    let local = home_dir.join(".local").join("bin").join(bin);
    if local.is_file() {
        return Some(local);
    }
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .chain(crate::native_agents::fallback_bin_dirs(home_dir))
        .map(|dir| dir.join(bin))
        .find(|path| path.is_file())
}

/// `.claude.json` 里有没有登录记录（官方文档：这个文件保存登录会话）：`oauthAccount` 是
/// claude.ai 与 Console 登录的账号记录，`primaryApiKey` 是 Console 登录在 Linux 上保管的 key。
/// 只看键是否存在，不读取取值。文件存在但解析不了时无法确认，报错。
///
/// 返回 `None` 表示没有登录记录，`Some(方式)` 是能从记录看出的登录方式（看不出时为 `None`）。
fn claude_json_login(path: &Path) -> Result<Option<Option<&'static str>>, String> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("无法读取 .claude.json".into()),
    };
    let root = serde_json::from_slice::<Value>(&bytes)
        .map_err(|_| ".claude.json 不是合法 JSON".to_string())?;
    let has = |key: &str| root.get(key).is_some_and(|value| !value.is_null());
    Ok(if has("primaryApiKey") {
        Some(Some("console"))
    } else if has("oauthAccount") {
        Some(None)
    } else {
        None
    })
}

/// 官方账号登录状态：`method` 是官方命令报告的登录方式（不含账号与令牌）。
pub(crate) struct LoginStatus {
    pub logged_in: bool,
    pub method: Option<&'static str>,
}

fn logged_out() -> LoginStatus {
    LoginStatus {
        logged_in: false,
        method: None,
    }
}

/// 某个配置目录里存的官方账号登录。`config_dir` 为 `None` 表示助手的主配置（不设
/// `CLAUDE_CONFIG_DIR` / 用默认 `CODEX_HOME`），`Some` 是实例目录。`Err` 表示无法确认。
pub(crate) fn official_status(
    home_dir: &Path,
    kind: AuthKind,
    bin: &str,
    config_dir: Option<&Path>,
) -> Result<LoginStatus, String> {
    if config_dir.is_some_and(|dir| !dir.is_dir()) {
        return Ok(logged_out());
    }
    // Claude 的登录记录：设了配置目录时都在目录里；主配置的 .claude.json 在 HOME 下
    let claude_json = config_dir.map_or_else(
        || home_dir.join(".claude.json"),
        |dir| dir.join(".claude.json"),
    );
    // `.credentials.json` 还存 MCP 服务的 OAuth 登录（官方认证文档），存在不等于登录了账号，
    // 以官方命令为准；`.claude.json` 的登录记录不受目录里声明的凭据影响，先看它。
    if matches!(kind, AuthKind::Claude) {
        if let Some(method) = claude_json_login(&claude_json)? {
            return Ok(LoginStatus {
                logged_in: true,
                method,
            });
        }
    }
    let cli = resolve_cli(home_dir, bin)
        .ok_or_else(|| format!("找不到 {bin} 命令，无法用官方命令确认登录状态"))?;
    let mut command = std::process::Command::new(&cli);
    command
        .env("HOME", home_dir)
        .env("PATH", crate::native_agents::path_for_command(&cli));
    match kind {
        AuthKind::Claude => {
            command.args(["--settings", &claude_probe_settings(), "auth", "status"]);
            match config_dir {
                Some(dir) => command.env("CLAUDE_CONFIG_DIR", dir),
                None => command.env_remove("CLAUDE_CONFIG_DIR"),
            };
            for name in CLAUDE_PROBE_ENV_REMOVE {
                command.env_remove(name);
            }
            command.env("ANTHROPIC_CONFIG_DIR", CLAUDE_PROBE_NO_PROFILE_DIR);
            let (code, stdout, _) = crate::native_upstream::capture_exit(command, PROBE_TIMEOUT)
                .ok_or_else(|| format!("{bin} auth status 没有正常结束"))?;
            let status = serde_json::from_slice::<Value>(&stdout)
                .ok()
                .filter(|_| code == 0 || code == 1)
                .ok_or_else(|| format!("{bin} auth status 的输出无法识别"))?;
            if status.get("loggedIn").and_then(Value::as_bool) != Some(true) {
                return Ok(logged_out());
            }
            // 目录声明的凭据已经取消：剩下的 claude.ai / Console 登录是这个目录存的；长期令牌
            // 只可能来自托管设置，按有登录处理（不放行第三方凭据写进来）。API key、apiKeyHelper
            // 与云厂商（third_party）也只可能来自托管设置，不是官方账号登录。
            let key_source = status.get("apiKeySource").and_then(Value::as_str);
            let method = match status.get("authMethod").and_then(Value::as_str) {
                _ if key_source == Some(CLAUDE_CONSOLE_KEY_SOURCE) => "console",
                Some("claude.ai") => "claude.ai",
                Some("oauth_token") => "oauth_token",
                _ => return Ok(logged_out()),
            };
            Ok(LoginStatus {
                logged_in: true,
                method: Some(method),
            })
        }
        AuthKind::Codex => {
            let codex_home = config_dir.map_or_else(|| home_dir.join(".codex"), Path::to_path_buf);
            command
                .args(["login", "status"])
                .env("CODEX_HOME", codex_home);
            for name in CODEX_PROBE_ENV_REMOVE {
                command.env_remove(name);
            }
            // 退出码 1 也可能是出错；只有明确说「未登录」才算确认没有
            match crate::native_upstream::capture_exit(command, PROBE_TIMEOUT) {
                Some((0, stdout, stderr)) => {
                    let text = format!(
                        "{}{}",
                        String::from_utf8_lossy(&stdout),
                        String::from_utf8_lossy(&stderr)
                    );
                    let method = if text.contains("ChatGPT") {
                        "chatgpt"
                    } else if text.contains("API key") {
                        "api_key"
                    } else {
                        "other"
                    };
                    Ok(LoginStatus {
                        logged_in: true,
                        method: Some(method),
                    })
                }
                Some((1, stdout, stderr))
                    if String::from_utf8_lossy(&stdout).contains("Not logged in")
                        || String::from_utf8_lossy(&stderr).contains("Not logged in") =>
                {
                    Ok(logged_out())
                }
                _ => Err(format!("{bin} login status 没有给出可识别的结果")),
            }
        }
    }
}

/// 实例目录里是否存有官方账号登录；`Err` 表示无法确认，调用方应按「有」处理。
pub(crate) fn official_login(
    home_dir: &Path,
    kind: AuthKind,
    bin: &str,
    config_dir: &Path,
) -> Result<bool, String> {
    official_status(home_dir, kind, bin, Some(config_dir)).map(|status| status.logged_in)
}

/// `agents:auth-status --only <agents:助手 | profiles:实例>`：用官方命令查一次登录状态。
///
/// 只读：不写库、不取锁、不读取凭据内容，只返回是否登录与登录方式。第三方实例里有官方登录时
/// 标为 `mixed`（两种凭据混在同一个配置目录）。官方命令一次要一两秒，只在界面上点它时才调用。
pub(crate) fn status_command(
    ctx: &crate::native::Context,
    req: &crate::native::Request,
) -> anyhow::Result<Value> {
    let mut only = None;
    let mut iter = req.args.iter();
    while let Some(arg) = iter.next() {
        match (arg.as_str(), only.is_some()) {
            ("--only", false) => only = iter.next().cloned(),
            _ => {
                anyhow::bail!("agents:auth-status 只接受一个 --only <agents:助手 | profiles:实例>")
            }
        }
    }
    let only = only.ok_or_else(|| anyhow::anyhow!("agents:auth-status 需要 --only"))?;
    let (kind, bin, config_dir, third_party) = if let Some(id) = only.strip_prefix("profiles:") {
        let policy = crate::store::effective_policy(ctx)?;
        let def = crate::native_profiles::model::load_definitions(&policy)
            .into_iter()
            .flatten()
            .find(|def| def.id == id)
            .ok_or_else(|| anyhow::anyhow!("未知实例: {id}"))?;
        let dir =
            crate::native_profiles::model::validate_config_dir(&def, &ctx.home_dir, &ctx.data_dir)?;
        let kind = match def.kind {
            crate::native_profiles::model::ProfileKind::Claude => AuthKind::Claude,
            crate::native_profiles::model::ProfileKind::Codex => AuthKind::Codex,
        };
        let bin = crate::native_profiles::status::base_bin(ctx, &def);
        (
            kind,
            bin,
            Some(dir),
            crate::native_profiles::model::is_third_party(&def),
        )
    } else {
        let id = only.strip_prefix("agents:").unwrap_or(&only);
        let def = crate::native_agents::load_definitions(ctx)?
            .into_iter()
            .find(|def| def.id == id)
            .ok_or_else(|| anyhow::anyhow!("没有这个助手：{id}"))?;
        if def.spec_ref().get("profile").is_some() {
            anyhow::bail!("{id} 是独立环境，请用 --only profiles:{id}");
        }
        let kind = match crate::native_agent_adapters::kind_for_bin(&def.bin) {
            Some(crate::native_agent_adapters::AdapterKind::Claude) => AuthKind::Claude,
            Some(crate::native_agent_adapters::AdapterKind::Codex) => AuthKind::Codex,
            None => anyhow::bail!("只有 Claude Code 与 Codex 有官方账号登录状态"),
        };
        (kind, def.bin.clone(), None, false)
    };
    let result = official_status(&ctx.home_dir, kind, &bin, config_dir.as_deref());
    Ok(match result {
        Ok(status) => json!({
            "element": only,
            "state": if status.logged_in { "logged-in" } else { "logged-out" },
            "logged_in": status.logged_in,
            "method": status.method,
            "third_party": third_party,
            "mixed": third_party && status.logged_in,
            "reason": null,
        }),
        Err(reason) => json!({
            "element": only,
            "state": "unknown",
            "logged_in": null,
            "method": null,
            "third_party": third_party,
            "mixed": false,
            "reason": reason,
        }),
    })
}

/// 会压过主配置里官方账号的外部设置（官方认证优先级文档 / 源码确认的变量）：主配置只放官方
/// 账号，但这些来源不归本工具管，只如实提示。
const CLAUDE_OVERRIDE_ENV: &[&str] = &[
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_BASE_URL",
    "CLAUDE_CODE_OAUTH_TOKEN",
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_VERTEX",
    "CLAUDE_CODE_USE_FOUNDRY",
    "CLAUDE_CODE_USE_ANTHROPIC_AWS",
    "CLAUDE_CODE_USE_MANTLE",
    // Anthropic 配置档案与工作负载身份联合：命名档案与联合身份都排在 /login 之前
    "ANTHROPIC_PROFILE",
    "ANTHROPIC_FEDERATION_RULE_ID",
    // 附加请求头，可以带 Authorization 等凭据头
    "ANTHROPIC_CUSTOM_HEADERS",
];
/// Codex：`CODEX_ACCESS_TOKEN` 所有入口都读、`CODEX_API_KEY` 由 `codex exec` 等非交互入口读，
/// 都优先于 `auth.json` 里的登录（`rust-v0.156.1` `login/src/auth/manager.rs`）。
const CODEX_OVERRIDE_ENV: &[&str] = &["CODEX_API_KEY", "CODEX_ACCESS_TOKEN"];
/// Claude `settings.json`（主配置与托管设置）的 `env` 里会压过官方账号、又不属于第三方接入键的变量。
/// 第三方接入键（地址与两种第三方凭据）写在主配置里由「混用」提示处理，这里只看托管设置里的。
const CLAUDE_SETTINGS_OVERRIDE_ENV: &[&str] = &[
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_VERTEX",
    "CLAUDE_CODE_USE_FOUNDRY",
    "CLAUDE_CODE_USE_ANTHROPIC_AWS",
    "CLAUDE_CODE_USE_MANTLE",
    "CLAUDE_CODE_OAUTH_TOKEN",
    "ANTHROPIC_PROFILE",
    "ANTHROPIC_CUSTOM_HEADERS",
];
const CLAUDE_MANAGED_ONLY_ENV: &[&str] = &[
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_BASE_URL",
];
/// 托管设置里限定登录方式的键：设了任一个，用第三方凭据（API key、令牌、apiKeyHelper）的会话在
/// 启动时就被拦下（官方设置参考 `forceLoginMethod` / `forceLoginOrgUUID`）。
const CLAUDE_MANAGED_LOGIN_KEYS: &[&str] = &["forceLoginMethod", "forceLoginOrgUUID"];
/// shell 启动时会读的配置文件（相对 HOME）。
const SHELL_RC_FILES: &[&str] = &[
    ".zshenv",
    ".zshrc",
    ".zprofile",
    ".bashrc",
    ".bash_profile",
    ".profile",
];
const MAX_RC_BYTES: u64 = 1024 * 1024;
const MAX_JSON_BYTES: u64 = 1024 * 1024;

/// 一行 shell 配置是否给 `name` 赋值（`NAME=…` 或 `export NAME=…`，注释行不算）。
fn assigns(line: &str, name: &str) -> bool {
    let line = line.trim_start();
    if line.starts_with('#') {
        return false;
    }
    let line = line.strip_prefix("export ").map_or(line, str::trim_start);
    line.strip_prefix(name)
        .is_some_and(|rest| rest.starts_with('='))
}

/// 读一个不太大的 JSON 文件；读不了、太大或不是 JSON 都当作没有（提示是尽力而为，不报错）。
fn read_small_json(path: &Path) -> Option<Value> {
    std::fs::metadata(path)
        .ok()
        .filter(|meta| meta.is_file() && meta.len() <= MAX_JSON_BYTES)?;
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

/// 取值是否真的打开了（非空；开关类的 `0` 也不算）。
fn set_value(value: Option<&Value>) -> bool {
    value
        .and_then(Value::as_str)
        .is_some_and(|text| !text.is_empty() && text != "0")
}

/// Claude 托管设置文件（官方设置文档：各平台固定目录下的 `managed-settings.json` 与
/// `managed-settings.d/*.json`，优先级最高）。macOS 描述文件与 Windows 注册表不在这里看。
fn claude_managed_settings_dirs() -> Vec<std::path::PathBuf> {
    #[cfg(target_os = "macos")]
    let dirs = vec![std::path::PathBuf::from(
        "/Library/Application Support/ClaudeCode",
    )];
    #[cfg(all(unix, not(target_os = "macos")))]
    let dirs = vec![std::path::PathBuf::from("/etc/claude-code")];
    #[cfg(windows)]
    let dirs = vec![std::path::PathBuf::from("C:\\Program Files\\ClaudeCode")];
    dirs
}

fn claude_managed_settings_files(dirs: &[std::path::PathBuf]) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    for dir in dirs {
        files.push(dir.join("managed-settings.json"));
        let mut drop_ins: Vec<_> = std::fs::read_dir(dir.join("managed-settings.d"))
            .map(|entries| {
                entries
                    .filter_map(|entry| entry.ok().map(|entry| entry.path()))
                    .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
                    .collect()
            })
            .unwrap_or_default();
        drop_ins.sort();
        files.extend(drop_ins);
    }
    files
}

/// 活动的 Anthropic 配置档案是不是联合身份（WIF 参考文档：`active_config` 是一行档案名，缺省为
/// `default`；`configs/<名>.json` 不含机密，`authentication.type` 为 `oidc_federation` 时排在 /login
/// 之前）。只读这两个不含机密的文件，不碰 `credentials/`。
fn anthropic_federation_profile(home_dir: &Path) -> Option<String> {
    let base = home_dir.join(".config/anthropic");
    let name = std::fs::read_to_string(base.join("active_config"))
        .ok()
        .map(|text| text.trim().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "default".to_string());
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        || name.starts_with('.')
    {
        return None;
    }
    let config = read_small_json(&base.join("configs").join(format!("{name}.json")))?;
    (config
        .get("authentication")
        .and_then(|auth| auth.get("type"))
        .and_then(Value::as_str)
        == Some("oidc_federation"))
    .then(|| format!("~/.config/anthropic/configs/{name}.json"))
}

/// 主配置之外、会压过（或限定）官方账号的设置：`[{source, name}]`，只有来源与变量名，不含取值。
/// 只读：读 shell 配置文件、Claude 的 settings.json 与托管设置、Anthropic 配置档案、Codex 的
/// config.toml 与本进程环境（`env_set` 注入，便于测试）。
pub(crate) fn credential_overrides(
    home_dir: &Path,
    kind: AuthKind,
    env_set: &dyn Fn(&str) -> bool,
) -> Vec<Value> {
    credential_overrides_with(home_dir, kind, env_set, &claude_managed_settings_dirs())
}

fn credential_overrides_with(
    home_dir: &Path,
    kind: AuthKind,
    env_set: &dyn Fn(&str) -> bool,
    managed_dirs: &[std::path::PathBuf],
) -> Vec<Value> {
    let names = match kind {
        AuthKind::Claude => CLAUDE_OVERRIDE_ENV,
        AuthKind::Codex => CODEX_OVERRIDE_ENV,
    };
    let mut out = Vec::new();
    for file in SHELL_RC_FILES {
        let path = home_dir.join(file);
        let Some(text) = std::fs::metadata(&path)
            .ok()
            .filter(|meta| meta.is_file() && meta.len() <= MAX_RC_BYTES)
            .and_then(|_| std::fs::read_to_string(&path).ok())
        else {
            continue;
        };
        for name in names {
            if text.lines().any(|line| assigns(line, name)) {
                out.push(json!({"source": format!("~/{file}"), "name": name}));
            }
        }
    }
    for name in names {
        if env_set(name) {
            out.push(json!({"source": "进程环境", "name": name}));
        }
    }
    match kind {
        AuthKind::Claude => {
            if let Some(root) = read_small_json(&home_dir.join(".claude/settings.json")) {
                if set_value(root.get("apiKeyHelper")) {
                    out.push(json!({"source": "~/.claude/settings.json", "name": "apiKeyHelper"}));
                }
                for name in CLAUDE_SETTINGS_OVERRIDE_ENV {
                    if set_value(root.get("env").and_then(|env| env.get(*name))) {
                        out.push(json!({"source": "~/.claude/settings.json", "name": name}));
                    }
                }
            }
            for path in claude_managed_settings_files(managed_dirs) {
                let Some(root) = read_small_json(&path) else {
                    continue;
                };
                let source = path.display().to_string();
                if set_value(root.get("apiKeyHelper")) {
                    out.push(json!({"source": source, "name": "apiKeyHelper"}));
                }
                for name in CLAUDE_SETTINGS_OVERRIDE_ENV
                    .iter()
                    .chain(CLAUDE_MANAGED_ONLY_ENV)
                {
                    if set_value(root.get("env").and_then(|env| env.get(*name))) {
                        out.push(json!({"source": source, "name": name}));
                    }
                }
                for key in CLAUDE_MANAGED_LOGIN_KEYS {
                    let value = root.get(*key);
                    if set_value(value)
                        || value
                            .and_then(Value::as_array)
                            .is_some_and(|a| !a.is_empty())
                    {
                        out.push(json!({"source": source, "name": key}));
                    }
                }
            }
            if let Some(source) = anthropic_federation_profile(home_dir) {
                out.push(json!({"source": source, "name": "oidc_federation"}));
            }
        }
        AuthKind::Codex => {
            // 内置 openai 接入方式的地址覆盖（官方配置参考）：官方账号的凭据会随请求发往这个地址
            let config = std::fs::metadata(home_dir.join(".codex/config.toml"))
                .ok()
                .filter(|meta| meta.is_file() && meta.len() <= MAX_RC_BYTES)
                .and_then(|_| std::fs::read_to_string(home_dir.join(".codex/config.toml")).ok())
                .and_then(|text| text.parse::<toml::Table>().ok());
            if config
                .as_ref()
                .and_then(|doc| doc.get("openai_base_url"))
                .and_then(toml::Value::as_str)
                .is_some_and(|url| !url.trim().is_empty())
            {
                out.push(json!({"source": "~/.codex/config.toml", "name": "openai_base_url"}));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overrides_report_sources_and_names_but_never_values() {
        let home = std::env::temp_dir().join(format!("am-auth-overrides-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join(".claude")).unwrap();
        std::fs::write(
            home.join(".zshrc"),
            "# export ANTHROPIC_API_KEY=commented\nexport ANTHROPIC_BASE_URL=https://relay.example.com\nalias x='ANTHROPIC_AUTH_TOKEN=a claude'\n  CODEX_API_KEY=sk-secret-value\n",
        )
        .unwrap();
        std::fs::write(
            home.join(".claude/settings.json"),
            r#"{"apiKeyHelper":"~/bin/key.sh","env":{"CLAUDE_CODE_USE_BEDROCK":"1","CLAUDE_CODE_USE_VERTEX":"0"}}"#,
        )
        .unwrap();

        let claude = credential_overrides(&home, AuthKind::Claude, &|name| {
            name == "CLAUDE_CODE_OAUTH_TOKEN"
        });
        assert_eq!(
            claude,
            vec![
                json!({"source": "~/.zshrc", "name": "ANTHROPIC_BASE_URL"}),
                json!({"source": "进程环境", "name": "CLAUDE_CODE_OAUTH_TOKEN"}),
                json!({"source": "~/.claude/settings.json", "name": "apiKeyHelper"}),
                json!({"source": "~/.claude/settings.json", "name": "CLAUDE_CODE_USE_BEDROCK"}),
            ],
            "注释行、别名里的赋值与取值为 0 的开关都不算"
        );
        let codex = credential_overrides(&home, AuthKind::Codex, &|_| false);
        assert_eq!(
            codex,
            vec![json!({"source": "~/.zshrc", "name": "CODEX_API_KEY"})]
        );
        assert!(!format!("{claude:?}{codex:?}").contains("sk-secret-value"));
        assert!(!format!("{claude:?}").contains("relay.example.com"));
        let _ = std::fs::remove_dir_all(&home);
    }

    /// 官方认证优先级里新近的来源同样提示：另两个云厂商开关、命名档案与联合身份、附加请求头、
    /// 托管设置（含 drop-in 与限定登录方式的键）、活动的联合身份档案、Codex 的 openai_base_url。
    #[test]
    fn overrides_cover_newer_sources_and_managed_settings_without_values() {
        let home = std::env::temp_dir().join(format!("am-auth-overrides2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let managed = home.join("managed");
        std::fs::create_dir_all(managed.join("managed-settings.d")).unwrap();
        std::fs::create_dir_all(home.join(".config/anthropic/configs")).unwrap();
        std::fs::create_dir_all(home.join(".codex")).unwrap();
        std::fs::create_dir_all(home.join(".claude")).unwrap();
        std::fs::write(
            home.join(".bashrc"),
            "export CLAUDE_CODE_USE_MANTLE=1\nANTHROPIC_PROFILE=prod\nexport ANTHROPIC_CUSTOM_HEADERS='X-A: secret-header'\n",
        )
        .unwrap();
        std::fs::write(
            home.join(".claude/settings.json"),
            r#"{"env":{"CLAUDE_CODE_OAUTH_TOKEN":"sk-ant-oat01-secret","CLAUDE_CODE_USE_ANTHROPIC_AWS":"1"}}"#,
        )
        .unwrap();
        std::fs::write(
            managed.join("managed-settings.json"),
            r#"{"forceLoginMethod":"claudeai","forceLoginOrgUUID":["org-1"],"env":{"ANTHROPIC_BASE_URL":"https://gw.corp.example"}}"#,
        )
        .unwrap();
        std::fs::write(
            managed.join("managed-settings.d/10-helper.json"),
            r#"{"apiKeyHelper":"/opt/corp/key.sh"}"#,
        )
        .unwrap();
        std::fs::write(home.join(".config/anthropic/active_config"), "wif\n").unwrap();
        std::fs::write(
            home.join(".config/anthropic/configs/wif.json"),
            r#"{"version":"1.0","authentication":{"type":"oidc_federation","federation_rule_id":"fdrl_x"}}"#,
        )
        .unwrap();
        std::fs::write(
            home.join(".codex/config.toml"),
            "openai_base_url = \"https://relay.example.com/v1\"\n",
        )
        .unwrap();

        let claude =
            credential_overrides_with(&home, AuthKind::Claude, &|_| false, &[managed.clone()]);
        let names: Vec<(String, String)> = claude
            .iter()
            .map(|item| {
                (
                    item["source"].as_str().unwrap().to_string(),
                    item["name"].as_str().unwrap().to_string(),
                )
            })
            .collect();
        let managed_main = managed.join("managed-settings.json").display().to_string();
        let managed_drop = managed
            .join("managed-settings.d/10-helper.json")
            .display()
            .to_string();
        for expected in [
            (
                "~/.bashrc".to_string(),
                "CLAUDE_CODE_USE_MANTLE".to_string(),
            ),
            ("~/.bashrc".to_string(), "ANTHROPIC_PROFILE".to_string()),
            (
                "~/.bashrc".to_string(),
                "ANTHROPIC_CUSTOM_HEADERS".to_string(),
            ),
            (
                "~/.claude/settings.json".to_string(),
                "CLAUDE_CODE_OAUTH_TOKEN".to_string(),
            ),
            (
                "~/.claude/settings.json".to_string(),
                "CLAUDE_CODE_USE_ANTHROPIC_AWS".to_string(),
            ),
            (managed_main.clone(), "ANTHROPIC_BASE_URL".to_string()),
            (managed_main.clone(), "forceLoginMethod".to_string()),
            (managed_main.clone(), "forceLoginOrgUUID".to_string()),
            (managed_drop, "apiKeyHelper".to_string()),
            (
                "~/.config/anthropic/configs/wif.json".to_string(),
                "oidc_federation".to_string(),
            ),
        ] {
            assert!(names.contains(&expected), "缺 {expected:?}：{names:?}");
        }
        let text = format!("{claude:?}");
        for secret in [
            "secret-header",
            "sk-ant-oat01-secret",
            "gw.corp.example",
            "/opt/corp/key.sh",
            "fdrl_x",
        ] {
            assert!(!text.contains(secret), "不得带出取值 {secret}");
        }

        let codex = credential_overrides_with(&home, AuthKind::Codex, &|_| false, &[]);
        assert_eq!(
            codex,
            vec![json!({"source": "~/.codex/config.toml", "name": "openai_base_url"})]
        );
        assert!(!format!("{codex:?}").contains("relay.example.com"));

        // 用户自己用 `ant auth login` 登录的档案（user_oauth）排在 /login 之后，不提示
        std::fs::write(
            home.join(".config/anthropic/configs/wif.json"),
            r#"{"authentication":{"type":"user_oauth"}}"#,
        )
        .unwrap();
        let claude = credential_overrides_with(&home, AuthKind::Claude, &|_| false, &[]);
        assert!(!claude.iter().any(|item| item["name"] == "oidc_federation"));
        let _ = std::fs::remove_dir_all(&home);
    }

    /// 桌面应用的 PATH 里没有 npm 全局目录：命令装在那里也要找得到，且它要靠同目录的
    /// 解释器（npm 装的助手是 node 脚本）才能跑起来。
    #[test]
    fn npm_全局目录里的命令也能找到_并能用同目录的解释器运行() {
        use std::os::unix::fs::PermissionsExt;
        let home = std::env::temp_dir().join(format!("am-auth-npm-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let bin = home.join(".npm-global/bin");
        std::fs::create_dir_all(&bin).unwrap();
        let interpreter = format!("am-fake-node-{}", std::process::id());
        let cli = format!("am-npm-codex-{}", std::process::id());
        for (name, body) in [
            (
                interpreter.as_str(),
                "#!/bin/sh\nexec /bin/sh \"$@\"\n".to_string(),
            ),
            (
                cli.as_str(),
                format!("#!/usr/bin/env {interpreter}\necho 'Not logged in' >&2\nexit 1\n"),
            ),
        ] {
            let path = bin.join(name);
            std::fs::write(&path, body).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let dir = home.join(".codex-relay");
        std::fs::create_dir_all(&dir).unwrap();

        assert_eq!(resolve_cli(&home, &cli), Some(bin.join(&cli)));
        let status = official_status(&home, AuthKind::Codex, &cli, Some(&dir));
        assert_eq!(status.map(|s| s.logged_in), Ok(false));
        let _ = std::fs::remove_dir_all(&home);
    }
}
