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

/// 探测时从子进程环境里去掉的变量：它们会让官方命令报告进程环境里的凭据，
/// 而不是这个配置目录里存的登录。
const CLAUDE_PROBE_ENV_REMOVE: &[&str] = &[
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_API_KEY",
    "CLAUDE_CODE_OAUTH_TOKEN",
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_VERTEX",
    "CLAUDE_CODE_USE_FOUNDRY",
    // 未公开，但若从外部继承会把登录存储指到别的目录
    "CLAUDE_SECURESTORAGE_CONFIG_DIR",
];
const CODEX_PROBE_ENV_REMOVE: &[&str] = &["CODEX_API_KEY", "OPENAI_API_KEY", "CODEX_ACCESS_TOKEN"];
const PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

/// 官方 CLI：先找 `<HOME>/.local/bin`（官方安装器的落点），再找 PATH。
fn resolve_cli(home_dir: &Path, bin: &str) -> Option<std::path::PathBuf> {
    let local = home_dir.join(".local").join("bin").join(bin);
    if local.is_file() {
        return Some(local);
    }
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|dir| dir.join(bin))
            .find(|path| path.is_file())
    })
}

/// 该目录的 settings.json 是否自己声明了凭据：声明了时官方命令只报告它，
/// 看不到被它遮住的登录。
fn claude_settings_declare_credential(config_dir: &Path) -> bool {
    let Some(root) = std::fs::read(config_dir.join("settings.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
    else {
        return false;
    };
    let non_empty = |value: Option<&Value>| {
        value
            .and_then(Value::as_str)
            .is_some_and(|text| !text.is_empty())
    };
    let env = root.get("env");
    non_empty(env.and_then(|env| env.get("ANTHROPIC_AUTH_TOKEN")))
        || non_empty(env.and_then(|env| env.get("ANTHROPIC_API_KEY")))
        || non_empty(root.get("apiKeyHelper"))
}

/// `.claude.json` 里有没有登录会话记录（官方文档：这个文件保存登录会话）。
/// 只看键是否存在，不读取内容。文件存在但解析不了时无法确认，报错。
fn claude_json_records_login(path: &Path) -> Result<bool, String> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(_) => return Err("无法读取 .claude.json".into()),
    };
    let root = serde_json::from_slice::<Value>(&bytes)
        .map_err(|_| ".claude.json 不是合法 JSON".to_string())?;
    Ok(root
        .get("oauthAccount")
        .is_some_and(|value| !value.is_null()))
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
    let claude_dir = config_dir.map_or_else(|| home_dir.join(".claude"), Path::to_path_buf);
    let claude_json = config_dir.map_or_else(
        || home_dir.join(".claude.json"),
        |dir| dir.join(".claude.json"),
    );
    if matches!(kind, AuthKind::Claude)
        && (claude_dir.join(".credentials.json").exists()
            || claude_json_records_login(&claude_json)?)
    {
        return Ok(LoginStatus {
            logged_in: true,
            method: None,
        });
    }
    let cli = resolve_cli(home_dir, bin)
        .ok_or_else(|| format!("找不到 {bin} 命令，无法用官方命令确认登录状态"))?;
    let mut command = std::process::Command::new(&cli);
    command.env("HOME", home_dir);
    match kind {
        AuthKind::Claude => {
            command.args(["auth", "status"]);
            match config_dir {
                Some(dir) => command.env("CLAUDE_CONFIG_DIR", dir),
                None => command.env_remove("CLAUDE_CONFIG_DIR"),
            };
            for name in CLAUDE_PROBE_ENV_REMOVE {
                command.env_remove(name);
            }
            let (code, stdout, _) = crate::native_upstream::capture_exit(command, PROBE_TIMEOUT)
                .ok_or_else(|| format!("{bin} auth status 没有正常结束"))?;
            let status = serde_json::from_slice::<Value>(&stdout)
                .ok()
                .filter(|_| code == 0 || code == 1)
                .ok_or_else(|| format!("{bin} auth status 的输出无法识别"))?;
            if status.get("loggedIn").and_then(Value::as_bool) != Some(true) {
                return Ok(logged_out());
            }
            let method = match status.get("authMethod").and_then(Value::as_str) {
                Some("claude.ai") => "claude.ai",
                Some("oauth_token") => "oauth_token",
                Some("api_key") => "api_key",
                _ => "other",
            };
            // 目录自己声明了凭据时，这两种方式来自那份凭据；登录记录已在上面查过
            if matches!(method, "oauth_token" | "api_key")
                && claude_settings_declare_credential(&claude_dir)
            {
                return Ok(logged_out());
            }
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
