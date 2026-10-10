//! 助手官方账号登录的只读判断。
//!
//! 只经官方 CLI 的状态命令（`claude auth status` / `codex login status`）和官方文档说明的
//! 登录记录判断，不读取任何凭据内容、不返回令牌、不写文件。为让用户分得清是哪个账号，会带出
//! 账号标识（只返回给界面显示，不入库、不进审计与日志，全程只读本机文件、不联网）：Claude 取
//! `.claude.json` 登录记录里的邮箱、组织名与订阅档位；Codex 取 `auth.json` 里 `id_token` 的邮箱
//! 与套餐——同一文件里的访问令牌、刷新令牌与 API key 解析时跳过，取值不进任何内存结构。认证修改只由用户在终端
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
    std::iter::once(home_dir.join(".local").join("bin"))
        .chain(
            std::env::var_os("PATH")
                .map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
                .unwrap_or_default(),
        )
        .chain(crate::native_agents::fallback_bin_dirs(home_dir))
        .flat_map(|dir| crate::platform::command_candidates(&dir, bin))
        .find(|path| path.is_file())
}

/// `.claude.json` 顶层两个登录记录键是否存在且非 null。`primaryApiKey` 的取值就是 Console 的
/// API key，解析时一律跳过（`IgnoredAny`），不进任何内存结构；`oauthAccount` 只有账号标识，
/// 从中取邮箱、组织名与订阅档位给界面显示。
#[derive(Default)]
struct ClaudeJsonLogin {
    primary_api_key: bool,
    oauth_account: bool,
    account: Option<AccountLabel>,
}

/// 账号标识：只用于界面上认出是哪个账号，不含任何凭据。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct AccountLabel {
    pub email: Option<String>,
    pub org: Option<String>,
    pub plan: Option<String>,
}

impl AccountLabel {
    fn from_oauth_account(value: &Value) -> Option<Self> {
        let text = |key: &str| {
            value
                .get(key)
                .and_then(Value::as_str)
                .filter(|v| !v.is_empty())
                .map(str::to_string)
        };
        let label = AccountLabel {
            email: text("emailAddress"),
            org: text("organizationName"),
            plan: plan_label(
                text("organizationType").as_deref(),
                text("organizationRateLimitTier").as_deref(),
            ),
        };
        (label != AccountLabel::default()).then_some(label)
    }

    fn from_auth_status(value: &Value) -> Option<Self> {
        let text = |key: &str| {
            value
                .get(key)
                .and_then(Value::as_str)
                .filter(|v| !v.is_empty())
                .map(str::to_string)
        };
        let label = AccountLabel {
            email: text("email"),
            org: text("orgName"),
            plan: text("subscriptionType").map(|plan| capitalize(&plan)),
        };
        (label != AccountLabel::default()).then_some(label)
    }
}

/// Codex `auth.json` 里只取 `tokens.id_token`；其余字段（`access_token`、`refresh_token`、
/// `OPENAI_API_KEY` 等）由 serde 按未知字段跳过，取值不保留。
#[derive(serde::Deserialize)]
struct CodexAuthFile {
    #[serde(default)]
    tokens: Option<CodexAuthTokens>,
}

#[derive(serde::Deserialize)]
struct CodexAuthTokens {
    #[serde(default)]
    id_token: Option<String>,
}

/// `auth.json` 里有没有登录项：只看 `tokens`、`OPENAI_API_KEY` 是否存在且非空，取值由
/// `IgnoredAny` 跳过、不进任何内存结构。
#[derive(serde::Deserialize)]
struct CodexLoginPresence {
    #[serde(default)]
    tokens: Option<serde::de::IgnoredAny>,
    #[serde(default, rename = "OPENAI_API_KEY")]
    api_key: Option<serde::de::IgnoredAny>,
}

/// 不调用官方命令、只看本机文件判断 Codex 登录：`Some(true)` 有登录记录，`Some(false)` 确定没有，
/// `None` 本机文件说明不了（例如登录存在钥匙串里），需要官方命令确认。
fn codex_local_login(codex_home: &Path) -> Result<Option<LoginStatus>, String> {
    if !codex_home.is_dir() {
        return Ok(Some(logged_out()));
    }
    let bytes = match std::fs::read(codex_home.join("auth.json")) {
        Ok(bytes) if bytes.len() <= 1024 * 1024 => bytes,
        Ok(_) => return Err("auth.json 过大".into()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("无法读取 auth.json".into()),
    };
    let found = serde_json::from_slice::<CodexLoginPresence>(&bytes)
        .map_err(|_| "auth.json 不是合法 JSON".to_string())?;
    Ok(Some(if found.tokens.is_some() {
        LoginStatus {
            logged_in: true,
            method: Some("chatgpt"),
            account: codex_account(codex_home),
        }
    } else if found.api_key.is_some() {
        LoginStatus {
            logged_in: true,
            method: Some("api_key"),
            account: None,
        }
    } else {
        logged_out()
    }))
}

/// JWT 载荷用的 base64url（无填充）解码；只解码 `id_token` 的中段，不校验签名（只用于显示）。
fn base64url_decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let (mut buffer, mut bits) = (0u32, 0u32);
    for byte in text.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'-' | b'+' => 62,
            b'_' | b'/' => 63,
            b'=' => break,
            _ => return None,
        };
        buffer = (buffer << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
    }
    Some(out)
}

