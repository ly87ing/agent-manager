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
