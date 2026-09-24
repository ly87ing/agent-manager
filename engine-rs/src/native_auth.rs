//! 助手官方认证的只读、脱敏状态。
//!
//! 这里只判断认证存储是否存在以及可识别的认证类型；不返回账号、token、JSON 正文，
//! 也不写文件。认证修改必须由独立的显式命令完成，Provider 切换不得调用本模块的写路径。

use serde_json::{json, Value};
use std::path::Path;

const MAX_AUTH_FILE_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Copy)]
pub(crate) enum AuthKind {
    Claude,
    Codex,
}

impl AuthKind {
    fn file_name(self) -> &'static str {
        match self {
            Self::Claude => ".credentials.json",
            Self::Codex => "auth.json",
        }
    }
}

fn recognized_mode(kind: AuthKind, value: &Value) -> Option<&'static str> {
    let object = value.as_object()?;
    match kind {
        AuthKind::Claude => {
            let oauth = object
                .get("claudeAiOauth")
                .or_else(|| object.get("claude.ai_oauth"))?
                .as_object()?;
            oauth
                .get("accessToken")
                .and_then(Value::as_str)
                .filter(|token| !token.is_empty())
                .map(|_| "oauth")
        }
        AuthKind::Codex => match object.get("auth_mode").and_then(Value::as_str) {
            Some("chatgpt") => Some("chatgpt"),
            Some("apikey") => Some("apikey"),
            Some("chatgptAuthTokens") => Some("chatgpt-auth-tokens"),
            Some("headers") => Some("headers"),
            Some("agentIdentity") => Some("agent-identity"),
            Some("personalAccessToken") => Some("personal-access-token"),
            Some("bedrockApiKey") => Some("bedrock-api-key"),
            Some("bedrockAccessKeys") => Some("bedrock-access-keys"),
            _ => None,
        },
    }
}

/// 检查一个配置目录中的官方认证文件。返回值不含正文、账号或凭据。
pub(crate) fn inspect(config_dir: &Path, kind: AuthKind) -> Value {
    let path = config_dir.join(kind.file_name());
    let file = kind.file_name();
    let metadata = match std::fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return json!({
                "state": "missing",
                "authenticated": false,
                "mode": null,
                "storage": "file",
                "file": file,
            });
        }
        Err(_) => {
            return json!({
                "state": "unreadable",
                "authenticated": false,
                "mode": null,
                "storage": "file",
                "file": file,
            });
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return json!({
            "state": "unsafe",
            "authenticated": false,
            "mode": null,
            "storage": "file",
            "file": file,
        });
    }
    if metadata.len() > MAX_AUTH_FILE_BYTES {
        return json!({
            "state": "too-large",
            "authenticated": false,
            "mode": null,
            "storage": "file",
            "file": file,
        });
    }
    let value = match std::fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
    {
        Some(value) => value,
        None => {
            return json!({
                "state": "invalid",
                "authenticated": false,
                "mode": null,
                "storage": "file",
                "file": file,
            });
        }
    };
    let mode = recognized_mode(kind, &value);
    json!({
        "state": if mode.is_some() { "ready" } else { "unrecognized" },
        "authenticated": mode.is_some(),
        "mode": mode,
        "storage": "file",
        "file": file,
    })
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

/// 该目录的 `.claude.json` 里有没有登录会话记录（官方文档：这个文件保存登录会话）。
/// 只看键是否存在，不读取内容。文件存在但解析不了时无法确认，报错。
fn claude_json_records_login(config_dir: &Path) -> Result<bool, String> {
    let path = config_dir.join(".claude.json");
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(_) => return Err("无法读取该目录的 .claude.json".into()),
    };
    let root = serde_json::from_slice::<Value>(&bytes)
        .map_err(|_| "该目录的 .claude.json 不是合法 JSON".to_string())?;
    Ok(root
        .get("oauthAccount")
        .is_some_and(|value| !value.is_null()))
}

/// 配置目录里是否存有官方账号登录。
///
/// 只经官方 CLI 的状态命令（`claude auth status` / `codex login status`）和官方文档说明的
/// 登录记录判断，不读取任何凭据内容。`Ok(true)` 有登录，`Ok(false)` 确认没有；
/// `Err` 表示无法确认，调用方应按「有」处理（拒绝写入第三方凭据）。
pub(crate) fn official_login(
    home_dir: &Path,
    kind: AuthKind,
    bin: &str,
    config_dir: &Path,
) -> Result<bool, String> {
    if !config_dir.is_dir() {
        return Ok(false);
    }
    if matches!(kind, AuthKind::Claude)
        && (config_dir.join(".credentials.json").exists() || claude_json_records_login(config_dir)?)
    {
        return Ok(true);
    }
    let cli = resolve_cli(home_dir, bin)
        .ok_or_else(|| format!("找不到 {bin} 命令，无法用官方命令确认该目录没有官方登录"))?;
    let mut command = std::process::Command::new(&cli);
    command.env("HOME", home_dir);
    match kind {
        AuthKind::Claude => {
            command
                .args(["auth", "status"])
                .env("CLAUDE_CONFIG_DIR", config_dir);
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
                return Ok(false);
            }
            let method = status
                .get("authMethod")
                .and_then(Value::as_str)
                .unwrap_or("");
            // 目录自己声明了凭据时，这两种方式来自那份凭据；登录记录已在上面查过
            let from_declared = matches!(method, "oauth_token" | "api_key")
                && claude_settings_declare_credential(config_dir);
            Ok(!from_declared)
        }
        AuthKind::Codex => {
            command
                .args(["login", "status"])
                .env("CODEX_HOME", config_dir);
            for name in CODEX_PROBE_ENV_REMOVE {
                command.env_remove(name);
            }
            // 退出码 1 也可能是出错；只有明确说「未登录」才算确认没有
            match crate::native_upstream::capture_exit(command, PROBE_TIMEOUT) {
                Some((0, _, _)) => Ok(true),
                Some((1, stdout, stderr))
                    if String::from_utf8_lossy(&stdout).contains("Not logged in")
                        || String::from_utf8_lossy(&stderr).contains("Not logged in") =>
                {
                    Ok(false)
                }
                _ => Err(format!("{bin} login status 没有给出可识别的结果")),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_status_only_exposes_redacted_shape() {
        let root = std::env::temp_dir().join(format!("agent-manager-auth-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("auth.json"),
            br#"{"auth_mode":"chatgpt","tokens":{"refresh_token":"never-expose"}}"#,
        )
        .unwrap();

        let status = inspect(&root, AuthKind::Codex);
        assert_eq!(status["state"], "ready");
        assert_eq!(status["mode"], "chatgpt");
        assert_eq!(status["authenticated"], true);
        assert!(!status.to_string().contains("never-expose"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