fn chatgpt_plan_label(plan: &str) -> String {
    match plan {
        "prolite" => "Pro Lite".to_string(),
        other => capitalize(other),
    }
}

/// `<CODEX_HOME>/auth.json` 的账号标识；文件不存在（例如登录存在钥匙串里）或看不懂时为 None。
fn codex_account(codex_home: &Path) -> Option<AccountLabel> {
    let bytes = std::fs::read(codex_home.join("auth.json")).ok()?;
    if bytes.len() > 1024 * 1024 {
        return None;
    }
    let id_token = serde_json::from_slice::<CodexAuthFile>(&bytes)
        .ok()?
        .tokens?
        .id_token?;
    let claims: Value =
        serde_json::from_slice(&base64url_decode(id_token.split('.').nth(1)?)?).ok()?;
    let text = |value: Option<&Value>| {
        value
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
            .map(str::to_string)
    };
    let label = AccountLabel {
        email: text(claims.get("email")),
        org: None,
        plan: text(
            claims
                .get("https://api.openai.com/auth")
                .and_then(|auth| auth.get("chatgpt_plan_type")),
        )
        .map(|plan| chatgpt_plan_label(&plan)),
    };
    (label != AccountLabel::default()).then_some(label)
}

fn capitalize(word: &str) -> String {
    let mut chars = word.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

/// 订阅档位：`claude_max` + `default_claude_max_5x` → `Max 5x`；只有类型时取类型（`claude_pro` → `Pro`）。
fn plan_label(org_type: Option<&str>, tier: Option<&str>) -> Option<String> {
    let base = org_type.map(|kind| capitalize(kind.trim_start_matches("claude_")))?;
    let multiple = tier
        .and_then(|tier| tier.rsplit('_').next())
        .filter(|last| {
            last.ends_with('x')
                && last[..last.len() - 1].chars().all(|c| c.is_ascii_digit())
                && last.len() > 1
        });
    Some(match multiple {
        Some(multiple) => format!("{base} {multiple}"),
        None => base,
    })
}

impl<'de> serde::Deserialize<'de> for ClaudeJsonLogin {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Present(bool);
        impl<'de> serde::Deserialize<'de> for Present {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                Ok(Present(
                    Option::<serde::de::IgnoredAny>::deserialize(deserializer)?.is_some(),
                ))
            }
        }
        struct Keys;
        impl<'de> serde::de::Visitor<'de> for Keys {
            type Value = ClaudeJsonLogin;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a JSON object")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<Self::Value, A::Error> {
                let mut out = ClaudeJsonLogin::default();
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "primaryApiKey" => out.primary_api_key |= map.next_value::<Present>()?.0,
                        "oauthAccount" => {
                            let value = map.next_value::<Value>()?;
                            if !value.is_null() {
                                out.oauth_account = true;
                                out.account = AccountLabel::from_oauth_account(&value);
                            }
                        }
                        _ => {
                            map.next_value::<serde::de::IgnoredAny>()?;
                        }
                    }
                }
                Ok(out)
            }
        }
        deserializer.deserialize_map(Keys)
    }
}

/// `.claude.json` 里有没有登录记录（官方文档：这个文件保存登录会话）：`oauthAccount` 是
/// claude.ai 与 Console 登录的账号记录，`primaryApiKey` 是 Console 登录在 Linux 上保管的 key。
/// 只看键是否存在，取值在解析时就跳过。文件存在但解析不了时无法确认，报错。
///
/// 返回 `None` 表示没有登录记录，`Some(方式)` 是能从记录看出的登录方式（看不出时为 `None`）。
fn claude_json_login(
    path: &Path,
) -> Result<Option<(Option<&'static str>, Option<AccountLabel>)>, String> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("无法读取 .claude.json".into()),
    };
    let found = serde_json::from_slice::<ClaudeJsonLogin>(&bytes)
        .map_err(|_| ".claude.json 不是合法 JSON".to_string())?;
    Ok(if found.primary_api_key {
        Some((Some("console"), found.account))
    } else if found.oauth_account {
        Some((None, found.account))
    } else {
        None
    })
}

/// 官方账号登录状态：`method` 是官方命令报告的登录方式（不含账号与令牌）。
pub(crate) struct LoginStatus {
    pub logged_in: bool,
    pub method: Option<&'static str>,
    pub account: Option<AccountLabel>,
}

fn logged_out() -> LoginStatus {
    LoginStatus {
        logged_in: false,
        method: None,
        account: None,
    }
}

/// 某个配置目录里存的官方账号登录。`config_dir` 为 `None` 表示助手的主配置（不设
/// `CLAUDE_CONFIG_DIR` / 用默认 `CODEX_HOME`），`Some` 是实例目录。`Err` 表示无法确认。
///
/// `probe` 为 false 时只看本机文件、不调用官方命令：官方命令本身会写配置目录（`claude` 首次运行
/// 往 `.claude.json` 写首次启动字段并留备份，`codex` 在 `CODEX_HOME/tmp` 建临时文件），打开页面
/// 这类查询不能因此改动用户目录。本机文件说明不了时返回 `Err`，由用户点「确认」再带 probe 查。
pub(crate) fn official_status(
    home_dir: &Path,
    kind: AuthKind,
    bin: &str,
    config_dir: Option<&Path>,
    probe: bool,
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
        if let Some((method, account)) = claude_json_login(&claude_json)? {
            return Ok(LoginStatus {
                logged_in: true,
                method,
                account,
            });
        }
        if !probe {
            // 从没运行过（没有 .claude.json）就一定没登录；有文件但没有登录记录时，可能是托管设置里的
            // 长期令牌等官方命令才看得到的来源，交给用户确认
            return if claude_json.exists() {
                Err("本机登录记录里没有账号；需要时用官方命令确认".into())
            } else {
                Ok(logged_out())
            };
        }
    }
    if matches!(kind, AuthKind::Codex) && !probe {
        let codex_home = config_dir.map_or_else(|| home_dir.join(".codex"), Path::to_path_buf);
        return codex_local_login(&codex_home)?.ok_or_else(|| {
            "本机没有 auth.json（登录可能存在钥匙串里）；需要时用官方命令确认".into()
        });
    }
    let cli = resolve_cli(home_dir, bin)
        .ok_or_else(|| format!("找不到 {bin} 命令，无法用官方命令确认登录状态"))?;
    let mut command = crate::process::command(&cli);
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
                account: AccountLabel::from_auth_status(&status),
            })
        }
        AuthKind::Codex => {
            let codex_home = config_dir.map_or_else(|| home_dir.join(".codex"), Path::to_path_buf);
            command
                .args(["login", "status"])
                .env("CODEX_HOME", &codex_home);
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
                        account: (method == "chatgpt")
                            .then(|| codex_account(&codex_home))
                            .flatten(),
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
    // 写入前的核对（用户已发起操作）：本机文件说明不了时用官方命令确认
    official_status(home_dir, kind, bin, Some(config_dir), true).map(|status| status.logged_in)
}

/// `agents:auth-status --only <agents:助手 | profiles:实例>`：用官方命令查一次登录状态。
///
/// 只读：不写库、不取锁、不读取凭据内容，只返回是否登录与登录方式。第三方实例里有官方登录时
/// 标为 `mixed`（两种凭据混在同一个配置目录）。默认只看本机文件；带 `--probe` 才调用官方命令
/// （一次要一两秒，且官方命令自己会写配置目录），只在用户点「确认」时带上。
pub(crate) fn status_command(
    ctx: &crate::native::Context,
    req: &crate::native::Request,
) -> anyhow::Result<Value> {
    let mut only = None;
    let mut probe = false;
    let mut iter = req.args.iter();
    while let Some(arg) = iter.next() {
        match (arg.as_str(), only.is_some()) {
            ("--only", false) => only = iter.next().cloned(),
            ("--probe", _) => probe = true,
            _ => {
                anyhow::bail!("agents:auth-status 只接受一个 --only <agents:助手 | profiles:实例> 与可选的 --probe")
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
    let result = official_status(&ctx.home_dir, kind, &bin, config_dir.as_deref(), probe);
    Ok(match result {
        Ok(status) => json!({
            "element": only,
            "state": if status.logged_in { "logged-in" } else { "logged-out" },
            "logged_in": status.logged_in,
            "method": status.method,
            "account": status.account.map(|account| json!({
                "email": account.email,
                "org": account.org,
                "plan": account.plan,
            })),
            "third_party": third_party,
            "mixed": third_party && status.logged_in,
            "reason": null,
        }),
        Err(reason) => json!({
            "element": only,
            "state": "unknown",
            "logged_in": null,
            "method": null,
            "account": null,
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
    // 换了配置目录：主配置 ~/.claude 与 ~/.claude.json 里的设置与登录都不生效
    "CLAUDE_CONFIG_DIR",
];
/// Codex：`CODEX_ACCESS_TOKEN` 所有入口都读、`CODEX_API_KEY` 由 `codex exec` 等非交互入口读，
/// 都优先于 `auth.json` 里的登录（`rust-v0.156.1` `login/src/auth/manager.rs`）。
/// 另有 `CODEX_HOME`：换了目录，主配置 ~/.codex 里的设置与登录都不生效。
const CODEX_OVERRIDE_ENV: &[&str] = &["CODEX_API_KEY", "CODEX_ACCESS_TOKEN", "CODEX_HOME"];
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
/// Codex `config.toml` 里改变官方账号凭据去向或限定登录的键（官方配置参考与 `rust-v0.156.1`
/// 源码）：内置 openai 与 ChatGPT 后端的地址（凭据随请求发往那里）；限定登录方式与工作区——不符的
/// 登录会被拒绝，已有的在 `codex exec` 与交互界面启动时被 Codex 退出（实测会删掉 `auth.json`）。
const CODEX_CONFIG_AUTH_KEYS: &[&str] = &[
    "openai_base_url",
    "chatgpt_base_url",
    "forced_login_method",
    "forced_chatgpt_workspace_id",
];
/// Codex 系统 `requirements.toml` 里限定登录的键（`config/src/config_requirements.rs`）。
const CODEX_REQUIREMENTS_AUTH_KEYS: &[&str] = &[
    "chatgpt_base_url",
    "allowed_login_methods",
    "allowed_chatgpt_workspaces",
];
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

/// 读一个不太大的 TOML 文件；同 `read_small_json`，读不了就当作没有。
fn read_small_toml(path: &Path) -> Option<toml::Table> {
    std::fs::metadata(path)
        .ok()
        .filter(|meta| meta.is_file() && meta.len() <= MAX_RC_BYTES)?;
    std::fs::read_to_string(path).ok()?.parse().ok()
}

/// TOML 取值是否真的设了（非空字符串、非空数组或表）。
fn toml_set(value: Option<&toml::Value>) -> bool {
    match value {
        Some(toml::Value::String(text)) => !text.trim().is_empty(),
        Some(toml::Value::Array(items)) => !items.is_empty(),
        Some(toml::Value::Table(table)) => !table.is_empty(),
        Some(_) => true,
        None => false,
    }
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

/// Codex 的系统级配置目录（`rust-v0.156.1` `config/src/loader`：Unix 固定 `/etc/codex`，Windows 为
/// `%ProgramData%\OpenAI\Codex`）。macOS 描述文件下发的设置不在这里看。
fn codex_system_dirs() -> Vec<std::path::PathBuf> {
    #[cfg(unix)]
    let dirs = vec![std::path::PathBuf::from("/etc/codex")];
    #[cfg(windows)]
    let dirs = vec![std::path::PathBuf::from("C:\\ProgramData\\OpenAI\\Codex")];
    dirs
}

fn system_dirs(kind: AuthKind) -> Vec<std::path::PathBuf> {
    match kind {
        AuthKind::Claude => claude_managed_settings_dirs(),
        AuthKind::Codex => codex_system_dirs(),
    }
}

/// Claude 托管设置里会压过凭据或限定登录的项。托管设置优先级最高，主配置与每个独立环境都挡不住。
fn claude_managed_overrides(dirs: &[std::path::PathBuf]) -> Vec<Value> {
    let mut out = Vec::new();
    for path in claude_managed_settings_files(dirs) {
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
    out
}

/// Codex 系统级文件里会压过（或限定）某份 `config.toml` 的项。`user` 是这份配置（主配置或实例的
/// `config.toml`）；`official` 表示它用官方账号。按 `rust-v0.156.1` 的分层：
/// - 系统 `config.toml` 排在这份配置之下，只有这份没写的键才生效；
/// - 旧版 `managed_config.toml`（仅 Unix）叠在最上层，照样生效；
/// - `requirements.toml` 的 `model_provider` 强制所有配置目录改用它指定的接入方式，
///   `model_providers` 里同名的定义整个替换掉本地的。
///
/// 实测（真实 0.156.1）：`managed_config.toml` 的 `openai_base_url` 压过用户配置、官方凭据发往它；
/// `requirements.toml` 的 `model_provider` 让实例的第三方接入改走它指定的地址。
fn codex_system_overrides(
    dirs: &[std::path::PathBuf],
    user: Option<&toml::Table>,
    official: bool,
) -> Vec<Value> {
    let own_provider = user
        .and_then(|doc| doc.get("model_provider"))
        .and_then(toml::Value::as_str)
        .unwrap_or("openai");
    let user_sets = |key: &str| user.is_some_and(|doc| toml_set(doc.get(key)));
    let forced_provider = |doc: &toml::Table| {
        doc.get("model_provider")
            .and_then(toml::Value::as_str)
            .is_some_and(|provider| !provider.trim().is_empty() && provider != own_provider)
    };
    let mut out = Vec::new();
    for dir in dirs {
        let system = dir.join("config.toml");
        if let Some(doc) = read_small_toml(&system) {
            let source = system.display().to_string();
            if !user_sets("model_provider") && forced_provider(&doc) {
                out.push(json!({"source": source, "name": "model_provider"}));
            }
            if official {
                for key in CODEX_CONFIG_AUTH_KEYS {
                    if toml_set(doc.get(*key)) && !user_sets(key) {
                        out.push(json!({"source": source, "name": key}));
                    }
                }
            }
        }
        #[cfg(unix)]
        {
            let managed = dir.join("managed_config.toml");
            if let Some(doc) = read_small_toml(&managed) {
                let source = managed.display().to_string();
                if forced_provider(&doc) {
                    out.push(json!({"source": source, "name": "model_provider"}));
                }
                if official {
                    for key in CODEX_CONFIG_AUTH_KEYS {
                        if toml_set(doc.get(*key)) {
                            out.push(json!({"source": source, "name": key}));
                        }
                    }
                }
            }
        }
        let requirements = dir.join("requirements.toml");
        if let Some(doc) = read_small_toml(&requirements) {
            let source = requirements.display().to_string();
            if forced_provider(&doc) {
                out.push(json!({"source": source, "name": "model_provider"}));
            }
            if doc
                .get("model_providers")
                .and_then(toml::Value::as_table)
                .is_some_and(|providers| providers.contains_key(own_provider))
            {
                out.push(json!({"source": source, "name": "model_providers"}));
            }
            if official {
                for key in CODEX_REQUIREMENTS_AUTH_KEYS {
                    if toml_set(doc.get(*key)) {
                        out.push(json!({"source": source, "name": key}));
                    }
                }
            }
        }
    }
    out
}

/// 独立环境挡不住的系统级设置：`[{source, name}]`，只有来源与键名，不含取值；只读。
///
/// 实例在自己的配置里取消了 shell 继承的凭据（Claude 置空、Codex 启动前清空），但系统级设置对
/// 每个配置目录都生效：Claude 托管设置优先级最高，实测其 `env` 压过实例 `settings.json`、
/// `forceLoginMethod` 让用第三方凭据的实例启动即被拦下；Codex 见 `codex_system_overrides`。
/// 官方账号的 Claude 实例另报共享的 Anthropic 联合身份档案（不随配置目录隔离、排在 /login 之前）。
pub(crate) fn instance_system_overrides(
    home_dir: &Path,
    kind: AuthKind,
    official: bool,
    instance_config: Option<&[u8]>,
) -> Vec<Value> {
    instance_system_overrides_with(
        home_dir,
        kind,
        official,
        instance_config,
        &system_dirs(kind),
    )
}

fn instance_system_overrides_with(
    home_dir: &Path,
    kind: AuthKind,
    official: bool,
    instance_config: Option<&[u8]>,
    dirs: &[std::path::PathBuf],
) -> Vec<Value> {
    match kind {
        AuthKind::Claude => {
            let mut out = claude_managed_overrides(dirs);
            if official {
                if let Some(source) = anthropic_federation_profile(home_dir) {
                    out.push(json!({"source": source, "name": "oidc_federation"}));
                }
            }
            out
        }
        AuthKind::Codex => {
            let mut user = instance_config
                .and_then(|bytes| std::str::from_utf8(bytes).ok())
                .and_then(|text| text.parse::<toml::Table>().ok());
            if !official {
                // 第三方实例写入后用自己的接入方式；文件还没写时按将要写的判断
                user.get_or_insert_with(toml::Table::new)
                    .entry("model_provider")
                    .or_insert_with(|| {
                        toml::Value::String(
                            crate::native_profiles::model::CODEX_PROVIDER_ID.to_string(),
                        )
                    });
            }
            codex_system_overrides(dirs, user.as_ref(), official)
        }
    }
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
    credential_overrides_with(home_dir, kind, env_set, &system_dirs(kind))
}

fn credential_overrides_with(
    home_dir: &Path,
    kind: AuthKind,
    env_set: &dyn Fn(&str) -> bool,
    dirs: &[std::path::PathBuf],
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
            out.extend(claude_managed_overrides(dirs));
            if let Some(source) = anthropic_federation_profile(home_dir) {
                out.push(json!({"source": source, "name": "oidc_federation"}));
            }
        }
        AuthKind::Codex => {
            // 主配置自己写的地址覆盖与登录限定（官方配置参考）：官方账号的凭据会随请求发往这些
            // 地址；限定登录的键会让另一种登录被拒绝或在启动时被退出
            let config = read_small_toml(&home_dir.join(".codex/config.toml"));
            for key in CODEX_CONFIG_AUTH_KEYS {
                if toml_set(config.as_ref().and_then(|doc| doc.get(*key))) {
                    out.push(json!({"source": "~/.codex/config.toml", "name": key}));
                }
            }
            out.extend(codex_system_overrides(dirs, config.as_ref(), true));
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
            .join("managed-settings.d")
            .join("10-helper.json")
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

    /// 账号标识取自 oauthAccount：邮箱、组织名、订阅档位；Console 的 API key 取值不进结果。
    #[test]
    fn claude_json_login_labels_the_account_without_the_console_key() {
        let dir = std::env::temp_dir().join(format!("am-auth-label-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(".claude.json");
        std::fs::write(
            &path,
            r#"{"primaryApiKey":"sk-ant-api03-secret","oauthAccount":{"emailAddress":"YOUR_EMAIL@example.com","organizationName":"YOUR_ORG","organizationType":"claude_max","organizationRateLimitTier":"default_claude_max_20x","accountUuid":"u"}}"#,
        )
        .unwrap();
        let (method, account) = claude_json_login(&path).unwrap().unwrap();
        assert_eq!(method, Some("console"));
        let account = account.unwrap();
        assert_eq!(account.email.as_deref(), Some("YOUR_EMAIL@example.com"));
        assert_eq!(account.org.as_deref(), Some("YOUR_ORG"));
        assert_eq!(account.plan.as_deref(), Some("Max 20x"));
        assert!(!format!("{account:?}").contains("sk-ant"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn base64url(bytes: &[u8]) -> String {
        const TABLE: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let n = chunk.iter().fold(0u32, |acc, b| (acc << 8) | u32::from(*b))
                << (8 * (3 - chunk.len()));
            for i in 0..=chunk.len() {
                out.push(TABLE[((n >> (18 - 6 * i)) & 63) as usize] as char);
            }
        }
        out
    }

    /// Codex 账号标识只取 id_token 里的邮箱与套餐；访问令牌、刷新令牌与 API key 的取值不进结果。
    #[test]
    fn codex_account_reads_only_the_id_token_claims() {
        let dir = std::env::temp_dir().join(format!("am-auth-codex-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let payload = json!({
            "email": "YOUR_EMAIL@example.com",
            "https://api.openai.com/auth": {"chatgpt_plan_type": "prolite", "chatgpt_account_id": "a"}
        });
        let id_token = format!(
            "{}.{}.sig",
            base64url(br#"{"alg":"RS256"}"#),
            base64url(payload.to_string().as_bytes())
        );
        std::fs::write(
            dir.join("auth.json"),
            json!({
                "auth_mode": "chatgpt",
                "OPENAI_API_KEY": "sk-proj-SECRET_KEY",
                "tokens": {"id_token": id_token, "access_token": "SECRET_ACCESS", "refresh_token": "SECRET_REFRESH", "account_id": "a"}
            })
            .to_string(),
        )
        .unwrap();
        let account = codex_account(&dir).unwrap();
        assert_eq!(account.email.as_deref(), Some("YOUR_EMAIL@example.com"));
        assert_eq!(account.plan.as_deref(), Some("Pro Lite"));
        let shown = format!("{account:?}");
        for secret in ["SECRET_ACCESS", "SECRET_REFRESH", "SECRET_KEY"] {
            assert!(!shown.contains(secret), "{shown}");
        }

        // 登录存在钥匙串里（没有 auth.json）或内容看不懂：认不出，不报错
        std::fs::write(
            dir.join("auth.json"),
            r#"{"tokens":{"id_token":"not-a-jwt"}}"#,
        )
        .unwrap();
        assert_eq!(codex_account(&dir), None);
        std::fs::remove_file(dir.join("auth.json")).unwrap();
        assert_eq!(codex_account(&dir), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn plan_labels_follow_type_and_rate_tier() {
        assert_eq!(
            plan_label(Some("claude_max"), Some("default_claude_max_5x")).as_deref(),
            Some("Max 5x")
        );
        assert_eq!(
            plan_label(Some("claude_pro"), Some("default_claude_ai")).as_deref(),
            Some("Pro")
        );
        assert_eq!(
            plan_label(Some("claude_team"), None).as_deref(),
            Some("Team")
        );
        assert_eq!(plan_label(None, Some("default_claude_max_5x")), None);
        assert_eq!(
            AccountLabel::from_auth_status(
                &json!({"email": "YOUR_EMAIL@example.com", "orgName": "YOUR_ORG", "subscriptionType": "max"})
            ),
            Some(AccountLabel {
                email: Some("YOUR_EMAIL@example.com".into()),
                org: Some("YOUR_ORG".into()),
                plan: Some("Max".into())
            })
        );
    }

    /// 登录记录只看顶层键在不在（非 null）；嵌套里同名的键不算。
    #[test]
    fn claude_json_login_looks_only_at_top_level_keys() {
        let dir = std::env::temp_dir().join(format!("am-auth-cj-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(".claude.json");
        let check = |text: &str| {
            std::fs::write(&path, text).unwrap();
            claude_json_login(&path).map(|found| found.map(|(method, _)| method))
        };
        assert_eq!(
            check(r#"{"primaryApiKey":"sk-ant-api03-secret","oauthAccount":{"a":1}}"#),
            Ok(Some(Some("console")))
        );
        assert_eq!(
            check(r#"{"oauthAccount":{"emailAddress":"x"}}"#),
            Ok(Some(None))
        );
        assert_eq!(
            check(r#"{"oauthAccount":null,"primaryApiKey":null}"#),
            Ok(None)
        );
        assert_eq!(
            check(
                r#"{"mcpServers":{"x":{"primaryApiKey":"k","oauthAccount":{}}},"projects":[1,{"oauthAccount":1}]}"#
            ),
            Ok(None)
        );
        assert!(check("{not json").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// shell 启动文件导出了配置目录：助手改用那个目录，主配置不生效。本工具写的启动命令是别名
    /// （`alias 名='CLAUDE_CONFIG_DIR=… claude'`），不算导出。
    #[test]
    fn shell_exported_config_dir_is_reported_but_launch_aliases_are_not() {
        let home = std::env::temp_dir().join(format!("am-auth-dir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(
            home.join(".zprofile"),
            "export CLAUDE_CONFIG_DIR=\"$HOME/.claude-work\"\nCODEX_HOME=/opt/codex-home\nalias cc-work='CLAUDE_CONFIG_DIR=\"$HOME/.claude-w\" claude'\nalias cx='CODEX_API_KEY= CODEX_ACCESS_TOKEN= CODEX_HOME=\"$HOME/.codex-w\" codex'\n",
        )
        .unwrap();
        assert_eq!(
            credential_overrides_with(&home, AuthKind::Claude, &|_| false, &[]),
            vec![json!({"source": "~/.zprofile", "name": "CLAUDE_CONFIG_DIR"})]
        );
        assert_eq!(
            credential_overrides_with(&home, AuthKind::Codex, &|_| false, &[]),
            vec![json!({"source": "~/.zprofile", "name": "CODEX_HOME"})]
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    fn pairs(items: &[Value]) -> Vec<(String, String)> {
        items
            .iter()
            .map(|item| {
                (
                    item["source"].as_str().unwrap().to_string(),
                    item["name"].as_str().unwrap().to_string(),
                )
            })
            .collect()
    }

    /// Codex 系统级文件按 0.156.1 的分层提示：系统 config.toml 只在主配置没写该键时生效，
    /// managed_config.toml 叠在最上层，requirements.toml 强制接入方式与限定登录；主配置自己写的
    /// 地址覆盖与登录限定也提示。只报来源与键名。
    #[test]
    fn codex_主配置提示系统级文件与自己写的登录限定_按分层判断谁生效() {
        let home = std::env::temp_dir().join(format!("am-auth-codex-sys-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let etc = home.join("etc-codex");
        std::fs::create_dir_all(&etc).unwrap();
        std::fs::create_dir_all(home.join(".codex")).unwrap();
        std::fs::write(
            home.join(".codex/config.toml"),
            "openai_base_url = \"https://user.example/v1\"\nchatgpt_base_url = \"https://chat.example\"\nforced_login_method = \"api\"\nforced_chatgpt_workspace_id = [\"ws-secret\"]\n",
        )
        .unwrap();
        std::fs::write(
            etc.join("config.toml"),
            "model_provider = \"orgp\"\nopenai_base_url = \"https://system.example/v1\"\nforced_login_method = \"chatgpt\"\n",
        )
        .unwrap();
        std::fs::write(
            etc.join("managed_config.toml"),
            "openai_base_url = \"https://managed.example/v1\"\n",
        )
        .unwrap();
        std::fs::write(
            etc.join("requirements.toml"),
            "model_provider = \"openai\"\nallowed_login_methods = [\"chatgpt\"]\nallowed_chatgpt_workspaces = []\n",
        )
        .unwrap();

        let codex = credential_overrides_with(&home, AuthKind::Codex, &|_| false, &[etc.clone()]);
        let system = etc.join("config.toml").display().to_string();
        let managed = etc.join("managed_config.toml").display().to_string();
        let requirements = etc.join("requirements.toml").display().to_string();
        let user = "~/.codex/config.toml".to_string();
        assert_eq!(
            pairs(&codex),
            vec![
                (user.clone(), "openai_base_url".to_string()),
                (user.clone(), "chatgpt_base_url".to_string()),
                (user.clone(), "forced_login_method".to_string()),
                (user, "forced_chatgpt_workspace_id".to_string()),
                // 主配置没写 model_provider：系统 config.toml 的生效；地址与登录限定被主配置自己的压住
                (system, "model_provider".to_string()),
            ]
            .into_iter()
            // managed_config.toml 只在 Unix 上生效（Codex 本身如此）
            .chain(cfg!(unix).then(|| (managed, "openai_base_url".to_string())))
            // 强制成 openai 对主配置的官方账号无碍，不提示；空数组不算设了
            .chain([(requirements, "allowed_login_methods".to_string())])
            .collect::<Vec<_>>()
        );
        let text = format!("{codex:?}");
        for value in [
            "user.example",
            "chat.example",
            "ws-secret",
            "system.example",
            "managed.example",
            "orgp",
        ] {
            assert!(!text.contains(value), "不得带出取值 {value}");
        }
        let _ = std::fs::remove_dir_all(&home);
    }

    /// 独立环境挡不住系统级设置：第三方实例只关心强制换掉它接入方式的项与压过它 env 的托管设置；
    /// 官方账号实例另看限定登录的项与共享的联合身份档案。
    #[test]
    fn 实例提示挡不住的系统级设置_按第三方与官方账号区分() {
        let home = std::env::temp_dir().join(format!("am-auth-inst-sys-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let etc = home.join("etc-codex");
        std::fs::create_dir_all(&etc).unwrap();
        std::fs::write(
            etc.join("config.toml"),
            "model_provider = \"orgp\"\nforced_login_method = \"chatgpt\"\n",
        )
        .unwrap();
        std::fs::write(
            etc.join("requirements.toml"),
            "model_provider = \"openai\"\nallowed_login_methods = [\"chatgpt\"]\n[model_providers.agent-manager]\nname = \"org\"\nbase_url = \"https://org.example/v1\"\n",
        )
        .unwrap();
        let system = etc.join("config.toml").display().to_string();
        let requirements = etc.join("requirements.toml").display().to_string();

        let relay = b"model_provider = \"agent-manager\"\n[model_providers.agent-manager]\nname = \"agent-manager\"\nbase_url = \"https://relay.example/v1\"\nexperimental_bearer_token = \"tok\"\n";
        let third_party = instance_system_overrides_with(
            &home,
            AuthKind::Codex,
            false,
            Some(relay),
            &[etc.clone()],
        );
        assert_eq!(
            pairs(&third_party),
            vec![
                (requirements.clone(), "model_provider".to_string()),
                (requirements.clone(), "model_providers".to_string()),
            ],
            "实例自己写了 model_provider，系统 config.toml 的不生效；登录限定与第三方实例无关"
        );

        // 文件还没写：按将要写的接入方式判断，照样提示
        let not_written =
            instance_system_overrides_with(&home, AuthKind::Codex, false, None, &[etc.clone()]);
        assert_eq!(pairs(&not_written), pairs(&third_party));

        let official = instance_system_overrides_with(
            &home,
            AuthKind::Codex,
            true,
            Some(b"model = \"gpt-5.5\"\n"),
            &[etc.clone()],
        );
        assert_eq!(
            pairs(&official),
            vec![
                (system.clone(), "model_provider".to_string()),
                (system, "forced_login_method".to_string()),
                (requirements, "allowed_login_methods".to_string()),
            ],
            "官方账号实例用内置 openai：强制成 openai 不提示"
        );

        let managed = home.join("managed");
        std::fs::create_dir_all(&managed).unwrap();
        std::fs::write(
            managed.join("managed-settings.json"),
            r#"{"forceLoginMethod":"claudeai","env":{"ANTHROPIC_BASE_URL":"https://gw.corp.example"}}"#,
        )
        .unwrap();
        std::fs::create_dir_all(home.join(".config/anthropic/configs")).unwrap();
        std::fs::write(
            home.join(".config/anthropic/configs/default.json"),
            r#"{"authentication":{"type":"oidc_federation"}}"#,
        )
        .unwrap();
        let managed_file = managed.join("managed-settings.json").display().to_string();
        let claude_relay = instance_system_overrides_with(
            &home,
            AuthKind::Claude,
            false,
            None,
            &[managed.clone()],
        );
        assert_eq!(
            pairs(&claude_relay),
            vec![
                (managed_file.clone(), "ANTHROPIC_BASE_URL".to_string()),
                (managed_file.clone(), "forceLoginMethod".to_string()),
            ]
        );
        let claude_official =
            instance_system_overrides_with(&home, AuthKind::Claude, true, None, &[managed]);
        assert_eq!(
            pairs(&claude_official).last(),
            Some(&(
                "~/.config/anthropic/configs/default.json".to_string(),
                "oidc_federation".to_string()
            ))
        );
        let text = format!("{third_party:?}{official:?}{claude_relay:?}{claude_official:?}");
        for value in ["org.example", "relay.example", "gw.corp.example", "orgp"] {
            assert!(!text.contains(value), "不得带出取值 {value}");
        }
        let _ = std::fs::remove_dir_all(&home);
    }

    /// 桌面应用的 PATH 里没有 npm 全局目录：命令装在那里也要找得到，且它要靠同目录的
    /// 解释器（npm 装的助手是 node 脚本）才能跑起来。
    #[test]
    #[cfg(unix)]
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
        let status = official_status(&home, AuthKind::Codex, &cli, Some(&dir), true);
        assert_eq!(status.map(|s| s.logged_in), Ok(false));
        let _ = std::fs::remove_dir_all(&home);
    }
}
