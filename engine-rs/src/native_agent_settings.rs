//! 助手的运行参数：接入配置（地址、认证、模型）、权限与客户端运行偏好。
//! 写进助手自己的配置文件，因此 IDE、别的终端、任何入口启动它都一样生效。
//!
//! **实际配置为准**：助手的实际文件才是当前生效的配置。status 从实际文件读出当前值、
//! 写入基准 `base` 与按内容识别的当前接入；库里只存已保存的接入配置（备选），`active`
//! 与运行参数副本只是「上次写入」的记录，不再当作当前值展示或预填。
//!
//! 写入分三种，互不越界：`--runtime` 只改补丁里那几项运行参数的键；`--use` /
//! `--use-default` 只改接入键，切换前先把还没保存的实际接入回填进库（身份与上次写入的那套
//! 一致就更新它，否则新建导入项）；`--save-current` 只回填、不写文件。每次写入都要带
//! status 给出的 `base`，重读后对不上就拒绝，文件与库都不动。
//!
//! 密钥语义与实例、MCP 一致：库里只存变量名，写目标文件时解析成真值。
//! `settings.json` 的 `env` 是字面量映射，Claude Code 不对它做变量展开，
//! 引用写进去不生效；落地文件里是明文，与 MCP 写助手配置同级，由文件权限保护。
//! 回填遇到没保存过的明文令牌时，只写进用户选择的外部密钥文件，库里只留变量名。
//!
//! 受管粒度按「键路径」而不是整个顶层键：`~/.claude/settings.json` 里还住着 hooks 域
//! 和用户自己的 permissions.allow/deny、别的 env 变量，整键归属会互相覆盖。

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{bail, Result};
use serde_json::{Map, Value};

use crate::native::Context;
pub(crate) use crate::native_agent_adapters::AdapterKind as Kind;
use crate::native_agent_adapters::{
    kind_for_bin, CLAUDE_EFFORT_LEVELS, CODEX_REASONING_EFFORTS, CODEX_WEB_SEARCH_MODES,
};

/// Claude 的目标文件，相对 home。
pub(crate) const CLAUDE_SETTINGS: &str = ".claude/settings.json";

/// 本工具维护的状态栏脚本，落在数据目录里（不放助手目录，那是助手的地盘）。
pub(crate) const STATUSLINE_FILE: &str = "statusline.sh";

/// 内置状态栏正文：显示模型/目录/上下文/额度，并把额度存一份供本工具读。
const STATUSLINE_BODY: &str = include_str!("../assets/statusline.sh");

/// 本工具在 settings.json 里拥有的键路径。其余一律是别人的，原样保留。
const CLAUDE_OWNED_PATHS: &[&[&str]] = &[
    &["env", "ANTHROPIC_BASE_URL"],
    &["env", "ANTHROPIC_AUTH_TOKEN"],
    &["env", "ANTHROPIC_API_KEY"],
    &["env", "ANTHROPIC_MODEL"],
    &["env", "ANTHROPIC_SMALL_FAST_MODEL"],
    &["env", "ANTHROPIC_DEFAULT_OPUS_MODEL"],
    &["env", "ANTHROPIC_DEFAULT_SONNET_MODEL"],
    &["env", "ANTHROPIC_DEFAULT_HAIKU_MODEL"],
    &["env", "API_TIMEOUT_MS"],
    &["env", "ENABLE_TOOL_SEARCH"],
    &["env", "CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS"],
    &["env", "CLAUDE_CODE_EFFORT_LEVEL"],
    &["env", "CLAUDE_CODE_DISABLE_ARTIFACT"],
    &["permissions", "defaultMode"],
    &["attribution", "commit"],
    &["attribution", "pr"],
];

/// 其中属于接入的键：切换接入只动它们，识别当前接入也只比它们。
const CLAUDE_CONNECTION_PATHS: &[&[&str]] = &[
    &["env", "ANTHROPIC_BASE_URL"],
    &["env", "ANTHROPIC_AUTH_TOKEN"],
    &["env", "ANTHROPIC_API_KEY"],
    &["env", "ANTHROPIC_MODEL"],
    &["env", "ANTHROPIC_SMALL_FAST_MODEL"],
    &["env", "ANTHROPIC_DEFAULT_OPUS_MODEL"],
    &["env", "ANTHROPIC_DEFAULT_SONNET_MODEL"],
    &["env", "ANTHROPIC_DEFAULT_HAIKU_MODEL"],
    &["env", "API_TIMEOUT_MS"],
];

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AutomationMode {
    #[default]
    Manual,
    Auto,
    Full,
}

impl AutomationMode {
    fn parse(obj: &Map<String, Value>) -> Result<Self> {
        let legacy_full = obj.get("yolo").and_then(Value::as_bool).unwrap_or(false);
        let mode = match non_empty(obj.get("automation_mode")).as_deref() {
            None => {
                if legacy_full {
                    Self::Full
                } else {
                    Self::Manual
                }
            }
            Some("manual") => Self::Manual,
            Some("auto") => Self::Auto,
            Some("full") => Self::Full,
            Some(other) => {
                bail!("settings.automation_mode 只能是 manual / auto / full，收到 {other}")
            }
        };
        if legacy_full && mode != Self::Full {
            bail!("settings.yolo=true 与 settings.automation_mode 冲突");
        }
        Ok(mode)
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Auto => "auto",
            Self::Full => "full",
        }
    }
}

const MAX_CLIENT_CONFIG_BYTES: u64 = 4 * 1024 * 1024;

/// 读客户端配置文件。符号链接（dotfiles 仓库常见）按最终目标读，目标必须是普通文件；
/// 断链、目录与超限一律按固定状态返回，不读正文。
fn read_observed_config(
    path: &std::path::Path,
) -> std::result::Result<Option<Vec<u8>>, &'static str> {
    let link = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("unreadable"),
    };
    let metadata = if link.file_type().is_symlink() {
        std::fs::metadata(path).map_err(|_| "unsafe")?
    } else {
        link
    };
    if !metadata.is_file() {
        return Err("unsafe");
    }
    if metadata.len() > MAX_CLIENT_CONFIG_BYTES {
        return Err("too-large");
    }
    std::fs::read(path).map(Some).map_err(|_| "unreadable")
}

fn string_at<'a>(value: &'a Value, path: &[&str]) -> Option<&'a str> {
    path.iter()
        .try_fold(value, |current, key| current.get(*key))?
        .as_str()
        .filter(|value| !value.is_empty())
}

fn claude_observed_config(
    bytes: &[u8],
    statusline_path: Option<&std::path::Path>,
) -> Option<Value> {
    let root = serde_json::from_slice::<Value>(bytes)
        .ok()?
        .as_object()?
        .clone();
    let value = Value::Object(root);
    let credential_kind = if string_at(&value, &["env", "ANTHROPIC_AUTH_TOKEN"]).is_some() {
        Some("token")
    } else if string_at(&value, &["env", "ANTHROPIC_API_KEY"]).is_some() {
        Some("api_key")
    } else {
        None
    };
    let statusline = path_get(&value, &["statusLine", "command"])
        .and_then(Value::as_str)
        .map(|command| {
            match statusline_path.and_then(|path| builtin_statusline_form(command, path)) {
                Some(StatuslineForm::Quoted) => "builtin",
                // 旧版写的裸路径：仍是我们的，但不能报成与期望一致，否则用户不会重新应用
                Some(StatuslineForm::Legacy) => "builtin-legacy",
                None => "custom",
            }
        });
    // 三档预设之外的取值（acceptEdits、plan 等）如实报 custom 并给出原值：表单据此预填，不改就不动
    let (automation_mode, automation_raw) = match string_at(&value, &["permissions", "defaultMode"])
    {
        None | Some("manual") => ("manual", Value::Null),
        Some("auto") => ("auto", Value::Null),
        Some("bypassPermissions") => ("full", Value::Null),
        Some(other) => ("custom", json!({"permissions.defaultMode": other})),
    };
    let hide_attribution = path_get(&value, &["attribution", "commit"]).and_then(Value::as_str)
        == Some("")
        && path_get(&value, &["attribution", "pr"]).and_then(Value::as_str) == Some("");
    Some(json!({
        "provider": "anthropic",
        "base_url": string_at(&value, &["env", "ANTHROPIC_BASE_URL"]),
        "model": string_at(&value, &["env", "ANTHROPIC_MODEL"]),
        "small_fast_model": string_at(&value, &["env", "ANTHROPIC_SMALL_FAST_MODEL"]),
        "model_aliases": {
            "opus": string_at(&value, &["env", "ANTHROPIC_DEFAULT_OPUS_MODEL"]),
            "sonnet": string_at(&value, &["env", "ANTHROPIC_DEFAULT_SONNET_MODEL"]),
            "haiku": string_at(&value, &["env", "ANTHROPIC_DEFAULT_HAIKU_MODEL"])
        },
        "timeout_ms": string_at(&value, &["env", "API_TIMEOUT_MS"]),
        "credential": {"configured": credential_kind.is_some(), "kind": credential_kind, "variable": null},
        "runtime": {
            "automation_mode": automation_mode,
            "automation_raw": automation_raw,
            "statusline": statusline,
            "claude_effort_level": string_at(&value, &["env", "CLAUDE_CODE_EFFORT_LEVEL"]),
            "claude_tool_search": string_at(&value, &["env", "ENABLE_TOOL_SEARCH"]) == Some("true"),
            "claude_agent_teams": string_at(&value, &["env", "CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS"]) == Some("1"),
            "claude_disable_artifact": string_at(&value, &["env", "CLAUDE_CODE_DISABLE_ARTIFACT"]) == Some("1"),
            "claude_hide_attribution": hide_attribution
        }
    }))
}

fn codex_observed_config(bytes: &[u8]) -> Option<Value> {
    let text = std::str::from_utf8(bytes).ok()?;
    let root = text.parse::<toml::Value>().ok()?;
    let provider = root.get("model_provider").and_then(toml::Value::as_str);
    let provider_config = provider
        .and_then(|name| root.get("model_providers")?.get(name))
        .and_then(toml::Value::as_table);
    let env_key = provider_config
        .and_then(|table| table.get("env_key"))
        .and_then(toml::Value::as_str)
        .filter(|value| !value.is_empty());
    let text_key = |key: &str| root.get(key).and_then(toml::Value::as_str);
    let (approval, sandbox, reviewer) = (
        text_key("approval_policy"),
        text_key("sandbox_mode"),
        text_key("approvals_reviewer"),
    );
    // 与 Claude 同理：三档预设之外的组合报 custom 并给出原值
    let (automation_mode, automation_raw) = match (approval, sandbox, reviewer) {
        (None, None, None) | (Some("on-request"), Some("workspace-write"), None | Some("user")) => {
            ("manual", Value::Null)
        }
        (Some("on-request"), Some("workspace-write"), Some("auto_review")) => ("auto", Value::Null),
        (Some("never"), Some("danger-full-access"), _) => ("full", Value::Null),
        _ => {
            let mut raw = Map::new();
            for (key, value) in [
                ("approval_policy", approval),
                ("sandbox_mode", sandbox),
                ("approvals_reviewer", reviewer),
            ] {
                if let Some(value) = value {
                    raw.insert(key.into(), Value::String(value.into()));
                }
            }
            ("custom", Value::Object(raw))
        }
    };
    Some(json!({
        "provider": provider.unwrap_or("openai"),
        "base_url": provider_config.and_then(|table| table.get("base_url")).and_then(toml::Value::as_str),
        "model": root.get("model").and_then(toml::Value::as_str),
        "timeout_ms": provider_config.and_then(|table| table.get("stream_idle_timeout_ms")).and_then(toml::Value::as_integer),
        "credential": {"configured": env_key.is_some(), "kind": env_key.map(|_| "env"), "variable": env_key},
        "runtime": {
            "automation_mode": automation_mode,
            "automation_raw": automation_raw,
            "codex_reasoning_effort": root.get("model_reasoning_effort").and_then(toml::Value::as_str),
            "codex_web_search": root.get("web_search").and_then(toml::Value::as_str),
            "codex_context_window": root.get("model_context_window").and_then(toml::Value::as_integer),
            "codex_auto_compact_token_limit": root.get("model_auto_compact_token_limit").and_then(toml::Value::as_integer)
        }
    }))
}

/// 只读客户端当前配置，输出固定脱敏结构（不含令牌正文），同时交回原始字节供识别与摘要使用。
/// `path` 必须与真实写入目标一致，状态响应据此告诉界面最终会写进哪个文件。
fn observe(
    path: &std::path::Path,
    kind: Kind,
    statusline_path: Option<&std::path::Path>,
) -> (Value, Option<Vec<u8>>) {
    let file = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let target = path.display().to_string();
    let bytes = match read_observed_config(path) {
        Ok(Some(bytes)) => bytes,
        Ok(None) => {
            return (
                json!({"state": "missing", "file": file, "target": target, "config": null}),
                None,
            )
        }
        Err(state) => {
            return (
                json!({"state": state, "file": file, "target": target, "config": null}),
                None,
            )
        }
    };
    let config = match kind {
        Kind::Claude => claude_observed_config(&bytes, statusline_path),
        Kind::Codex => codex_observed_config(&bytes),
    };
    match config {
        Some(config) => (
            json!({"state": "ready", "file": file, "target": target, "config": config}),
            Some(bytes),
        ),
        None => (
            json!({"state": "invalid", "file": file, "target": target, "config": null}),
            None,
        ),
    }
}

/// `agents:status` 的 `observed.current_config`：实际值、写入基准 `base` 与按内容识别的当前接入。
///
/// 只读零写：缺库不建库，密钥只读已刷新的缓存。`base` 只在文件可读（或还不存在）时给出，
/// 界面没有它就不能写；识别当前接入所需的任何一步读不了，都只让 `current` 为空，不让整条 status 失败。
pub(crate) fn current_config_status(
    ctx: &Context,
    agent_id: &str,
    spec: &Value,
    config_path: Option<&str>,
    kind: Kind,
) -> Value {
    let path = match target_path(ctx, kind, config_path) {
        Ok(path) => path,
        Err(_) => {
            let raw = config_path.unwrap_or("~/.codex/config.toml");
            let target = crate::native_mcp::targets::expand_home(raw, &ctx.home_dir);
            return json!({
                "state": "invalid-target",
                "file": target.file_name().and_then(|name| name.to_str()).unwrap_or_default(),
                "target": target.display().to_string(),
                "config": null,
                "base": null,
                "current": null
            });
        }
    };
    let statusline_path = statusline_target(ctx);
    let (mut status, bytes) = observe(
        &path,
        kind,
        (kind == Kind::Claude).then_some(statusline_path.as_path()),
    );
    status["base"] = Value::Null;
    status["current"] = Value::Null;
    if !matches!(status["state"].as_str(), Some("ready" | "missing")) {
        return status;
    }
    let bytes = bytes.unwrap_or_default();
    let Ok(recorded) = recorded_providers(ctx, agent_id) else {
        return status;
    };
    status["base"] = Value::String(base_digest(kind, &bytes, &recorded, &statusline_path));
    let (Ok(settings), Ok(live)) = (parse(spec), Live::read(kind, &bytes)) else {
        return status;
    };
    if let (Live::Claude(root), Some(credential)) = (
        &live,
        status
            .pointer_mut("/config/credential")
            .and_then(Value::as_object_mut),
    ) {
        credential.insert("saved_var".into(), json!(saved_token_var(root, &settings)));
    }
    let identified = identify(&live, &settings, &live.claimed(&recorded));
    status["current"] = identified.to_json(&settings);
    status
}

/// 哪个已保存接入的令牌变量解析值与实际令牌一致；只比较，不输出取值。
fn saved_token_var(root: &Value, settings: &AgentSettings) -> Option<String> {
    let (_, token) = claude_live_token(root)?;
    settings
        .profiles
        .values()
        .filter_map(|profile| profile.token_var.as_deref())
        .find(|var| crate::secret_values::lookup(var).as_deref() == Some(token.as_str()))
        .map(str::to_string)
}

/// 状态栏是**条件受管**：只有 `statusLine.command` 指向本工具部署的那份脚本时才算我们的。
/// 用户自己定义的状态栏与本工具无关，既不清空也不算外部修改。
const STATUSLINE_PATH: &[&str] = &["statusLine"];

/// POSIX shell 单引号引用：整体包进 `'…'`，其中的 `'` 写成 `'\''`。
fn shell_single_quote(raw: &str) -> String {
    format!("'{}'", raw.replace('\'', "'\\''"))
}

/// 写进 `statusLine.command` 的内置状态栏命令。Claude Code 在 shell 里执行这条命令，
/// 数据目录可能含空格（macOS 的 Application Support），所以始终引用，与路径内容无关。
fn statusline_command(statusline_path: &std::path::Path) -> String {
    shell_single_quote(&statusline_path.display().to_string())
}

/// 指向本工具状态栏的两种写法：当前的引用形式，与旧版写入的裸路径。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StatuslineForm {
    Quoted,
    Legacy,
}

/// 这条命令是不是本工具的状态栏、是哪种写法；识别、摘要与撤销都走这一个判定。
fn builtin_statusline_form(
    command: &str,
    statusline_path: &std::path::Path,
) -> Option<StatuslineForm> {
    if command == statusline_command(statusline_path) {
        Some(StatuslineForm::Quoted)
    } else if std::path::Path::new(command) == statusline_path {
        Some(StatuslineForm::Legacy)
    } else {
        None
    }
}

fn statusline_is_ours(root: &Value, statusline_path: &std::path::Path) -> bool {
    path_get(root, &["statusLine", "command"])
        .and_then(Value::as_str)
        .is_some_and(|cmd| builtin_statusline_form(cmd, statusline_path).is_some())
}

/// 令牌走哪个请求头。两种中转站都有，配错了会一直 401。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum AuthKind {
    /// `Authorization: Bearer <token>`，写 `ANTHROPIC_AUTH_TOKEN`。
    #[default]
    Token,
    /// `x-api-key: <key>`，写 `ANTHROPIC_API_KEY`。
    ApiKey,
}

impl AuthKind {
    fn parse(raw: Option<&Value>) -> Result<Self> {
        match raw.and_then(Value::as_str) {
            None | Some("token") => Ok(AuthKind::Token),
            Some("api_key") => Ok(AuthKind::ApiKey),
            Some(other) => bail!("auth 只能是 token 或 api_key，收到 {other}"),
        }
    }

    fn env_key(self) -> &'static str {
        match self {
            AuthKind::Token => "ANTHROPIC_AUTH_TOKEN",
            AuthKind::ApiKey => "ANTHROPIC_API_KEY",
        }
    }
}

/// 状态栏由谁维护。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum StatusLine {
    /// 不碰：用户自己定义的那份原样保留。
    #[default]
    Untouched,
    /// 用本工具维护的那份：部署脚本并把 statusLine 指过去。
    Builtin,
}

impl StatusLine {
    fn parse(raw: Option<&Value>) -> Result<Self> {
        match raw.and_then(Value::as_str) {
            None | Some("") | Some("untouched") => Ok(StatusLine::Untouched),
            Some("builtin") => Ok(StatusLine::Builtin),
            Some(other) => bail!("statusline 只能是 builtin 或 untouched，收到 {other}"),
        }
    }
}

/// 一套接入配置：连到哪、拿什么认证、用什么模型。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct ConnectionProfile {
    /// 界面上显示的名字；留空时用键名。
    pub(crate) label: Option<String>,
    pub(crate) base_url: Option<String>,
    pub(crate) auth: AuthKind,
    /// 令牌所在的环境变量名。库里只存这个名字，取值另行解析。
    pub(crate) token_var: Option<String>,
    pub(crate) model: Option<String>,
    /// 后台小任务用的轻量模型。
    pub(crate) small_fast_model: Option<String>,
    /// opus / sonnet / haiku 各自映射到中转站的哪个模型名。
    pub(crate) model_aliases: BTreeMap<String, String>,
    pub(crate) timeout_ms: Option<u64>,
    /// Codex 专用：直接引用 config.toml 里已有的接入方式（用户自己的表或内置 id）。
    /// 切换到它只写 `model_provider`，不写也不删那张表；与地址、令牌变量、超时互斥。
    pub(crate) provider_ref: Option<String>,
}

/// 一个助手的全部运行参数。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct AgentSettings {
    /// 当前生效的接入配置名。
    pub(crate) active: Option<String>,
    pub(crate) profiles: BTreeMap<String, ConnectionProfile>,
    /// 自动化程度；完全放开时调用方负责二次确认。
    pub(crate) automation_mode: AutomationMode,
    pub(crate) statusline: StatusLine,
    pub(crate) claude_tool_search: bool,
    pub(crate) claude_agent_teams: bool,
    pub(crate) claude_effort_level: Option<String>,
    pub(crate) claude_disable_artifact: bool,
    pub(crate) claude_hide_attribution: bool,
    pub(crate) codex_reasoning_effort: Option<String>,
    pub(crate) codex_context_window: Option<u64>,
    pub(crate) codex_auto_compact_token_limit: Option<u64>,
    pub(crate) codex_web_search: Option<String>,
}

fn non_empty(raw: Option<&Value>) -> Option<String> {
    let text = raw?.as_str()?.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

fn optional_choice(
    object: &Map<String, Value>,
    key: &str,
    allowed: &[&str],
) -> Result<Option<String>> {
    let Some(value) = non_empty(object.get(key)) else {
        return Ok(None);
    };
    if allowed.contains(&value.as_str()) {
        Ok(Some(value))
    } else {
        bail!(
            "settings.{key} 只能是 {}，收到 {value}",
            allowed.join(" / ")
        )
    }
}

fn optional_positive_u64(object: &Map<String, Value>, key: &str) -> Result<Option<u64>> {
    let Some(value) = object.get(key) else {
        return Ok(None);
    };
    let number = value
        .as_u64()
        .filter(|number| *number > 0)
        .ok_or_else(|| anyhow::anyhow!("settings.{key} 必须是正整数"))?;
    Ok(Some(number))
}

/// 接入配置名：只用安全字符，便于在命令行与界面里直接指名。
fn valid_profile_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn parse_profile(id: &str, raw: &Value) -> Result<ConnectionProfile> {
    let Some(obj) = raw.as_object() else {
        bail!("接入配置 {id} 必须是对象");
    };
    let base_url = non_empty(obj.get("base_url"));
    if let Some(url) = base_url.as_deref() {
        // 实际配置里已有的地址不能因为这里的校验存不下：允许任意主机的 http（界面对非 https
        // 提示令牌将明文传输），其余协议一律拒绝
        if !url.starts_with("https://") && !url.starts_with("http://") {
            bail!("接入配置 {id} 的地址只允许 https:// 或 http://");
        }
    }
    let token_var = non_empty(obj.get("token_var"));
    if let Some(name) = token_var.as_deref() {
        if !crate::secret_values::valid_var_name(name) {
            bail!(
                "接入配置 {id} 的令牌变量名不合法：要以字母或下划线开头，只能用字母、数字与下划线"
            );
        }
    }
    let timeout_ms = match obj.get("timeout_ms") {
        None | Some(Value::Null) => None,
        Some(value) => {
            let ms = value
                .as_u64()
                .ok_or_else(|| anyhow::anyhow!("接入配置 {id} 的超时必须是非负整数毫秒"))?;
            if ms == 0 {
                None
            } else {
                Some(ms)
            }
        }
    };
    let mut model_aliases = BTreeMap::new();
    if let Some(raw_aliases) = obj.get("model_aliases") {
        let map = raw_aliases
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("接入配置 {id} 的模型别名必须是对象"))?;
        for tier in ["opus", "sonnet", "haiku"] {
            if let Some(value) = non_empty(map.get(tier)) {
                model_aliases.insert(tier.to_string(), value);
            }
        }
    }
    let provider_ref = non_empty(obj.get("provider_ref"));
    if let Some(reference) = provider_ref.as_deref() {
        if reference.len() > 128 || reference.chars().any(char::is_control) {
            bail!("接入配置 {id} 引用的接入方式名不合法");
        }
        if base_url.is_some() || token_var.is_some() || timeout_ms.is_some() {
            bail!(
                "接入配置 {id} 引用了 config.toml 里的接入方式 {reference}：地址、令牌变量与超时由那张表决定，不能再单独填写"
            );
        }
    }
    Ok(ConnectionProfile {
        label: non_empty(obj.get("label")),
        base_url,
        auth: AuthKind::parse(obj.get("auth"))?,
        token_var,
        model: non_empty(obj.get("model")),
        small_fast_model: non_empty(obj.get("small_fast_model")),
        model_aliases,
        timeout_ms,
        provider_ref,
    })
}

/// 从助手定义里读运行参数。缺 `settings` 段就是没配过。
pub(crate) fn parse(spec: &Value) -> Result<AgentSettings> {
    let Some(raw) = spec.get("settings") else {
        return Ok(AgentSettings::default());
    };
    let Some(obj) = raw.as_object() else {
        bail!("settings 必须是对象");
    };

    let mut profiles = BTreeMap::new();
    if let Some(raw_profiles) = obj.get("profiles") {
        let map = raw_profiles
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("settings.profiles 必须是对象"))?;
        for (name, value) in map {
            if !valid_profile_name(name) {
                bail!("接入配置名 {name} 不合法：只能用字母、数字、连字符与下划线，最长 64 位");
            }
            profiles.insert(name.clone(), parse_profile(name, value)?);
        }
    }

    let active = non_empty(obj.get("active"));
    if let Some(name) = active.as_deref() {
        if !profiles.contains_key(name) {
            bail!("settings.active 指向不存在的接入配置 {name}");
        }
    }

    Ok(AgentSettings {
        active,
        profiles,
        automation_mode: AutomationMode::parse(obj)?,
        statusline: StatusLine::parse(obj.get("statusline"))?,
        claude_tool_search: obj
            .get("claude_tool_search")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        claude_agent_teams: obj
            .get("claude_agent_teams")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        claude_effort_level: optional_choice(obj, "claude_effort_level", CLAUDE_EFFORT_LEVELS)?,
        claude_disable_artifact: obj
            .get("claude_disable_artifact")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        claude_hide_attribution: obj
            .get("claude_hide_attribution")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        codex_reasoning_effort: optional_choice(
            obj,
            "codex_reasoning_effort",
            CODEX_REASONING_EFFORTS,
        )?,
        codex_context_window: optional_positive_u64(obj, "codex_context_window")?,
        codex_auto_compact_token_limit: optional_positive_u64(
            obj,
            "codex_auto_compact_token_limit",
        )?,
        codex_web_search: optional_choice(obj, "codex_web_search", CODEX_WEB_SEARCH_MODES)?,
    })
}

// ---------- 键路径读写 ----------

fn path_get<'a>(root: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut cursor = root;
    for key in path {
        cursor = cursor.get(*key)?;
    }
    Some(cursor)
}

/// 设置某条路径上的值；中途缺的对象顺手补上。
fn path_set(root: &mut Map<String, Value>, path: &[&str], value: Value) {
    let (last, parents) = match path.split_last() {
        Some(parts) => parts,
        None => return,
    };
    let mut cursor = root;
    for key in parents {
        let entry = cursor
            .entry((*key).to_string())
            .or_insert_with(|| Value::Object(Map::new()));
        if !entry.is_object() {
            *entry = Value::Object(Map::new());
        }
        cursor = entry.as_object_mut().expect("just ensured object");
    }
    cursor.insert((*last).to_string(), value);
}

/// 删掉某条路径；父对象因此变空的话一并收走，不留 `"env": {}` 这种空壳。
fn path_remove(root: &mut Map<String, Value>, path: &[&str]) {
    let (last, parents) = match path.split_last() {
        Some(parts) => parts,
        None => return,
    };
    if parents.is_empty() {
        root.remove(*last);
        return;
    }
    let mut cursor: &mut Map<String, Value> = root;
    let mut chain: Vec<String> = Vec::new();
    for key in parents {
        match cursor.get_mut(*key).and_then(Value::as_object_mut) {
            Some(next) => {
                chain.push((*key).to_string());
                cursor = next;
            }
            None => return,
        }
    }
    cursor.remove(*last);
    // 自底向上收空壳
    while let Some(tail) = chain.pop() {
        let mut probe: &mut Map<String, Value> = root;
        for key in &chain {
            probe = probe
                .get_mut(key)
                .and_then(Value::as_object_mut)
                .expect("chain was walked above");
        }
        let empty = probe
            .get(&tail)
            .and_then(Value::as_object)
            .is_some_and(Map::is_empty);
        if !empty {
            return;
        }
        probe.remove(&tail);
    }
}

/// 把本工具拥有的那几条路径切出来，用于外部修改检测。
/// 用户往 settings.json 里加自己的东西不该被判成冲突。
pub(crate) fn owned_slice(bytes: &[u8], statusline_path: &std::path::Path) -> Value {
    let root = serde_json::from_slice::<Value>(bytes).unwrap_or(Value::Null);
    let mut slice = Map::new();
    for path in CLAUDE_OWNED_PATHS {
        if let Some(found) = path_get(&root, path) {
            path_set(&mut slice, path, found.clone());
        }
    }
    // 指向别处的状态栏是用户的，不进摘要——他换自己的脚本不该被判成冲突
    if statusline_is_ours(&root, statusline_path) {
        if let Some(found) = path_get(&root, STATUSLINE_PATH) {
            path_set(&mut slice, STATUSLINE_PATH, found.clone());
        }
    }
    Value::Object(slice)
}

pub(crate) fn owned_digest(bytes: &[u8], statusline_path: &std::path::Path) -> String {
    crate::native_mcp::model::sha256_hex_bytes(
        serde_json::to_vec(&owned_slice(bytes, statusline_path))
            .unwrap_or_default()
            .as_slice(),
    )
}

// ---------- 渲染：接入与运行参数分开写，互不越界 ----------

/// 写 Claude 的接入键：先把接入键全部清掉，再按目标那套写回（`None` = 助手默认账号）。
/// 运行参数的键一律不碰。
///
/// `resolve` 把变量名换成取值；取不到就报错而不是写一个空令牌上去——
/// 空令牌会让助手以为配好了，实际每次请求都被拒。
fn write_claude_connection(
    root: &mut Map<String, Value>,
    profile: Option<&ConnectionProfile>,
    resolve: &dyn Fn(&str) -> Option<String>,
) -> Result<()> {
    // 切换接入配置时，上一套留下的变量必须消失，否则两套混着生效
    for path in CLAUDE_CONNECTION_PATHS {
        path_remove(root, path);
    }
    let Some(profile) = profile else {
        return Ok(());
    };
    if profile.provider_ref.is_some() {
        bail!("引用 config.toml 接入方式的接入配置只适用于 Codex");
    }
    if let Some(url) = profile.base_url.as_deref() {
        path_set(
            root,
            &["env", "ANTHROPIC_BASE_URL"],
            Value::String(url.to_string()),
        );
    }
    if let Some(name) = profile.token_var.as_deref() {
        let Some(value) = resolve(name) else {
            bail!("环境变量 {name} 还没有取值：先在密钥页填上，或在启动环境里 export 它");
        };
        path_set(root, &["env", profile.auth.env_key()], Value::String(value));
    }
    if let Some(model) = profile.model.as_deref() {
        path_set(
            root,
            &["env", "ANTHROPIC_MODEL"],
            Value::String(model.to_string()),
        );
    }
    if let Some(model) = profile.small_fast_model.as_deref() {
        path_set(
            root,
            &["env", "ANTHROPIC_SMALL_FAST_MODEL"],
            Value::String(model.to_string()),
        );
    }
    for (tier, target) in &profile.model_aliases {
        let key = match tier.as_str() {
            "opus" => "ANTHROPIC_DEFAULT_OPUS_MODEL",
            "sonnet" => "ANTHROPIC_DEFAULT_SONNET_MODEL",
            "haiku" => "ANTHROPIC_DEFAULT_HAIKU_MODEL",
            _ => continue,
        };
        path_set(root, &["env", key], Value::String(target.clone()));
    }
    if let Some(ms) = profile.timeout_ms {
        // Claude Code 读的是字符串形式的环境变量
        path_set(
            root,
            &["env", "API_TIMEOUT_MS"],
            Value::String(ms.to_string()),
        );
    }
    Ok(())
}

fn set_or_remove(root: &mut Map<String, Value>, path: &[&str], value: Option<String>) {
    match value {
        Some(value) => path_set(root, path, Value::String(value)),
        None => path_remove(root, path),
    }
}

/// 只写补丁里那几项运行参数；补丁外的键原样保留，包括本工具不提供的取值（如 acceptEdits）。
fn write_claude_runtime(
    root: &mut Map<String, Value>,
    patch: &RuntimePatch,
    statusline_path: &std::path::Path,
) {
    for (key, value) in &patch.0 {
        match (*key, value) {
            ("automation_mode", RuntimeValue::Mode(mode)) => {
                let literal = match mode {
                    AutomationMode::Manual => "manual",
                    AutomationMode::Auto => "auto",
                    AutomationMode::Full => "bypassPermissions",
                };
                path_set(
                    root,
                    &["permissions", "defaultMode"],
                    Value::String(literal.into()),
                );
            }
            // 状态栏：选内置就指过去；选「不碰」时只撤掉我们自己指过去的那条，
            // 用户手配的原样留着——它本来就不归我们管。
            ("statusline", RuntimeValue::Statusline(StatusLine::Builtin)) => {
                let mut entry = Map::new();
                entry.insert("type".into(), Value::String("command".into()));
                entry.insert(
                    "command".into(),
                    Value::String(statusline_command(statusline_path)),
                );
                entry.insert("padding".into(), Value::Number(0.into()));
                path_set(root, STATUSLINE_PATH, Value::Object(entry));
            }
            ("statusline", RuntimeValue::Statusline(StatusLine::Untouched)) => {
                if statusline_is_ours(&Value::Object(root.clone()), statusline_path) {
                    path_remove(root, STATUSLINE_PATH);
                }
            }
            ("claude_effort_level", RuntimeValue::Text(level)) => {
                set_or_remove(root, &["env", "CLAUDE_CODE_EFFORT_LEVEL"], level.clone());
            }
            ("claude_tool_search", RuntimeValue::Flag(on)) => {
                set_or_remove(
                    root,
                    &["env", "ENABLE_TOOL_SEARCH"],
                    on.then(|| "true".to_string()),
                );
            }
            ("claude_agent_teams", RuntimeValue::Flag(on)) => {
                set_or_remove(
                    root,
                    &["env", "CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS"],
                    on.then(|| "1".to_string()),
                );
            }
            ("claude_disable_artifact", RuntimeValue::Flag(on)) => {
                set_or_remove(
                    root,
                    &["env", "CLAUDE_CODE_DISABLE_ARTIFACT"],
                    on.then(|| "1".to_string()),
                );
            }
            ("claude_hide_attribution", RuntimeValue::Flag(on)) => {
                set_or_remove(root, &["attribution", "commit"], on.then(String::new));
                set_or_remove(root, &["attribution", "pr"], on.then(String::new));
            }
            // parse_runtime_patch 已按形态校验过字段名与取值类型
            _ => {}
        }
    }
}

// ---------- 运行参数补丁 ----------

/// 运行参数的一项取值（已按适配器词汇表校验）。
#[derive(Debug, Clone, PartialEq, Eq)]
enum RuntimeValue {
    Mode(AutomationMode),
    Statusline(StatusLine),
    Flag(bool),
    Text(Option<String>),
    Number(Option<u64>),
}

/// 运行参数补丁：只含这次要改的字段；没出现的字段一律不碰。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct RuntimePatch(BTreeMap<&'static str, RuntimeValue>);

impl RuntimePatch {
    fn sets_full(&self) -> bool {
        self.0.get("automation_mode") == Some(&RuntimeValue::Mode(AutomationMode::Full))
    }

    fn sets_builtin_statusline(&self) -> bool {
        self.0.get("statusline") == Some(&RuntimeValue::Statusline(StatusLine::Builtin))
    }

    fn keys(&self) -> Vec<&'static str> {
        self.0.keys().copied().collect()
    }
}

enum FieldKind {
    Mode,
    Statusline,
    Flag,
    Choice(&'static [&'static str]),
    Number,
}

const CLAUDE_RUNTIME_FIELDS: &[(&str, FieldKind)] = &[
    ("automation_mode", FieldKind::Mode),
    ("statusline", FieldKind::Statusline),
    (
        "claude_effort_level",
        FieldKind::Choice(CLAUDE_EFFORT_LEVELS),
    ),
    ("claude_tool_search", FieldKind::Flag),
    ("claude_agent_teams", FieldKind::Flag),
    ("claude_disable_artifact", FieldKind::Flag),
    ("claude_hide_attribution", FieldKind::Flag),
];

const CODEX_RUNTIME_FIELDS: &[(&str, FieldKind)] = &[
    ("automation_mode", FieldKind::Mode),
    (
        "codex_reasoning_effort",
        FieldKind::Choice(CODEX_REASONING_EFFORTS),
    ),
    (
        "codex_web_search",
        FieldKind::Choice(CODEX_WEB_SEARCH_MODES),
    ),
    ("codex_context_window", FieldKind::Number),
    ("codex_auto_compact_token_limit", FieldKind::Number),
];

/// 解析 `--runtime` 补丁。字段名必须属于该形态（拼错了不能静默不生效），
/// `null` / `""` / `false` 表示撤掉该项、恢复客户端默认值。
fn parse_runtime_patch(kind: Kind, raw: &Value) -> Result<RuntimePatch> {
    let Some(object) = raw.as_object() else {
        bail!("--runtime 必须是 JSON 对象");
    };
    let fields = match kind {
        Kind::Claude => CLAUDE_RUNTIME_FIELDS,
        Kind::Codex => CODEX_RUNTIME_FIELDS,
    };
    let mut patch = BTreeMap::new();
    for (key, value) in object {
        let Some((name, field)) = fields.iter().find(|(name, _)| name == key) else {
            bail!("{key} 不是这个助手的运行参数");
        };
        let parsed = match field {
            FieldKind::Mode => RuntimeValue::Mode(match value.as_str() {
                Some("manual") => AutomationMode::Manual,
                Some("auto") => AutomationMode::Auto,
                Some("full") => AutomationMode::Full,
                _ => bail!("automation_mode 只能是 manual / auto / full"),
            }),
            FieldKind::Statusline => RuntimeValue::Statusline(match value {
                Value::Null => StatusLine::Untouched,
                other => StatusLine::parse(Some(other))?,
            }),
            FieldKind::Flag => RuntimeValue::Flag(match value {
                Value::Bool(on) => *on,
                Value::Null => false,
                _ => bail!("{key} 必须是 true 或 false"),
            }),
            FieldKind::Choice(allowed) => {
                if !value.is_null() && !value.is_string() {
                    bail!("{key} 必须是字符串");
                }
                let single = Map::from_iter([(key.clone(), value.clone())]);
                RuntimeValue::Text(optional_choice(&single, key, allowed)?)
            }
            FieldKind::Number => RuntimeValue::Number(match value {
                Value::Null => None,
                Value::String(text) if text.trim().is_empty() => None,
                _ => {
                    let single = Map::from_iter([(key.clone(), value.clone())]);
                    optional_positive_u64(&single, key)?
                }
            }),
        };
        patch.insert(*name, parsed);
    }
    Ok(RuntimePatch(patch))
}

// ---------- Codex：config.toml ----------

/// Codex 的目标文件，相对 CODEX_HOME。
pub(crate) const CODEX_CONFIG: &str = "config.toml";

/// 旧版本写在受管 provider 表前的注释。新版本不再往助手配置里写它（哪些是本工具写的
/// 只记在库里），仅在迁移时读取；MCP 域写的同名注释不归这里管，原样保留。
const LEGACY_MANAGED_MARK: &str = "# [agent-manager:managed]";

/// 本工具在 config.toml 里拥有的顶层键。
const CODEX_OWNED_KEYS: &[&str] = &[
    "model_provider",
    "model",
    "approval_policy",
    "sandbox_mode",
    "approvals_reviewer",
    "model_reasoning_effort",
    "model_context_window",
    "model_auto_compact_token_limit",
    "web_search",
];

/// Codex 内置的 provider id，不允许覆盖（写了也不生效，还会盖掉内置定义）。
const CODEX_RESERVED_PROVIDERS: &[&str] = &["openai", "ollama", "lmstudio"];

/// 顶层键在哪儿结束：TOML 里 `[table]` 之后的键属于那个表，所以根键必须在第一个表头之前。
fn toml_root_end(lines: &[&str]) -> usize {
    lines
        .iter()
        .position(|line| {
            let t = line.trim();
            t.starts_with('[') && !t.starts_with("[[") || t.starts_with("[[")
        })
        .unwrap_or(lines.len())
}

/// 这一行是不是在给 `key` 赋值。
fn toml_line_assigns(line: &str, key: &str) -> bool {
    let t = line.trim_start();
    let Some(rest) = t.strip_prefix(key) else {
        return false;
    };
    rest.trim_start().starts_with('=')
}

/// 新的顶层键插在哪儿：第一个表头之前，且要越过紧贴表头的注释——
/// 那些注释属于下面那个表段（受管标记就是这么用的），插进去会把它们拆散。
fn toml_insert_point(lines: &[&str]) -> usize {
    let mut point = toml_root_end(lines);
    while point > 0 {
        let prev = lines[point - 1].trim();
        if prev.starts_with('#') || prev.is_empty() {
            point -= 1;
        } else {
            break;
        }
    }
    point
}

/// 设置或删除一个顶层键；其余行（含注释与格式）原样保留。
fn toml_set_root_key(content: &str, key: &str, value: Option<&str>) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let root_end = toml_root_end(&lines);
    // 先把现有的那一行（可能有多行重复赋值，本就非法）全部摘掉
    let kept: Vec<String> = lines
        .iter()
        .enumerate()
        .filter(|(index, line)| !(*index < root_end && toml_line_assigns(line, key)))
        .map(|(_, line)| (*line).to_string())
        .collect();

    let mut out = kept;
    if let Some(v) = value {
        let refs: Vec<&str> = out.iter().map(String::as_str).collect();
        let point = toml_insert_point(&refs);
        out.insert(point, format!("{key} = {v}"));
    }
    let mut text = out.join("\n");
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text
}

/// 取一个顶层键的原始字面量（含引号），没有则 None。
fn toml_root_key_value(content: &str, key: &str) -> Option<String> {
    let lines: Vec<&str> = content.lines().collect();
    let root_end = toml_root_end(&lines);
    lines[..root_end]
        .iter()
        .find(|line| toml_line_assigns(line, key))
        .and_then(|line| line.split_once('='))
        .map(|(_, value)| value.trim().to_string())
}

/// 表头文本（去掉方括号）是不是 `model_providers.<id>` 或它的子表，是则给出 `<id>`。
fn provider_id_of(header: &str) -> Option<&str> {
    let rest = header.trim().strip_prefix("model_providers.")?;
    if let Some(quoted) = rest.strip_prefix('"') {
        return quoted.find('"').map(|end| &quoted[..end]);
    }
    rest.split('.').next().map(str::trim)
}

/// 一行是不是表头；是则给出去掉方括号的表头文本。
fn toml_header(trimmed: &str) -> Option<&str> {
    (trimmed.starts_with('[') && trimmed.ends_with(']'))
        .then(|| trimmed.trim_matches(|c| c == '[' || c == ']'))
}

/// 本工具认领的 provider：库里记下的上次写入的那些，加上文件里仍带旧受管注释的
/// `[model_providers.*]`——那条注释只有旧版本写过，认下来才能把它们迁走。
fn codex_claimed_providers(content: &str, recorded: &[String]) -> BTreeSet<String> {
    let mut claimed: BTreeSet<String> = recorded.iter().cloned().collect();
    let mut pending_mark = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == LEGACY_MANAGED_MARK {
            pending_mark = true;
            continue;
        }
        if pending_mark {
            if let Some(id) = toml_header(trimmed).and_then(provider_id_of) {
                claimed.insert(id.to_string());
            }
        }
        pending_mark = false;
    }
    claimed
}

/// 文件里有没有 `[model_providers.<name>]` 或它的子表（与认领同一套表头解析）。
fn toml_has_provider(content: &str, name: &str) -> bool {
    content
        .lines()
        .any(|line| toml_header(line.trim()).and_then(provider_id_of) == Some(name))
}

/// 删掉认领集合里的 provider 表（含子表）以及紧贴在它们前面的旧受管注释，其余原样保留。
fn toml_remove_provider_tables(content: &str, claimed: &BTreeSet<String>) -> String {
    let mut out = String::new();
    let mut skip = false;
    let mut pending_mark = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == LEGACY_MANAGED_MARK {
            // 先攥着：下一行是要删的表头就连注释一起丢，否则原样放回
            if pending_mark {
                out.push_str(LEGACY_MANAGED_MARK);
                out.push('\n');
            }
            pending_mark = true;
            continue;
        }
        if let Some(header) = toml_header(trimmed) {
            let ours = provider_id_of(header).is_some_and(|id| claimed.contains(id));
            skip = ours;
            if !ours && pending_mark {
                out.push_str(LEGACY_MANAGED_MARK);
                out.push('\n');
            }
            pending_mark = false;
        } else if pending_mark {
            out.push_str(LEGACY_MANAGED_MARK);
            out.push('\n');
            pending_mark = false;
        }
        if !skip {
            out.push_str(line);
            out.push('\n');
        }
    }
    if pending_mark {
        out.push_str(LEGACY_MANAGED_MARK);
        out.push('\n');
    }
    out
}

/// 收集认领集合里的 provider 表正文，用于外部修改检测。
///
/// 口径与旧版本一致（表头 + 逐行 trim 的正文，旧受管注释不计入），带旧注释的既有文件
/// 算出的摘要不变，升级后不会被误判为外部修改。
fn toml_provider_tables(content: &str, claimed: &BTreeSet<String>) -> Vec<String> {
    let mut found = Vec::new();
    let mut current: Option<String> = None;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == LEGACY_MANAGED_MARK {
            continue;
        }
        if let Some(header) = toml_header(trimmed) {
            if let Some(block) = current.take() {
                found.push(block);
            }
            if provider_id_of(header).is_some_and(|id| claimed.contains(id)) {
                current = Some(format!("{trimmed}\n"));
            }
            continue;
        }
        if let Some(block) = current.as_mut() {
            block.push_str(trimmed);
            block.push('\n');
        }
    }
    if let Some(block) = current.take() {
        found.push(block);
    }
    found
}

/// config.toml 里本工具拥有的那部分：明确列举的顶层键 + 认领的 provider 表。
///
/// `recorded` 是库里记下的上次写入的 provider id。
pub(crate) fn codex_owned_slice(content: &str, recorded: &[String]) -> Value {
    let mut slice = Map::new();
    for key in CODEX_OWNED_KEYS {
        if let Some(value) = toml_root_key_value(content, key) {
            slice.insert((*key).to_string(), Value::String(value));
        }
    }
    let claimed = codex_claimed_providers(content, recorded);
    let tables = toml_provider_tables(content, &claimed);
    if !tables.is_empty() {
        slice.insert(
            "model_providers".into(),
            Value::Array(tables.into_iter().map(Value::String).collect()),
        );
    }
    Value::Object(slice)
}

pub(crate) fn codex_owned_digest(content: &str, recorded: &[String]) -> String {
    crate::native_mcp::model::sha256_hex_bytes(
        serde_json::to_vec(&codex_owned_slice(content, recorded))
            .unwrap_or_default()
            .as_slice(),
    )
}

/// Codex 内置、不需要表的接入方式。
fn codex_builtin(id: &str) -> bool {
    CODEX_RESERVED_PROVIDERS.contains(&id)
}

/// 本工具为一套自定义接入写的 `[model_providers.<name>]` 表正文。
///
/// 令牌不落盘：Codex 的 provider 用 `env_key` 指向环境变量名，由它自己去读，
/// 因此这里只写变量名——与 Claude 那边被迫写真值不同。
fn codex_provider_table_text(name: &str, profile: &ConnectionProfile) -> Result<String> {
    let Some(url) = profile.base_url.as_deref() else {
        bail!("接入配置 {name} 要填接口地址，Codex 的自定义接入方式没有默认地址");
    };
    let mut table = format!(
        "\n[model_providers.{}]\nname = {}\nbase_url = {}\n",
        name,
        crate::native_mcp::codecs::toml_basic_string(profile.label.as_deref().unwrap_or(name)),
        crate::native_mcp::codecs::toml_basic_string(url),
    );
    if let Some(var) = profile.token_var.as_deref() {
        table.push_str(&format!(
            "env_key = {}\n",
            crate::native_mcp::codecs::toml_basic_string(var)
        ));
        // Provider 请求优先使用 env_key；保留这个标志让 Codex 继续识别并刷新
        // auth.json 中的 ChatGPT 登录，避免切换第三方接入后账号/插件表现为掉线。
        table.push_str("requires_openai_auth = true\n");
    }
    // wire_api 目前只有 responses 这一种，写死避免用户填错
    table.push_str("wire_api = \"responses\"\n");
    if let Some(ms) = profile.timeout_ms {
        table.push_str(&format!("stream_idle_timeout_ms = {ms}\n"));
    }
    Ok(table)
}

/// 按接入配置渲染出的 provider 表（解析后的结构），识别与回填时拿来和实际表比较。
fn codex_rendered_table(name: &str, profile: &ConnectionProfile) -> Result<toml::value::Table> {
    let parsed: toml::Value = codex_provider_table_text(name, profile)?.parse()?;
    parsed
        .get("model_providers")
        .and_then(|providers| providers.get(name))
        .and_then(toml::Value::as_table)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("渲染出的接入方式表缺失：{name}"))
}

/// 表里的 `name` 只是显示名（本工具写的是接入配置的显示名），比较内容时不算。
fn without_name(table: &toml::value::Table) -> toml::value::Table {
    let mut table = table.clone();
    table.remove("name");
    table
}

/// 文件里（按解析结果）有没有 `model_providers.<id>`，内联表写法也算。
fn codex_has_provider_parsed(text: &str, id: &str) -> bool {
    text.parse::<toml::Value>()
        .ok()
        .and_then(|root| root.get("model_providers")?.get(id).cloned())
        .is_some()
}

/// 写 Codex 的接入部分：撤掉本工具认领的 provider 表与 `model_provider` / `model`，
/// 再按目标那套写回（`None` = 助手默认账号）。运行参数的顶层键一律不碰。
///
/// `recorded` 是库里记下的上次写入的 provider id；返回渲染结果与本次写入的 provider id，
/// 后者由调用方记回库里。引用型接入（`provider_ref`）只写 `model_provider`，不写也不删那张表。
fn write_codex_connection(
    existing: &str,
    target: Option<(&str, &ConnectionProfile)>,
    recorded: &[String],
) -> Result<(String, Vec<String>)> {
    let claimed = codex_claimed_providers(existing, recorded);
    let mut text = toml_remove_provider_tables(existing, &claimed);
    for key in ["model_provider", "model"] {
        text = toml_set_root_key(&text, key, None);
    }
    let mut written = Vec::new();
    let Some((name, profile)) = target else {
        return Ok((text, written));
    };
    if let Some(reference) = profile.provider_ref.as_deref() {
        if !codex_builtin(reference) && !codex_has_provider_parsed(&text, reference) {
            bail!(
                "接入配置 {name} 引用的接入方式 {reference} 已不在 {CODEX_CONFIG} 中（也不是 Codex 内置的），拒绝切换：请先恢复那张表，或删除这个接入配置"
            );
        }
        text = toml_set_root_key(
            &text,
            "model_provider",
            Some(&crate::native_mcp::codecs::toml_basic_string(reference)),
        );
    } else {
        if CODEX_RESERVED_PROVIDERS.contains(&name) {
            bail!("{name} 是 Codex 内置的接入方式，换个名字（内置的不能覆盖）");
        }
        // 只有真配了地址或令牌变量才值得建一个 provider；否则就是用 Codex 自带的登录
        if profile.base_url.is_some() || profile.token_var.is_some() {
            let table = codex_provider_table_text(name, profile)?;
            // 库里没记录的同名表是用户自己的：再追加一个会成重复表，Codex 读不了
            if !claimed.contains(name) && toml_has_provider(existing, name) {
                bail!(
                    "{CODEX_CONFIG} 中已有你自己的同名接入方式 {name}，拒绝覆盖；请给这个接入配置换个名字（例如创建副本后删掉原项）"
                );
            }
            if !text.ends_with('\n') && !text.is_empty() {
                text.push('\n');
            }
            text.push_str(&table);
            written.push(name.to_string());
            text = toml_set_root_key(
                &text,
                "model_provider",
                Some(&crate::native_mcp::codecs::toml_basic_string(name)),
            );
        }
    }
    if let Some(model) = profile.model.as_deref() {
        text = toml_set_root_key(
            &text,
            "model",
            Some(&crate::native_mcp::codecs::toml_basic_string(model)),
        );
    }
    Ok((text, written))
}

/// Codex 内置接入方式的已知默认端点与令牌变量（本机的不需要令牌）。
fn codex_builtin_endpoint(id: &str) -> Option<(&'static str, Option<&'static str>)> {
    match id {
        "openai" => Some(("https://api.openai.com/v1", Some("OPENAI_API_KEY"))),
        "ollama" => Some(("http://localhost:11434/v1", None)),
        "lmstudio" => Some(("http://localhost:1234/v1", None)),
        _ => None,
    }
}

/// 引用型接入实际连到哪：config.toml 里被引用的那张表的地址与令牌变量；表不在时，
/// 内置接入方式用已知默认值，其余明确报错。只读，供可用性检查使用。
pub(crate) fn codex_referenced_endpoint(
    ctx: &Context,
    config_path: Option<&str>,
    reference: &str,
) -> Result<(String, Option<String>)> {
    let path = codex_target(ctx, config_path)?;
    let bytes = match read_observed_config(&path) {
        Ok(bytes) => bytes.unwrap_or_default(),
        Err(state) => {
            bail!("{CODEX_CONFIG} 读不了（{state}），无法确定引用的接入方式 {reference} 连到哪")
        }
    };
    let root = std::str::from_utf8(&bytes)
        .ok()
        .and_then(|text| text.parse::<toml::Value>().ok())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "{CODEX_CONFIG} 不是合法的 TOML，无法确定引用的接入方式 {reference} 连到哪"
            )
        })?;
    if let Some(table) = root
        .get("model_providers")
        .and_then(|providers| providers.get(reference))
        .and_then(toml::Value::as_table)
    {
        let text = |key: &str| {
            table
                .get(key)
                .and_then(toml::Value::as_str)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        };
        let Some(base_url) = text("base_url") else {
            bail!("{CODEX_CONFIG} 里的接入方式 {reference} 没有 base_url，没有可测的端点");
        };
        return Ok((base_url, text("env_key")));
    }
    match codex_builtin_endpoint(reference) {
        Some((base_url, env_key)) => Ok((base_url.to_string(), env_key.map(str::to_string))),
        None => bail!(
            "引用的接入方式 {reference} 已不在 {CODEX_CONFIG} 中（也不是 Codex 内置的），无法检查；请先恢复那张表，或删除这个接入配置"
        ),
    }
}

/// 只写补丁里那几项运行参数的顶层键；补丁外的键原样保留。
fn write_codex_runtime(existing: &str, patch: &RuntimePatch) -> String {
    let mut text = existing.to_string();
    let string_literal = |value: &Option<String>| {
        value
            .as_deref()
            .map(crate::native_mcp::codecs::toml_basic_string)
    };
    for (key, value) in &patch.0 {
        match (*key, value) {
            ("automation_mode", RuntimeValue::Mode(mode)) => {
                for key in ["approval_policy", "sandbox_mode", "approvals_reviewer"] {
                    text = toml_set_root_key(&text, key, None);
                }
                let values: &[(&str, &str)] = match mode {
                    AutomationMode::Manual => &[
                        ("approval_policy", "\"on-request\""),
                        ("sandbox_mode", "\"workspace-write\""),
                        ("approvals_reviewer", "\"user\""),
                    ],
                    AutomationMode::Auto => &[
                        ("approval_policy", "\"on-request\""),
                        ("sandbox_mode", "\"workspace-write\""),
                        ("approvals_reviewer", "\"auto_review\""),
                    ],
                    AutomationMode::Full => &[
                        ("approval_policy", "\"never\""),
                        ("sandbox_mode", "\"danger-full-access\""),
                    ],
                };
                for (key, literal) in values {
                    text = toml_set_root_key(&text, key, Some(literal));
                }
            }
            ("codex_reasoning_effort", RuntimeValue::Text(value)) => {
                let literal = string_literal(value);
                text = toml_set_root_key(&text, "model_reasoning_effort", literal.as_deref());
            }
            ("codex_web_search", RuntimeValue::Text(value)) => {
                let literal = string_literal(value);
                text = toml_set_root_key(&text, "web_search", literal.as_deref());
            }
            ("codex_context_window", RuntimeValue::Number(value)) => {
                let literal = value.map(|number| number.to_string());
                text = toml_set_root_key(&text, "model_context_window", literal.as_deref());
            }
            ("codex_auto_compact_token_limit", RuntimeValue::Number(value)) => {
                let literal = value.map(|number| number.to_string());
                text =
                    toml_set_root_key(&text, "model_auto_compact_token_limit", literal.as_deref());
            }
            _ => {}
        }
    }
    text
}

/// Codex 的目标文件绝对路径：配置文件就在 CODEX_HOME 下。
pub(crate) fn codex_target(ctx: &Context, config_path: Option<&str>) -> Result<std::path::PathBuf> {
    let target = match config_path {
        Some(raw) => crate::native_mcp::targets::expand_home(raw, &ctx.home_dir),
        None => ctx.home_dir.join(".codex").join(CODEX_CONFIG),
    };
    if target.file_name().and_then(|name| name.to_str()) != Some(CODEX_CONFIG) {
        bail!(
            "Codex 普通运行设置只允许写 config.toml；认证文件必须通过独立的显式认证操作修改：{}",
            target.display()
        );
    }
    Ok(target)
}

/// 目标文件的绝对路径。
pub(crate) fn claude_target(ctx: &Context) -> std::path::PathBuf {
    ctx.home_dir.join(CLAUDE_SETTINGS)
}

/// 本工具维护的状态栏脚本位置。
pub(crate) fn statusline_target(ctx: &Context) -> std::path::PathBuf {
    ctx.data_dir.join(STATUSLINE_FILE)
}

/// 部署内置状态栏脚本；内容没变就不重写，避免每次应用都动文件时间。
fn deploy_statusline(path: &std::path::Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if std::fs::read(path).ok().as_deref() != Some(STATUSLINE_BODY.as_bytes()) {
        std::fs::write(path, STATUSLINE_BODY)?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

// ---------- 实际接入：解析与识别当前项 ----------

/// 其中两个令牌键：哪个有值就走哪个认证头。
const CLAUDE_TOKEN_PATHS: &[&[&str]] = &[
    &["env", "ANTHROPIC_AUTH_TOKEN"],
    &["env", "ANTHROPIC_API_KEY"],
];

/// 接入键的切片。空字符串与没写等价（写入时整组接入键会先清掉），不因它判成未保存。
fn connection_slice(root: &Value) -> Value {
    let mut slice = Map::new();
    for path in CLAUDE_CONNECTION_PATHS {
        if let Some(found) = path_get(root, path).filter(|value| value.as_str() != Some("")) {
            path_set(&mut slice, path, found.clone());
        }
    }
    Value::Object(slice)
}

/// 实际令牌：AUTH_TOKEN 优先，与渲染时两者只写一个一致。
fn claude_live_token(root: &Value) -> Option<(AuthKind, String)> {
    if let Some(token) = string_at(root, &["env", "ANTHROPIC_AUTH_TOKEN"]) {
        return Some((AuthKind::Token, token.to_string()));
    }
    string_at(root, &["env", "ANTHROPIC_API_KEY"])
        .map(|token| (AuthKind::ApiKey, token.to_string()))
}

/// 一套接入配置写到空文档后的接入部分；令牌取不到值等渲染失败时为 None。
fn claude_rendered_connection(
    profile: &ConnectionProfile,
    resolve: &dyn Fn(&str) -> Option<String>,
) -> Option<Value> {
    let mut root = Map::new();
    write_claude_connection(&mut root, Some(profile), resolve).ok()?;
    Some(connection_slice(&Value::Object(root)))
}

/// Codex 实际文件里与接入有关的部分。
struct CodexLive {
    text: String,
    /// `model_provider`；缺省即内置 openai。
    provider: Option<String>,
    /// `model_provider` 指向的表；内置或不存在为 None。
    table: Option<toml::value::Table>,
    model: Option<String>,
}

impl CodexLive {
    fn provider_id(&self) -> &str {
        self.provider.as_deref().unwrap_or("openai")
    }

    fn on_openai(&self) -> bool {
        self.provider_id() == "openai"
    }

    fn table_text(&self, key: &str) -> Option<&str> {
        self.table
            .as_ref()?
            .get(key)
            .and_then(toml::Value::as_str)
            .filter(|value| !value.is_empty())
    }

    fn table_timeout(&self) -> Option<u64> {
        self.table
            .as_ref()?
            .get("stream_idle_timeout_ms")
            .and_then(toml::Value::as_integer)
            .and_then(|ms| u64::try_from(ms).ok())
            .filter(|ms| *ms > 0)
    }
}

/// 助手实际文件的解析结果。解析不了的文件在写路径上一律拒绝，不当成空文档覆盖。
enum Live {
    /// Claude：整份 settings.json（对象）。
    Claude(Value),
    Codex(CodexLive),
}

impl Live {
    fn read(kind: Kind, bytes: &[u8]) -> Result<Self> {
        match kind {
            Kind::Claude => {
                if bytes.is_empty() {
                    return Ok(Live::Claude(Value::Object(Map::new())));
                }
                match serde_json::from_slice::<Value>(bytes) {
                    Ok(value @ Value::Object(_)) => Ok(Live::Claude(value)),
                    _ => bail!(
                        "{CLAUDE_SETTINGS} 不是合法的 JSON 对象；为免覆盖你的内容，拒绝写入，请先修好它"
                    ),
                }
            }
            Kind::Codex => {
                let invalid = || {
                    anyhow::anyhow!(
                        "{CODEX_CONFIG} 不是合法的 TOML；为免覆盖你的内容，拒绝写入，请先修好它"
                    )
                };
                let text = std::str::from_utf8(bytes).map_err(|_| invalid())?;
                let root = text.parse::<toml::Value>().map_err(|_| invalid())?;
                let provider = root
                    .get("model_provider")
                    .and_then(toml::Value::as_str)
                    .map(str::to_string);
                let table = provider
                    .as_deref()
                    .and_then(|id| root.get("model_providers")?.get(id))
                    .and_then(toml::Value::as_table)
                    .cloned();
                let model = root
                    .get("model")
                    .and_then(toml::Value::as_str)
                    .filter(|model| !model.is_empty())
                    .map(str::to_string);
                Ok(Live::Codex(CodexLive {
                    text: text.to_string(),
                    provider,
                    table,
                    model,
                }))
            }
        }
    }

    /// 没有任何接入键 = 助手默认账号。
    fn is_default(&self) -> bool {
        match self {
            Live::Claude(root) => connection_slice(root)
                .as_object()
                .is_some_and(Map::is_empty),
            Live::Codex(live) => live.on_openai() && live.model.is_none(),
        }
    }

    /// 本工具认领的 provider 表（库里记录 + 旧受管注释）；Claude 没有这一项。
    fn claimed(&self, recorded: &[String]) -> BTreeSet<String> {
        match self {
            Live::Claude(_) => BTreeSet::new(),
            Live::Codex(live) => codex_claimed_providers(&live.text, recorded),
        }
    }
}

enum Match {
    Yes,
    No,
    /// 地址、模型等都对上，只是令牌变量在本进程取不到值，无法核对令牌。
    Unverified,
}

/// 已保存接入与实际接入逐项相等：直接用写入函数把它渲染一遍再比，不另写一套比较规则。
fn profile_matches(name: &str, profile: &ConnectionProfile, live: &Live) -> Match {
    match live {
        Live::Claude(root) => {
            let actual = connection_slice(root);
            if claude_rendered_connection(profile, &crate::secret_values::lookup).as_ref()
                == Some(&actual)
            {
                return Match::Yes;
            }
            // 令牌变量取不到值：假设它就是实际令牌再比一次，对得上就是「无法核对」
            let unresolved = profile
                .token_var
                .as_deref()
                .is_some_and(|var| crate::secret_values::lookup(var).is_none());
            match claude_live_token(root) {
                Some((_, token))
                    if unresolved
                        && claude_rendered_connection(profile, &|_| Some(token.clone()))
                            .as_ref()
                            == Some(&actual) =>
                {
                    Match::Unverified
                }
                _ => Match::No,
            }
        }
        Live::Codex(live) => {
            if codex_matches(name, profile, live) {
                Match::Yes
            } else {
                Match::No
            }
        }
    }
}

/// Codex：比较语义内容（接入方式 + 模型），忽略表名与显示名。
fn codex_matches(name: &str, profile: &ConnectionProfile, live: &CodexLive) -> bool {
    if live.model.as_deref() != profile.model.as_deref() {
        return false;
    }
    if let Some(reference) = profile.provider_ref.as_deref() {
        return live.provider_id() == reference;
    }
    if profile.base_url.is_none() && profile.token_var.is_none() {
        return live.on_openai();
    }
    match (&live.table, codex_rendered_table(name, profile)) {
        (Some(actual), Ok(expected)) => {
            !live.on_openai() && without_name(actual) == without_name(&expected)
        }
        _ => false,
    }
}

/// 实际接入与上次写入的那套「身份」是否一致：Claude 看地址 + 认证头，Codex 看地址 +
/// 令牌变量（且仍是本工具写的那张表）或引用的是同一个接入方式。身份一致时，
/// 外部只改了模型、别名、超时等，就回填进它，而不是每次都新建导入项。
fn same_identity(
    name: &str,
    profile: &ConnectionProfile,
    live: &Live,
    claimed: &BTreeSet<String>,
) -> bool {
    match live {
        Live::Claude(root) => {
            profile.provider_ref.is_none()
                && profile.base_url.as_deref() == string_at(root, &["env", "ANTHROPIC_BASE_URL"])
                && profile.token_var.as_ref().map(|_| profile.auth)
                    == claude_live_token(root).map(|(kind, _)| kind)
        }
        Live::Codex(live) => {
            if let Some(reference) = profile.provider_ref.as_deref() {
                return live.provider_id() == reference;
            }
            if profile.base_url.is_none() && profile.token_var.is_none() {
                return live.on_openai();
            }
            live.provider.as_deref() == Some(name)
                && claimed.contains(name)
                && live.table_text("base_url") == profile.base_url.as_deref()
                && live.table_text("env_key") == profile.token_var.as_deref()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Current {
    /// 助手默认账号：没有任何接入键。
    Default,
    /// 与某套已保存接入逐项一致。
    Profile(String),
    /// 实际接入对不上任何已保存项。
    Unsaved,
}

struct Identified {
    current: Current,
    unverified: Vec<String>,
    /// 未保存、但与上次写入的那套身份一致：切换时回填进它。
    drifted_from: Option<String>,
}

impl Identified {
    fn profile(&self) -> Option<&str> {
        match &self.current {
            Current::Profile(name) => Some(name),
            _ => None,
        }
    }

    fn to_json(&self, settings: &AgentSettings) -> Value {
        json!({
            "profile": self.profile(),
            "default": self.current == Current::Default,
            "last_written": settings.active,
            "unverified": self.unverified,
            "drifted_from": self.drifted_from,
        })
    }
}

/// 按内容识别当前接入。`active` 只在内容相同的候选之间做平局裁决，不单独断言当前项。
fn identify(live: &Live, settings: &AgentSettings, claimed: &BTreeSet<String>) -> Identified {
    let mut candidates = Vec::new();
    let mut unverified = Vec::new();
    for (name, profile) in &settings.profiles {
        match profile_matches(name, profile, live) {
            Match::Yes => candidates.push(name.clone()),
            Match::Unverified => unverified.push(name.clone()),
            Match::No => {}
        }
    }
    let active = settings.active.as_deref();
    let current = if let Some(name) = active.filter(|name| candidates.iter().any(|c| c == name)) {
        Current::Profile(name.to_string())
    } else if live.is_default() {
        Current::Default
    } else if let Some(first) = candidates.first() {
        Current::Profile(first.clone())
    } else {
        Current::Unsaved
    };
    let drifted_from = if current == Current::Unsaved {
        active
            .filter(|name| {
                settings
                    .profiles
                    .get(*name)
                    .is_some_and(|profile| same_identity(name, profile, live, claimed))
            })
            .map(str::to_string)
    } else {
        None
    };
    Identified {
        current,
        unverified,
        drifted_from,
    }
}

// ---------- 回填：切换前把还没保存的实际接入存进库 ----------

/// 令牌的处理结果（只记变量名，不记取值）。
#[derive(Debug, Clone, PartialEq, Eq)]
enum TokenOutcome {
    /// 实际没有令牌。
    None,
    /// 实际令牌与该项自己的变量解析值一致。
    Kept,
    /// 已有变量（别的接入或密钥文件里）的值就是它。
    Reused(String),
    /// 写进了用户选择的外部密钥文件。
    Adopted(String),
    /// 用户显式放弃了它。
    Discarded,
}

impl TokenOutcome {
    fn label(&self) -> &'static str {
        match self {
            TokenOutcome::None => "none",
            TokenOutcome::Kept => "kept",
            TokenOutcome::Reused(_) => "reused",
            TokenOutcome::Adopted(_) => "adopted",
            TokenOutcome::Discarded => "discarded",
        }
    }
}

struct Backfill {
    name: String,
    /// true = 新建的导入项；false = 回填进上次写入的那套。
    created: bool,
    profile: ConnectionProfile,
    token: TokenOutcome,
    /// 令牌取值（仅在写进密钥文件时用于本次渲染，不落库、不进日志）。
    adopted_value: Option<String>,
    secrets_write: Option<crate::secret_values::PlannedWrite>,
}

fn unrepresentable(reasons: &[String]) -> DispatchError {
    DispatchError::new(
        "AGENT_LIVE_UNREPRESENTABLE",
        format!(
            "当前实际接入配置无法完整保存，切换会丢失这些内容：{}。确认放弃当前实际配置后可带 --discard-live 重试",
            reasons.join("；")
        ),
    )
}

fn agent_error(err: anyhow::Error) -> DispatchError {
    DispatchError::new("AGENT_ERROR", format!("{err:#}"))
}

/// 新导入项的内部名：带时间，避开已有接入与（Codex）文件里已有的表名。
fn import_name(settings: &AgentSettings, live: &Live) -> String {
    let base = format!("imported-{}", chrono::Local::now().format("%Y%m%d-%H%M%S"));
    let taken = |name: &str| {
        settings.profiles.contains_key(name)
            || matches!(live, Live::Codex(codex) if toml_has_provider(&codex.text, name))
    };
    let mut name = base.clone();
    let mut index = 2;
    while taken(&name) {
        name = format!("{base}-{index}");
        index += 1;
    }
    name
}

fn import_label() -> String {
    format!(
        "导入的配置（{}）",
        chrono::Local::now().format("%Y-%m-%d %H:%M")
    )
}

/// 写进密钥文件时用的新变量名：避开密钥文件、进程环境与任何接入已经在用的名字，
/// 不改变别的接入的取值来源。
fn adopted_var_name(owner: &str, settings: &AgentSettings) -> String {
    let mut stem: String = owner
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();
    if !stem.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_') {
        stem.insert_str(0, "AM_");
    }
    let base = format!("{stem}_TOKEN");
    let stored = crate::secret_values::stored_names();
    let taken = |name: &str| {
        stored.contains(name)
            || std::env::var_os(name).is_some()
            || settings
                .profiles
                .values()
                .any(|profile| profile.token_var.as_deref() == Some(name))
    };
    let mut name = base.clone();
    let mut index = 2;
    while taken(&name) {
        name = format!("{base}_{index}");
        index += 1;
    }
    name
}

/// 实际令牌还没保存时怎么办：复用已有变量 → 显式放弃 → 写进已选的密钥文件 → 拒绝。
fn plan_token(
    ctx: &Context,
    value: &str,
    owner: &str,
    settings: &AgentSettings,
    discard: bool,
) -> Result<(TokenOutcome, Option<crate::secret_values::PlannedWrite>), DispatchError> {
    for profile in settings.profiles.values() {
        if let Some(var) = profile.token_var.as_deref() {
            if crate::secret_values::lookup(var).as_deref() == Some(value) {
                return Ok((TokenOutcome::Reused(var.to_string()), None));
            }
        }
    }
    if let Some(var) = crate::secret_values::var_holding(value) {
        return Ok((TokenOutcome::Reused(var), None));
    }
    if discard {
        return Ok((TokenOutcome::Discarded, None));
    }
    if crate::secret_values::source_ready() {
        let var = adopted_var_name(owner, settings);
        let write = crate::secret_values::plan_merge(
            ctx,
            &BTreeMap::from([(var.clone(), value.to_string())]),
        )
        .map_err(agent_error)?;
        return Ok((TokenOutcome::Adopted(var), write));
    }
    Err(DispatchError::new(
        "AGENT_LIVE_TOKEN_UNSAVED",
        "当前实际配置里的令牌还没有保存在任何变量里，切换会丢失它：先到“设置 → 密钥”选择外部密钥文件（令牌会写进该文件，库里只存变量名），或确认放弃这个令牌后带 --discard-live-token 重试".to_string(),
    ))
}

/// 规划回填：身份与上次写入的那套一致就更新它，否则新建导入项。
///
/// 回填后的接入配置必须能把实际接入原样渲染回来，否则切换会悄悄丢内容，一律拒绝
/// （用户可以显式带 `--discard-live` 放弃）。所有检查都在任何写入之前完成。
fn plan_backfill(
    ctx: &Context,
    live: &Live,
    settings: &AgentSettings,
    identified: &Identified,
    claimed: &BTreeSet<String>,
    discard_token: bool,
) -> Result<Backfill, DispatchError> {
    let (name, created) = match identified.drifted_from.clone() {
        Some(active) => (active, false),
        None => (import_name(settings, live), true),
    };
    let mut profile = if created {
        ConnectionProfile {
            label: Some(import_label()),
            ..ConnectionProfile::default()
        }
    } else {
        settings.profiles[&name].clone()
    };
    let mut reasons = Vec::new();
    let mut token = TokenOutcome::None;
    let mut adopted_value = None;
    let mut secrets_write = None;

    match live {
        Live::Claude(root) => {
            let text = |key: &str| string_at(root, &["env", key]).map(str::to_string);
            profile.base_url = text("ANTHROPIC_BASE_URL");
            profile.model = text("ANTHROPIC_MODEL");
            profile.small_fast_model = text("ANTHROPIC_SMALL_FAST_MODEL");
            profile.model_aliases = [
                ("opus", "ANTHROPIC_DEFAULT_OPUS_MODEL"),
                ("sonnet", "ANTHROPIC_DEFAULT_SONNET_MODEL"),
                ("haiku", "ANTHROPIC_DEFAULT_HAIKU_MODEL"),
            ]
            .into_iter()
            .filter_map(|(tier, key)| text(key).map(|value| (tier.to_string(), value)))
            .collect();
            profile.timeout_ms = text("API_TIMEOUT_MS")
                .and_then(|raw| raw.parse::<u64>().ok())
                .filter(|ms| *ms > 0);

            // 令牌键自身的问题：两个都有只能存一个；不是非空字符串的存不下
            let present = |path: &[&str]| {
                path_get(root, path).is_some_and(|value| value.as_str() != Some(""))
            };
            if CLAUDE_TOKEN_PATHS.iter().all(|path| present(path)) {
                reasons.push(
                    "同时设置了 ANTHROPIC_AUTH_TOKEN 与 ANTHROPIC_API_KEY，只能保存其中一个"
                        .to_string(),
                );
            }
            for path in CLAUDE_TOKEN_PATHS {
                if present(path) && !path_get(root, path).is_some_and(Value::is_string) {
                    reasons.push(format!("{} 不是非空字符串", path.join(".")));
                }
            }
            let live_token = claude_live_token(root);
            if let Some((auth, _)) = &live_token {
                profile.auth = *auth;
            }
            // 其余接入键：按回填结果重新渲染，必须逐项与实际一致
            let mut probe = profile.clone();
            probe.token_var = live_token.as_ref().map(|_| "AM_PROBE".to_string());
            let actual = connection_slice(root);
            match claude_rendered_connection(&probe, &|_| {
                live_token.as_ref().map(|(_, value)| value.clone())
            }) {
                Some(rendered) => {
                    for path in CLAUDE_CONNECTION_PATHS {
                        if !CLAUDE_TOKEN_PATHS.contains(path)
                            && path_get(&rendered, path) != path_get(&actual, path)
                        {
                            reasons.push(format!("{} 的取值无法按原样保存", path.join(".")));
                        }
                    }
                }
                None => reasons.push("接入键无法按原样保存".to_string()),
            }
            if let Err(err) = parse_profile(&name, &Value::Object(profile_json(&profile))) {
                reasons.push(format!("{err:#}"));
            }
            if !reasons.is_empty() {
                return Err(unrepresentable(&reasons));
            }

            match live_token {
                None => profile.token_var = None,
                Some((_, value)) => {
                    let own = profile
                        .token_var
                        .as_deref()
                        .and_then(crate::secret_values::lookup);
                    if !created && own.as_deref() == Some(value.as_str()) {
                        token = TokenOutcome::Kept;
                    } else {
                        let (outcome, write) =
                            plan_token(ctx, &value, &name, settings, discard_token)?;
                        match &outcome {
                            TokenOutcome::Reused(var) => profile.token_var = Some(var.clone()),
                            TokenOutcome::Adopted(var) => {
                                profile.token_var = Some(var.clone());
                                adopted_value = Some(value.clone());
                            }
                            // 放弃：新建项不带令牌变量；回填进已有项时保留它原来的变量
                            TokenOutcome::Discarded if created => profile.token_var = None,
                            _ => {}
                        }
                        token = outcome;
                        secrets_write = write;
                    }
                }
            }
        }
        Live::Codex(live) => {
            // 本工具自己写的表：切走时会被撤掉，内容必须能完整存进接入配置
            let copy_claimed_table =
                |profile: &mut ConnectionProfile, reasons: &mut Vec<String>| {
                    profile.base_url = live.table_text("base_url").map(str::to_string);
                    profile.token_var = live.table_text("env_key").map(str::to_string);
                    profile.timeout_ms = live.table_timeout();
                    let Some(actual) = live.table.as_ref() else {
                        return;
                    };
                    match codex_rendered_table(&name, profile) {
                        Ok(expected) => {
                            let (actual, expected) =
                                (without_name(actual), without_name(&expected));
                            let keys: BTreeSet<&String> =
                                actual.keys().chain(expected.keys()).collect();
                            for key in keys {
                                if actual.get(key) != expected.get(key) {
                                    reasons.push(format!(
                                        "model_providers.{}.{key} 无法按原样保存",
                                        live.provider_id()
                                    ));
                                }
                            }
                        }
                        Err(err) => reasons.push(format!("{err:#}")),
                    }
                };
            profile.model = live.model.clone();
            if created {
                if live.on_openai() {
                    // 内置官方 + 自定义模型：一套只带模型的接入即可
                } else if claimed.contains(live.provider_id()) && live.table.is_some() {
                    if let Some(display) = live.table_text("name") {
                        profile.label = Some(format!("{display}（导入）"));
                    }
                    copy_claimed_table(&mut profile, &mut reasons);
                } else {
                    // 用户自己的表或内置接入方式：只引用，不复制也不认领，那张表原样留在文件里
                    profile.provider_ref = Some(live.provider_id().to_string());
                    profile.label = Some(
                        live.table_text("name")
                            .unwrap_or(live.provider_id())
                            .to_string(),
                    );
                }
            } else if profile.provider_ref.is_none()
                && (profile.base_url.is_some() || profile.token_var.is_some())
            {
                copy_claimed_table(&mut profile, &mut reasons);
            }
            if let Err(err) = parse_profile(&name, &Value::Object(profile_json(&profile))) {
                reasons.push(format!("{err:#}"));
            }
            if !reasons.is_empty() {
                return Err(unrepresentable(&reasons));
            }
        }
    }
    Ok(Backfill {
        name,
        created,
        profile,
        token,
        adopted_value,
        secrets_write,
    })
}

// ---------- 库里的接入配置 JSON ----------

/// 接入配置里本工具认识的字段；其余（备注、将来的扩展字段）写库时原样保留。
const PROFILE_FIELDS: &[&str] = &[
    "label",
    "base_url",
    "auth",
    "token_var",
    "model",
    "small_fast_model",
    "model_aliases",
    "timeout_ms",
    "provider_ref",
];

fn profile_json(profile: &ConnectionProfile) -> Map<String, Value> {
    let mut out = Map::new();
    let mut text = |key: &str, value: &Option<String>| {
        if let Some(value) = value {
            out.insert(key.to_string(), Value::String(value.clone()));
        }
    };
    text("label", &profile.label);
    text("base_url", &profile.base_url);
    text("token_var", &profile.token_var);
    text("model", &profile.model);
    text("small_fast_model", &profile.small_fast_model);
    text("provider_ref", &profile.provider_ref);
    if profile.token_var.is_some() && profile.auth == AuthKind::ApiKey {
        out.insert("auth".into(), Value::String("api_key".into()));
    }
    if !profile.model_aliases.is_empty() {
        out.insert(
            "model_aliases".into(),
            Value::Object(
                profile
                    .model_aliases
                    .iter()
                    .map(|(tier, model)| (tier.clone(), Value::String(model.clone())))
                    .collect(),
            ),
        );
    }
    if let Some(ms) = profile.timeout_ms {
        out.insert("timeout_ms".into(), Value::Number(ms.into()));
    }
    out
}

/// 用结构化的接入配置更新库里那一项：认识的字段逐个替换或删除（保留原位置），其余原样。
fn merge_profile_json(existing: Option<&Value>, profile: &ConnectionProfile) -> Map<String, Value> {
    let mut object = existing
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let known = profile_json(profile);
    for key in PROFILE_FIELDS {
        match known.get(*key) {
            Some(value) => {
                object.insert((*key).to_string(), value.clone());
            }
            None => {
                object.remove(*key);
            }
        }
    }
    object
}

// ---------- 落地 ----------

use crate::native::{DispatchError, Request};
use crate::native_mcp::rollback::{atomic_write_bytes_checked, read_existing_bytes, RollbackGuard};
use crate::store;
use rusqlite::{Connection, TransactionBehavior};
use serde_json::json;
use std::path::{Path, PathBuf};

const BINDINGS_DDL: &str = "CREATE TABLE IF NOT EXISTS agent_settings_bindings (
    agent_id TEXT PRIMARY KEY NOT NULL,
    file TEXT NOT NULL,
    applied_digest TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    -- 末列：旧库补列时 ALTER TABLE 只能追加到末尾，新建库与补列库的列顺序保持一致。
    -- Codex 上次写入的 provider id（JSON 字符串数组）：哪些表是本工具写的只记在这里。
    owned_providers TEXT NOT NULL DEFAULT '[]'
)";

fn ensure_table(conn: &Connection) -> Result<()> {
    conn.execute_batch(BINDINGS_DDL)?;
    // 旧库补列：补出来的行取 '[]'，旧版本写的表靠文件里的旧注释认领，下一次写入即记上
    if !has_owned_providers_column(conn)? {
        conn.execute_batch(
            "ALTER TABLE agent_settings_bindings
             ADD COLUMN owned_providers TEXT NOT NULL DEFAULT '[]'",
        )?;
    }
    Ok(())
}

fn has_owned_providers_column(conn: &Connection) -> Result<bool> {
    let mut stmt = conn.prepare("PRAGMA table_info(agent_settings_bindings)")?;
    let found = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .iter()
        .any(|name| name == "owned_providers");
    Ok(found)
}

/// 库里记下的 Codex 上次写入的 provider id；没写过为空。
/// 只读：缺库、缺表、旧库缺列都按 `[]` 理解，不在这里建库或补列。
fn recorded_providers(ctx: &Context, agent_id: &str) -> Result<Vec<String>> {
    let Some(conn) = store::open_readonly(ctx)? else {
        return Ok(Vec::new());
    };
    let exists: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='agent_settings_bindings'",
        [],
        |row| row.get(0),
    )?;
    if exists == 0 || !has_owned_providers_column(&conn)? {
        return Ok(Vec::new());
    }
    let raw: Option<String> = conn
        .query_row(
            "SELECT owned_providers FROM agent_settings_bindings WHERE agent_id = ?1",
            [agent_id],
            |row| row.get(0),
        )
        .map(Some)
        .or_else(|err| match err {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })?;
    let Some(raw) = raw else {
        return Ok(Vec::new());
    };
    serde_json::from_str::<Vec<String>>(&raw).map_err(|_| {
        anyhow::anyhow!("agent_settings_bindings.owned_providers 不是字符串数组，库可能已损坏")
    })
}

/// 写入基准：本工具负责的那部分的摘要。Codex 额外算上当前引用的 provider 表——
/// 界面展示、回填的正是它。
fn base_digest(kind: Kind, bytes: &[u8], recorded: &[String], statusline_path: &Path) -> String {
    match kind {
        Kind::Claude => owned_digest(bytes, statusline_path),
        Kind::Codex => {
            let text = String::from_utf8_lossy(bytes);
            let mut claimed = recorded.to_vec();
            let current = text.parse::<toml::Value>().ok().and_then(|root| {
                root.get("model_provider")
                    .and_then(toml::Value::as_str)
                    .map(str::to_string)
            });
            if let Some(id) = current {
                if !claimed.contains(&id) {
                    claimed.push(id);
                }
            }
            codex_owned_digest(&text, &claimed)
        }
    }
}

/// 该形态的目标文件。
fn target_path(ctx: &Context, kind: Kind, config_path: Option<&str>) -> Result<PathBuf> {
    match kind {
        Kind::Claude => Ok(claude_target(ctx)),
        Kind::Codex => codex_target(ctx, config_path),
    }
}

/// 写入落点：符号链接解析到最终目标（链接本身保持不变），目标必须是普通文件；
/// 文件还不存在时只解析它的父目录。hooks 域写同一类助手配置文件，沿用这一裁决。
pub(crate) fn resolve_write_target(path: &Path) -> Result<PathBuf> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => {
            let real = std::fs::canonicalize(path).map_err(|_| {
                anyhow::anyhow!("{} 是指向不存在目标的符号链接，拒绝写入", path.display())
            })?;
            if !real.is_file() {
                bail!("{} 不是普通文件，拒绝写入", path.display());
            }
            if matches!(
                real.file_name().and_then(|name| name.to_str()),
                Some("auth.json" | ".credentials.json")
            ) {
                bail!("{} 最终指向认证文件，拒绝写入", path.display());
            }
            Ok(real)
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            match (path.parent(), path.file_name()) {
                (Some(parent), Some(name)) if parent.exists() => {
                    Ok(std::fs::canonicalize(parent)?.join(name))
                }
                _ => Ok(path.to_path_buf()),
            }
        }
        Err(err) => Err(err.into()),
    }
}

/// 文件不存在时写出的空内容不值得落盘（切回默认之类不该凭空建一个空配置）。
fn is_empty_config(kind: Kind, bytes: &[u8]) -> bool {
    match kind {
        Kind::Claude => bytes == b"{}\n",
        Kind::Codex => bytes.iter().all(u8::is_ascii_whitespace),
    }
}

fn pretty_json(root: Map<String, Value>) -> Result<Vec<u8>> {
    let mut out = serde_json::to_vec_pretty(&Value::Object(root))?;
    out.push(b'\n');
    Ok(out)
}

enum Action {
    /// 运行参数补丁（原始 JSON，拿到助手形态后再校验）。
    Runtime(Value),
    /// 切换到某套接入；None = 助手默认账号。
    Switch(Option<String>),
    /// 只把实际配置存成接入配置，不写助手文件。
    SaveCurrent,
}

struct ConfigureArgs {
    id: String,
    base: String,
    action: Action,
    connection: Option<Value>,
    discard_live: bool,
    discard_live_token: bool,
    allow_yolo: bool,
}

fn configure_args(req: &Request) -> Result<ConfigureArgs, DispatchError> {
    let usage = |message: &str| DispatchError::new("USAGE_ERROR", message.to_string());
    let id = req
        .value("--only")
        .ok_or_else(|| usage("agents:configure 需要 --only <助手>"))?
        .trim_start_matches("agents:")
        .to_string();
    let runtime = req.value("--runtime");
    let use_name = req.value("--use");
    let use_default = req.flag("--use-default");
    let save_current = req.flag("--save-current");
    let chosen = [
        runtime.is_some(),
        use_name.is_some(),
        use_default,
        save_current,
    ]
    .into_iter()
    .filter(|given| *given)
    .count();
    if chosen != 1 {
        return Err(usage(
            "agents:configure 需要且只能给一种写法：--runtime <json>、--use <接入配置>、--use-default 或 --save-current",
        ));
    }
    let base = req
        .value("--base")
        .filter(|base| !base.is_empty())
        .ok_or_else(|| {
            usage(
                "agents:configure 需要 --base：先读 agents:status，把 current_config.base 原样带回",
            )
        })?
        .to_string();
    let action = if let Some(raw) = runtime {
        Action::Runtime(serde_json::from_str(raw).map_err(|_| usage("--runtime 必须是 JSON 对象"))?)
    } else if let Some(name) = use_name {
        Action::Switch(Some(name.to_string()))
    } else if use_default {
        Action::Switch(None)
    } else {
        Action::SaveCurrent
    };
    let connection = match req.value("--connection") {
        Some(raw) => Some(
            serde_json::from_str::<Value>(raw)
                .map_err(|_| usage("--connection 必须是 JSON 对象"))?,
        ),
        None => None,
    };
    if connection.is_some() && !matches!(action, Action::Switch(Some(_))) {
        return Err(usage("--connection 只能与 --use <接入配置> 一起用"));
    }
    let discard_live = req.flag("--discard-live");
    let discard_live_token = req.flag("--discard-live-token");
    if (discard_live || discard_live_token) && matches!(action, Action::Runtime(_)) {
        return Err(usage(
            "--discard-live / --discard-live-token 只用于切换接入或保存当前实际配置",
        ));
    }
    if discard_live && matches!(action, Action::SaveCurrent) {
        return Err(usage("保存当前实际配置时不能同时放弃它"));
    }
    Ok(ConfigureArgs {
        id,
        base,
        action,
        connection,
        discard_live,
        discard_live_token,
        allow_yolo: req.flag("--allow-yolo"),
    })
}

/// 这次写入要对库里 `settings` 做的改动；写前先在定义副本上演练一遍校验，事务里再照做。
struct SettingsChanges<'a> {
    backfill: Option<&'a Backfill>,
    connection: Option<(&'a str, &'a Value)>,
    /// Some(None) = 删掉 active（切回默认账号）。
    active: Option<Option<&'a str>>,
    patch: Option<&'a RuntimePatch>,
}

fn apply_settings_changes(settings: &mut Map<String, Value>, changes: &SettingsChanges) {
    let mut profiles = settings
        .get("profiles")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let mut profiles_changed = false;
    if let Some(plan) = changes.backfill {
        let mut merged = merge_profile_json(profiles.get(&plan.name), &plan.profile);
        if plan.created {
            merged.insert(
                "note".into(),
                Value::String("切换前从助手实际配置自动保存".into()),
            );
        }
        profiles.insert(plan.name.clone(), Value::Object(merged));
        profiles_changed = true;
    }
    if let Some((name, raw)) = changes.connection {
        profiles.insert(name.to_string(), raw.clone());
        profiles_changed = true;
    }
    if profiles_changed {
        settings.insert("profiles".into(), Value::Object(profiles));
    }
    match changes.active {
        Some(Some(name)) => {
            settings.insert("active".into(), Value::String(name.to_string()));
        }
        Some(None) => {
            settings.remove("active");
        }
        None => {}
    }
    if let Some(patch) = changes.patch {
        for (key, value) in &patch.0 {
            let stored = match value {
                RuntimeValue::Mode(mode) => {
                    // 旧版本的 yolo 开关与新值并存会让定义解析失败，写新值时一并撤掉
                    settings.remove("yolo");
                    Some(Value::String(mode.as_str().into()))
                }
                RuntimeValue::Statusline(StatusLine::Builtin) => {
                    Some(Value::String("builtin".into()))
                }
                RuntimeValue::Statusline(StatusLine::Untouched) => None,
                RuntimeValue::Flag(on) => on.then_some(Value::Bool(true)),
                RuntimeValue::Text(text) => text.clone().map(Value::String),
                RuntimeValue::Number(number) => number.map(|n| Value::Number(n.into())),
            };
            match stored {
                Some(value) => {
                    settings.insert((*key).to_string(), value);
                }
                None => {
                    settings.remove(*key);
                }
            }
        }
    }
}

/// 真实写入：`agents:configure --only <id> --base <摘要> <写法>`。
///
/// 写法四选一：`--runtime <json>` 只改补丁里的运行参数；`--use <接入配置>` /
/// `--use-default` 切换接入（可带 `--connection <json>` 先替换目标那套）；
/// `--save-current` 只把实际配置存进库。切换与保存前，还没保存的实际接入先回填进库；
/// 令牌没保存时按「复用 → 写进已选密钥文件 → `--discard-live-token` 放弃 → 拒绝」处理；
/// 存不下的实际内容拒绝，除非带 `--discard-live`。
///
/// 所有拒绝都在任何写入之前；写入阶段密钥文件、助手配置与库在同一个事务里，
/// 任一步失败都回滚。完全放开改的是助手的安全默认值，必须显式带 `--allow-yolo`。
pub(crate) fn apply(ctx: &Context, req: &Request) -> Result<Value, DispatchError> {
    let args = configure_args(req)?;
    let id = args.id.clone();

    let defs = crate::native_agents::load_definitions(ctx).map_err(agent_error)?;
    let def = defs
        .iter()
        .find(|d| d.id == id)
        .ok_or_else(|| DispatchError::new("AGENT_ERROR", format!("没有这个助手：{id}")))?;
    let kind = kind_for_bin(&def.bin).ok_or_else(|| {
        DispatchError::new(
            "AGENT_ERROR",
            "只有 Claude Code 与 Codex 形态的助手支持写运行参数".to_string(),
        )
    })?;
    let path = target_path(ctx, kind, def.config_path()).map_err(agent_error)?;
    let file_label = match kind {
        Kind::Claude => CLAUDE_SETTINGS,
        Kind::Codex => CODEX_CONFIG,
    };
    let statusline_path = statusline_target(ctx);

    // 重读实际文件：链接解析到最终目标；解析不了的文件拒绝，不当成空文档覆盖
    let real = resolve_write_target(&path).map_err(agent_error)?;
    let existing = read_existing_bytes(&real).map_err(agent_error)?;
    if existing
        .as_ref()
        .is_some_and(|bytes| bytes.len() as u64 > MAX_CLIENT_CONFIG_BYTES)
    {
        return Err(DispatchError::new(
            "AGENT_ERROR",
            format!("{file_label} 超过安全读取上限，拒绝写入"),
        ));
    }
    let bytes = existing.clone().unwrap_or_default();
    let live = Live::read(kind, &bytes).map_err(agent_error)?;
    let recorded = recorded_providers(ctx, &id).map_err(agent_error)?;
    if base_digest(kind, &bytes, &recorded, &statusline_path) != args.base {
        return Err(DispatchError::new(
            "AGENT_CONFIG_CHANGED",
            format!(
                "{file_label} 已在外部修改（与读取时不一致），文件与库都没有改动；请刷新后重试"
            ),
        ));
    }

    let settings = parse(def.spec_ref()).map_err(agent_error)?;
    let claimed = live.claimed(&recorded);
    let identified = identify(&live, &settings, &claimed);

    let mut patch = None;
    let mut backfill = None;
    let mut next = settings.clone();
    let mut new_bytes: Option<Vec<u8>> = None;
    let mut owned_after = recorded.clone();

    match &args.action {
        Action::Runtime(raw) => {
            let parsed = parse_runtime_patch(kind, raw)
                .map_err(|err| DispatchError::new("USAGE_ERROR", format!("{err:#}")))?;
            if parsed.0.is_empty() {
                return Err(DispatchError::new(
                    "USAGE_ERROR",
                    "--runtime 里没有要改的运行参数".to_string(),
                ));
            }
            if parsed.sets_full() && !args.allow_yolo {
                return Err(DispatchError::new(
                    "AGENT_ERROR",
                    "完全放开会关闭权限询问与沙箱，需要显式确认后再执行".to_string(),
                ));
            }
            new_bytes = Some(match &live {
                Live::Claude(root) => {
                    let mut root = root.as_object().cloned().unwrap_or_default();
                    write_claude_runtime(&mut root, &parsed, &statusline_path);
                    pretty_json(root).map_err(agent_error)?
                }
                Live::Codex(codex) => write_codex_runtime(&codex.text, &parsed).into_bytes(),
            });
            patch = Some(parsed);
        }
        Action::Switch(_) | Action::SaveCurrent => {
            // 目标先落定：指向不存在的配置一律拒绝，不静默落到「没有生效配置」
            if let Action::Switch(Some(name)) = &args.action {
                if let Some(raw) = &args.connection {
                    if !valid_profile_name(name) {
                        return Err(DispatchError::new(
                            "USAGE_ERROR",
                            format!("接入配置名 {name} 不合法：只能用字母、数字、连字符与下划线，最长 64 位"),
                        ));
                    }
                    let profile = parse_profile(name, raw).map_err(agent_error)?;
                    next.profiles.insert(name.clone(), profile);
                } else if !settings.profiles.contains_key(name) {
                    return Err(DispatchError::new(
                        "AGENT_ERROR",
                        format!("{id} 没有名为 {name} 的接入配置"),
                    ));
                }
            }
            if identified.current == Current::Unsaved && !args.discard_live {
                let plan = plan_backfill(
                    ctx,
                    &live,
                    &settings,
                    &identified,
                    &claimed,
                    args.discard_live_token,
                )?;
                // 编辑并切换到上次写入的那套时，用户这次提交的内容优先
                let overridden = matches!(&args.action, Action::Switch(Some(name)) if args.connection.is_some() && *name == plan.name);
                if !overridden {
                    next.profiles
                        .insert(plan.name.clone(), plan.profile.clone());
                }
                backfill = Some(plan);
            }
            if let Action::Switch(target) = &args.action {
                next.active = target.clone();
                let profile = target.as_deref().map(|name| &next.profiles[name]);
                // 刚写进密钥文件的令牌还没进缓存：本次渲染直接用它
                let adopted =
                    backfill
                        .as_ref()
                        .and_then(|plan| match (&plan.token, &plan.adopted_value) {
                            (TokenOutcome::Adopted(var), Some(value)) => {
                                Some((var.clone(), value.clone()))
                            }
                            _ => None,
                        });
                let resolve = |var: &str| match &adopted {
                    Some((name, value)) if name == var => Some(value.clone()),
                    _ => crate::secret_values::lookup(var),
                };
                new_bytes = Some(match &live {
                    Live::Claude(root) => {
                        let mut root = root.as_object().cloned().unwrap_or_default();
                        write_claude_connection(&mut root, profile, &resolve)
                            .map_err(agent_error)?;
                        pretty_json(root).map_err(agent_error)?
                    }
                    Live::Codex(codex) => {
                        let (text, written) = write_codex_connection(
                            &codex.text,
                            target.as_deref().zip(profile),
                            &recorded,
                        )
                        .map_err(agent_error)?;
                        owned_after = written;
                        text.into_bytes()
                    }
                });
            }
        }
    }

    if matches!(args.action, Action::SaveCurrent) && backfill.is_none() {
        // 已是某套已保存接入或默认账号：没有要保存的，零写
        return Ok(json!({
            "domain": "agents",
            "action": "configure",
            "element": format!("agents:{id}"),
            "target": path.display().to_string(),
            "mode": "save-current",
            "active": settings.active,
            "current": identified.profile(),
            "changed": [],
            "backfill": null,
            "base": args.base,
            "statusline": false,
        }));
    }

    let changes = SettingsChanges {
        backfill: backfill.as_ref().filter(|plan| {
            !matches!(&args.action, Action::Switch(Some(name)) if args.connection.is_some() && *name == plan.name)
        }),
        connection: match (&args.action, &args.connection) {
            (Action::Switch(Some(name)), Some(raw)) => Some((name.as_str(), raw)),
            _ => None,
        },
        active: match &args.action {
            Action::Switch(target) => Some(target.as_deref()),
            _ => None,
        },
        patch: patch.as_ref(),
    };
    // 写前演练：改动后的定义必须仍能被解析，否则一个字节都不写
    let mut preview = def
        .spec_ref()
        .get("settings")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    apply_settings_changes(&mut preview, &changes);
    let preview_settings = parse(&json!({"settings": preview})).map_err(agent_error)?;

    let file_write = new_bytes.filter(|new| match &existing {
        Some(old) => old != new,
        None => !is_empty_config(kind, new),
    });
    let final_bytes = file_write.clone().unwrap_or_else(|| bytes.clone());
    let new_base = base_digest(kind, &final_bytes, &owned_after, &statusline_path);
    let deploy = kind == Kind::Claude
        && patch
            .as_ref()
            .is_some_and(RuntimePatch::sets_builtin_statusline);
    let mode = match &args.action {
        Action::Runtime(_) => "runtime",
        Action::Switch(_) => "switch",
        Action::SaveCurrent => "save-current",
    };
    let target_name = match &args.action {
        Action::Switch(target) => target.clone(),
        _ => None,
    };
    let backfill_json = backfill.as_ref().map(|plan| {
        json!({
            "name": plan.name,
            "created": plan.created,
            "label": plan.profile.label,
            "token": plan.token.label(),
            "token_var": match &plan.token {
                TokenOutcome::Reused(var) | TokenOutcome::Adopted(var) => Some(var.clone()),
                _ => None,
            },
        })
    });
    let changed = patch.as_ref().map(RuntimePatch::keys).unwrap_or_default();
    let discarded_live = args.discard_live && identified.current == Current::Unsaved;
    let owned_json = serde_json::to_string(&owned_after).map_err(|err| agent_error(err.into()))?;

    let mut conn = store::open_write(ctx).map_err(agent_error)?;
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|err| agent_error(err.into()))?;
    let mut guard = RollbackGuard::new();
    let written = (|| -> Result<()> {
        ensure_table(&tx)?;
        if let Some(write) = backfill
            .as_ref()
            .and_then(|plan| plan.secrets_write.as_ref())
        {
            atomic_write_bytes_checked(&write.path, &write.new, Some(write.old.as_slice()))?;
            guard.record_written(&write.path, Some(write.old.clone()), write.new.clone());
        }
        // 脚本先就位再写 settings.json：反过来的话中途失败会留下一个指向不存在文件的状态栏
        if deploy {
            deploy_statusline(&statusline_path)?;
        }
        if let Some(content) = &file_write {
            // 替换前再按读到的字节比对一次，堵住读与写之间的外部修改
            atomic_write_bytes_checked(&real, content, existing.as_deref())?;
            guard.record_written(&real, existing.clone(), content.clone());
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                // 里面可能有明文令牌，按私密文件落权限
                std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o600))?;
            }
        }
        // 库：在事务里读到的定义上改 settings，未知键原样保留
        let mut policy = store::policy_from_connection(&tx)?;
        let definition = policy
            .get_mut("agents")
            .and_then(|agents| agents.get_mut("definitions"))
            .and_then(|definitions| definitions.get_mut(&id))
            .and_then(Value::as_object_mut)
            .ok_or_else(|| anyhow::anyhow!("库里没有助手 {id} 的定义"))?;
        let settings_value = definition
            .entry("settings".to_string())
            .or_insert_with(|| Value::Object(Map::new()));
        if !settings_value.is_object() {
            *settings_value = Value::Object(Map::new());
        }
        if let Some(settings_object) = settings_value.as_object_mut() {
            apply_settings_changes(settings_object, &changes);
        }
        store::write_policy(&tx, &policy)?;
        tx.execute(
            "INSERT INTO agent_settings_bindings
                 (agent_id, file, applied_digest, updated_at, owned_providers)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(agent_id) DO UPDATE SET file=excluded.file,
                 applied_digest=excluded.applied_digest, updated_at=excluded.updated_at,
                 owned_providers=excluded.owned_providers",
            rusqlite::params![
                &id,
                file_label,
                &new_base,
                chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                &owned_json
            ],
        )?;
        // 审计只记改了哪几项与变量名，不记任何取值
        store::record_operation(
            &tx,
            Some(&req.run_id),
            "agents:configure",
            Some(&id),
            Some(&json!({
                "file": file_label,
                "mode": mode,
                "target": target_name,
                "changed": changed,
                "backfill": backfill_json,
                "discarded_live": discarded_live,
                "statusline": deploy,
            })),
        )?;
        Ok(())
    })();
    if let Err(err) = written {
        drop(tx);
        if let Err(rollback) = guard.rollback() {
            return Err(agent_error(
                err.context(format!("回滚也失败了：{rollback:#}")),
            ));
        }
        return Err(agent_error(err));
    }
    if let Err(err) = tx.commit() {
        if let Err(rollback) = guard.rollback() {
            return Err(DispatchError::new(
                "AGENT_ERROR",
                format!("提交失败：{err}；回滚也失败了：{rollback:#}"),
            ));
        }
        return Err(agent_error(err.into()));
    }
    guard.commit();
    if backfill
        .as_ref()
        .is_some_and(|plan| plan.secrets_write.is_some())
    {
        crate::secret_values::refresh(ctx);
    }

    let current = Live::read(kind, &final_bytes).ok().map(|final_live| {
        let claimed = final_live.claimed(&owned_after);
        identify(&final_live, &preview_settings, &claimed)
    });
    Ok(json!({
        "domain": "agents",
        "action": "configure",
        "element": format!("agents:{id}"),
        "target": path.display().to_string(),
        "mode": mode,
        "active": preview_settings.active,
        "current": current.as_ref().and_then(Identified::profile),
        "changed": changed,
        "backfill": backfill_json,
        "discarded_live": discarded_live,
        "base": new_base,
        "statusline": deploy,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn no_secrets(_: &str) -> Option<String> {
        None
    }

    fn sl() -> std::path::PathBuf {
        std::path::PathBuf::from("/data/statusline.sh")
    }

    fn profile(entry: Value) -> ConnectionProfile {
        parse_profile("p", &entry).unwrap()
    }

    /// 只写接入键（测试用的小包装）：既有内容为基底，按目标那套写回。
    fn connect(
        existing: Option<&[u8]>,
        target: Option<&ConnectionProfile>,
        resolve: &dyn Fn(&str) -> Option<String>,
    ) -> Result<Vec<u8>> {
        let mut root = existing
            .map(|bytes| serde_json::from_slice::<Value>(bytes).unwrap())
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default();
        write_claude_connection(&mut root, target, resolve)?;
        pretty_json(root)
    }

    /// 只写运行参数补丁（测试用的小包装）。
    fn runtime(existing: Option<&[u8]>, patch: Value) -> Vec<u8> {
        let mut root = existing
            .map(|bytes| serde_json::from_slice::<Value>(bytes).unwrap())
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default();
        write_claude_runtime(
            &mut root,
            &parse_runtime_patch(Kind::Claude, &patch).unwrap(),
            &sl(),
        );
        pretty_json(root).unwrap()
    }

    fn codex_patch(patch: Value) -> RuntimePatch {
        parse_runtime_patch(Kind::Codex, &patch).unwrap()
    }

    const LEGACY_CODEX: &str = concat!(
        "model_provider = \"relay\"\n",
        "model = \"gpt-5\"\n",
        "approval_policy = \"on-request\"\n",
        "sandbox_mode = \"workspace-write\"\n",
        "approvals_reviewer = \"user\"\n",
        "service_tier = \"default\"\n",
        "\n",
        "# 我自己的接入\n",
        "[model_providers.mine]\n",
        "name = \"mine\"\n",
        "base_url = \"https://mine.example.com/v1\"\n",
        "\n",
        "# [agent-manager:managed]\n",
        "[model_providers.relay]\n",
        "name = \"relay\"\n",
        "base_url = \"https://relay.example.com/v1\"\n",
        "env_key = \"RELAY_KEY\"\n",
        "requires_openai_auth = true\n",
        "wire_api = \"responses\"\n",
        "\n",
        "# [agent-manager:managed]\n",
        "[mcp_servers.grok]\n",
        "command = \"grok\"\n",
    );

    /// 旧版本（写受管注释、靠注释认领）按同一份文件算出的摘要，改动前实际运行旧代码得到。
    const LEGACY_CODEX_DIGEST: &str =
        "c4f2c01c16017f3bc7814ff07743d5db0f63d7e89250290906c704e69d4569f9";

    fn relay_profile(name: &str) -> ConnectionProfile {
        parse_profile(
            name,
            &json!({"base_url": format!("https://{name}.example.com/v1"), "token_var": "RELAY_KEY"}),
        )
        .unwrap()
    }

    #[test]
    fn 带旧注释的既有文件摘要与旧版本一致_不被误判外部修改() {
        // 旧库没有 owned_providers，按 [] 计；旧注释照样认领
        assert_eq!(codex_owned_digest(LEGACY_CODEX, &[]), LEGACY_CODEX_DIGEST);
        // 升级后首次写入前库里就算已记上 relay，口径也不变
        assert_eq!(
            codex_owned_digest(LEGACY_CODEX, &["relay".to_string()]),
            LEGACY_CODEX_DIGEST
        );
        // 用户自己的 mine 不进摘要
        let slice = codex_owned_slice(LEGACY_CODEX, &[]);
        assert!(!slice.to_string().contains("mine.example.com"));
    }

    #[test]
    fn 带旧注释的既有文件写入后注释与旧表撤掉_新表不带注释_其余保留() {
        let (out, written) =
            write_codex_connection(LEGACY_CODEX, Some(("next", &relay_profile("next"))), &[])
                .unwrap();
        assert_eq!(written, vec!["next".to_string()]);
        assert!(!out.contains("[model_providers.relay]"), "{out}");
        assert!(out.contains("[model_providers.next]"), "{out}");
        assert!(
            !out.contains("# [agent-manager:managed]\n[model_providers"),
            "provider 表前不再有受管注释：{out}"
        );
        // 用户自己的表与注释逐字节保留
        assert!(out.contains(
            "# 我自己的接入\n[model_providers.mine]\nname = \"mine\"\nbase_url = \"https://mine.example.com/v1\"\n"
        ));
        assert!(out.contains("service_tier = \"default\""));
        // MCP 域的受管注释与表不归这里管
        assert!(out.contains("# [agent-manager:managed]\n[mcp_servers.grok]\ncommand = \"grok\"\n"));
        assert_eq!(out.matches("# [agent-manager:managed]").count(), 1);
    }

    #[test]
    fn 用户自己的同名_provider_含引号写法_未认领就拒绝_认领后撤掉再写() {
        let existing = concat!(
            "[model_providers.\"relay\"]\n",
            "base_url = \"https://hand.example.com\"\n",
            "[model_providers.\"relay\".http_headers]\n",
            "X = \"1\"\n",
            "[mcp_servers.relay]\n",
            "command = \"relay-mcp\"\n",
        );
        let relay = relay_profile("relay");
        let err = write_codex_connection(existing, Some(("relay", &relay)), &[]).unwrap_err();
        assert!(
            err.to_string()
                .contains("config.toml 中已有你自己的同名接入方式 relay，拒绝覆盖"),
            "{err}"
        );
        // 只有 MCP 同名表不算冲突
        let (_, written) = write_codex_connection(
            "[mcp_servers.relay]\ncommand = \"relay-mcp\"\n",
            Some(("relay", &relay)),
            &[],
        )
        .unwrap();
        assert_eq!(written, vec!["relay".to_string()]);

        let (out, written) =
            write_codex_connection(existing, Some(("relay", &relay)), &["relay".to_string()])
                .unwrap();
        assert_eq!(written, vec!["relay".to_string()]);
        let parsed: toml::Value = out.parse().unwrap();
        assert_eq!(
            parsed["model_providers"]["relay"]["base_url"].as_str(),
            Some("https://relay.example.com/v1")
        );
        assert!(!out.contains("hand.example.com"), "{out}");
        assert!(
            out.contains("[mcp_servers.relay]\ncommand = \"relay-mcp\"\n"),
            "{out}"
        );
    }

    #[test]
    fn 认领集合只含库里记录与旧注释_子表一起撤_别人的逐字节保留() {
        let existing = concat!(
            "[model_providers.a]\n",
            "name = \"a\"\n",
            "[model_providers.a.http_headers]\n",
            "X = \"1\"\n",
            "[model_providers.mine]\n",
            "base_url = \"https://mine.example.com\"\n",
            "[model_providers.\"quoted.id\"]\n",
            "base_url = \"https://q.example.com\"\n",
        );
        let claimed = codex_claimed_providers(existing, &["a".to_string()]);
        assert_eq!(claimed, BTreeSet::from(["a".to_string()]));
        let removed = toml_remove_provider_tables(existing, &claimed);
        assert_eq!(
            removed,
            concat!(
                "[model_providers.mine]\n",
                "base_url = \"https://mine.example.com\"\n",
                "[model_providers.\"quoted.id\"]\n",
                "base_url = \"https://q.example.com\"\n",
            )
        );
        // 带引号的 id 也能按记录认领
        let claimed = codex_claimed_providers(existing, &["quoted.id".to_string()]);
        let removed = toml_remove_provider_tables(existing, &claimed);
        assert!(!removed.contains("quoted.id"), "{removed}");
        assert!(removed.contains("[model_providers.a.http_headers]"));
    }

    #[test]
    fn 解析拒绝非法地址_变量名_配置名与认证方式() {
        // 实际配置里已有的地址不能因为这里的校验存不下：任意主机的 http 放行，其余协议仍拒绝
        let bad_url =
            json!({"settings": {"profiles": {"p": {"base_url": "ftp://evil.example.com"}}}});
        assert!(parse(&bad_url).is_err());
        assert!(parse(
            &json!({"settings": {"profiles": {"p": {"base_url": "http://192.168.1.10:8080"}}}})
        )
        .is_ok());
        // 引用 config.toml 里已有的接入方式：地址、令牌变量与超时由那张表决定，不能再单独填
        assert!(parse(
            &json!({"settings": {"profiles": {"p": {"provider_ref": "kimi", "model": "k2"}}}})
        )
        .is_ok());
        assert!(parse(&json!({"settings": {"profiles": {"p": {"provider_ref": "kimi", "base_url": "https://x"}}}})).is_err());
        assert!(
            parse(&json!({"settings": {"profiles": {"p": {"token_var": "has-dash"}}}})).is_err()
        );
        assert!(parse(&json!({"settings": {"profiles": {"bad name": {}}}})).is_err());
        assert!(parse(&json!({"settings": {"profiles": {"p": {"auth": "basic"}}}})).is_err());
        assert!(parse(&json!({"settings": {"statusline": "custom"}})).is_err());
        assert!(parse(&json!({"settings": {"claude_effort_level": "ultra"}})).is_err());
        assert!(parse(&json!({"settings": {"codex_reasoning_effort": "max"}})).is_err());
        assert!(parse(&json!({"settings": {"codex_context_window": 0}})).is_err());
        assert!(parse(&json!({"settings": {"codex_web_search": "auto"}})).is_err());
        // active 指向不存在的配置要当场拒绝，不静默落空
        assert!(parse(&json!({"settings": {"active": "nope", "profiles": {"p": {}}}})).is_err());
        // 没有 settings 段 = 没配过
        assert_eq!(
            parse(&json!({"bin": "claude"})).unwrap(),
            AgentSettings::default()
        );
    }

    #[test]
    fn 只改自己那几条路径_别人的原样保留() {
        let existing = serde_json::to_vec(&json!({
            "hooks": {"SessionStart": [{"hooks": [{"type": "command", "command": "x"}]}]},
            "statusLine": {"type": "command", "command": "/x/mine.sh"},
            "env": {"MY_OWN": "keep-me"},
            "permissions": {"allow": ["Bash(ls:*)"]}
        }))
        .unwrap();
        let relay = profile(json!({
            "base_url": "https://api.example.com",
            "model": "claude-opus-5"
        }));
        let out = connect(Some(&existing), Some(&relay), &no_secrets).unwrap();
        let parsed: Value = serde_json::from_slice(&out).unwrap();

        assert!(parsed["hooks"]["SessionStart"].is_array());
        assert_eq!(parsed["env"]["MY_OWN"], "keep-me");
        assert_eq!(parsed["permissions"]["allow"][0], "Bash(ls:*)");
        assert_eq!(
            parsed["env"]["ANTHROPIC_BASE_URL"],
            "https://api.example.com"
        );
        assert_eq!(parsed["env"]["ANTHROPIC_MODEL"], "claude-opus-5");
        // 没选内置状态栏时，用户自己那份原样留着
        assert_eq!(parsed["statusLine"]["command"], "/x/mine.sh");
    }

    #[test]
    fn claude_运行参数写入明确路径_清空时只撤掉这些路径() {
        let existing = serde_json::to_vec(&json!({
            "env": {"MY_OWN": "keep"},
            "attribution": {"other": "keep"}
        }))
        .unwrap();
        let out = runtime(
            Some(&existing),
            json!({
                "claude_tool_search": true,
                "claude_agent_teams": true,
                "claude_effort_level": "max",
                "claude_disable_artifact": true,
                "claude_hide_attribution": true
            }),
        );
        let parsed: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(parsed["env"]["ENABLE_TOOL_SEARCH"], "true");
        assert_eq!(parsed["env"]["CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS"], "1");
        assert_eq!(parsed["env"]["CLAUDE_CODE_EFFORT_LEVEL"], "max");
        assert_eq!(parsed["env"]["CLAUDE_CODE_DISABLE_ARTIFACT"], "1");
        assert_eq!(parsed["attribution"]["commit"], "");
        assert_eq!(parsed["attribution"]["pr"], "");

        let cleared = runtime(
            Some(&out),
            json!({
                "claude_tool_search": false,
                "claude_agent_teams": null,
                "claude_effort_level": "",
                "claude_disable_artifact": false,
                "claude_hide_attribution": false
            }),
        );
        let parsed: Value = serde_json::from_slice(&cleared).unwrap();
        assert_eq!(parsed["env"]["MY_OWN"], "keep");
        assert_eq!(parsed["attribution"]["other"], "keep");
        assert!(parsed["env"].get("ENABLE_TOOL_SEARCH").is_none());
        assert!(parsed["attribution"].get("commit").is_none());
    }

    #[test]
    fn 两种认证方式写不同的环境变量() {
        let resolve = |name: &str| (name == "TOK").then(|| "secret".to_string());
        let bearer = profile(json!({"token_var": "TOK"}));
        let parsed: Value =
            serde_json::from_slice(&connect(None, Some(&bearer), &resolve).unwrap()).unwrap();
        assert_eq!(parsed["env"]["ANTHROPIC_AUTH_TOKEN"], "secret");
        assert!(parsed["env"].get("ANTHROPIC_API_KEY").is_none());

        let api_key = profile(json!({"token_var": "TOK", "auth": "api_key"}));
        let parsed: Value =
            serde_json::from_slice(&connect(None, Some(&api_key), &resolve).unwrap()).unwrap();
        assert_eq!(parsed["env"]["ANTHROPIC_API_KEY"], "secret");
        assert!(parsed["env"].get("ANTHROPIC_AUTH_TOKEN").is_none());
    }

    #[test]
    fn 切换接入配置时上一套的变量必须消失() {
        let settings = parse(&json!({"settings": {
            "active": "relay",
            "profiles": {
                "official": {},
                "relay": {
                    "base_url": "https://relay.example.com",
                    "model": "relay-model",
                    "small_fast_model": "relay-small",
                    "model_aliases": {"opus": "relay-opus"},
                    "timeout_ms": 600000
                }
            }
        }}))
        .unwrap();
        let first = connect(None, settings.profiles.get("relay"), &no_secrets).unwrap();
        let parsed: Value = serde_json::from_slice(&first).unwrap();
        assert_eq!(
            parsed["env"]["ANTHROPIC_BASE_URL"],
            "https://relay.example.com"
        );
        assert_eq!(parsed["env"]["ANTHROPIC_SMALL_FAST_MODEL"], "relay-small");
        assert_eq!(parsed["env"]["ANTHROPIC_DEFAULT_OPUS_MODEL"], "relay-opus");
        assert_eq!(parsed["env"]["API_TIMEOUT_MS"], "600000");

        // 切到什么都没配的 official：上一套留下的变量一个都不许剩
        let second = connect(Some(&first), settings.profiles.get("official"), &no_secrets).unwrap();
        let parsed: Value = serde_json::from_slice(&second).unwrap();
        assert!(parsed.get("env").is_none(), "官方配置不需要任何环境变量");
    }

    #[test]
    fn 清空普通项只删自己那条_自动化模式写明安全值_且不碰同级() {
        let existing = serde_json::to_vec(&json!({
            "env": {"ANTHROPIC_BASE_URL": "https://old.example.com", "MY_OWN": "keep-me"},
            "permissions": {"defaultMode": "bypassPermissions", "allow": ["Bash(ls:*)"]}
        }))
        .unwrap();
        let out = connect(Some(&existing), None, &no_secrets).unwrap();
        let out = runtime(Some(&out), json!({"automation_mode": "manual"}));
        let parsed: Value = serde_json::from_slice(&out).unwrap();

        assert!(parsed["env"].get("ANTHROPIC_BASE_URL").is_none());
        assert_eq!(parsed["env"]["MY_OWN"], "keep-me", "同级的别人的变量要留着");
        assert_eq!(parsed["permissions"]["defaultMode"], "manual");
        assert_eq!(
            parsed["permissions"]["allow"][0], "Bash(ls:*)",
            "同级的 allow 要留着"
        );

        // 只剩我们那一条时整个父对象收走
        let only_ours =
            serde_json::to_vec(&json!({"env": {"ANTHROPIC_BASE_URL": "https://x"}})).unwrap();
        let out = connect(Some(&only_ours), None, &no_secrets).unwrap();
        let parsed: Value = serde_json::from_slice(&out).unwrap();
        assert!(parsed.get("env").is_none());
    }

    #[test]
    fn 令牌取不到值时拒绝写入_而不是写个空的上去() {
        let missing = profile(json!({"token_var": "MISSING_TOKEN"}));
        let err = connect(None, Some(&missing), &no_secrets).unwrap_err();
        assert!(err.to_string().contains("MISSING_TOKEN"));
    }

    #[test]
    fn 选了内置状态栏就指向本工具部署的那份_不选则不碰() {
        let out = runtime(None, json!({"statusline": "builtin"}));
        let parsed: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(parsed["statusLine"]["type"], "command");
        // command 在 shell 里执行：始终按单引号引用，路径含空格也不会被拆开
        assert_eq!(parsed["statusLine"]["command"], "'/data/statusline.sh'");

        // 从内置切回「不碰」：把我们指过去的那条撤掉，用户之后自己配什么是他的事
        let back = runtime(Some(&out), json!({"statusline": ""}));
        let parsed: Value = serde_json::from_slice(&back).unwrap();
        assert!(parsed.get("statusLine").is_none());

        // 旧版写的裸路径同样是我们的：「不碰」时一并撤掉
        let legacy = serde_json::to_vec(&json!({
            "statusLine": {"type": "command", "command": "/data/statusline.sh"}
        }))
        .unwrap();
        let back = runtime(Some(&legacy), json!({"statusline": null}));
        let parsed: Value = serde_json::from_slice(&back).unwrap();
        assert!(parsed.get("statusLine").is_none());
    }

    #[test]
    fn 状态栏命令按_posix_单引号引用_空格与单引号都不拆() {
        assert_eq!(shell_single_quote("/a b/c.sh"), "'/a b/c.sh'");
        assert_eq!(shell_single_quote("/it's/c.sh"), "'/it'\\''s/c.sh'");
        let path = std::path::Path::new("/Library/Application Support/it's/statusline.sh");
        let command = statusline_command(path);
        assert_eq!(
            command,
            "'/Library/Application Support/it'\\''s/statusline.sh'"
        );
        assert_eq!(
            builtin_statusline_form(&command, path),
            Some(StatuslineForm::Quoted)
        );
        assert_eq!(
            builtin_statusline_form(&path.display().to_string(), path),
            Some(StatuslineForm::Legacy)
        );
        assert_eq!(builtin_statusline_form("/x/other.sh", path), None);
        // 真让 shell 解一遍：得回原路径且只有一个词
        let echoed = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(format!("printf '%s\\n' {command}"))
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8(echoed.stdout).unwrap(),
            format!("{}\n", path.display())
        );
    }

    #[cfg(unix)]
    #[test]
    fn 引用形式经_shell_执行部署的脚本_额度落在数据目录下() {
        let root = std::env::temp_dir().join(format!(
            "am statusline it's-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let data_dir = root.join("Application Support");
        let path = data_dir.join(STATUSLINE_FILE);
        deploy_statusline(&path).unwrap();
        let command = statusline_command(&path);

        let mut child = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(&command)
            .env("CLAUDE_CONFIG_DIR", "/tmp/am-config")
            .env("AM_STATUSLINE_NOW", "1000")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        {
            use std::io::Write;
            child
                .stdin
                .take()
                .unwrap()
                .write_all(
                    br#"{"model":{"display_name":"Claude Opus"},"rate_limits":{"five_hour":{"used_percentage":42,"resets_at":4600},"seven_day":{"used_percentage":7,"resets_at":90000}}}"#,
                )
                .unwrap();
        }
        let out = child.wait_with_output().unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(stdout.contains("Opus"), "{stdout}");
        let usage = data_dir.join("usage").join("%2Ftmp%2Fam-config.json");
        let saved = std::fs::read_to_string(&usage).unwrap();
        assert!(saved.contains("\"used_percentage\":42"), "{saved}");
        assert!(saved.contains("\"used_percentage\":7"), "{saved}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn 用户自己的状态栏不归我们管_不清空也不算冲突() {
        let mine = serde_json::to_vec(&json!({
            "statusLine": {"type": "command", "command": "/x/mine.sh"},
            "env": {"ANTHROPIC_BASE_URL": "https://a"}
        }))
        .unwrap();
        // 「不碰」模式下渲染：用户那条原样留着
        let out = runtime(Some(&mine), json!({"statusline": ""}));
        let parsed: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(parsed["statusLine"]["command"], "/x/mine.sh");

        // 他换成别的脚本也不算改了我们的东西
        let switched = serde_json::to_vec(&json!({
            "statusLine": {"type": "command", "command": "/x/another.sh"},
            "env": {"ANTHROPIC_BASE_URL": "https://a"}
        }))
        .unwrap();
        assert_eq!(owned_digest(&mine, &sl()), owned_digest(&switched, &sl()));
    }

    #[test]
    fn 外部修改检测只看自己那几条路径() {
        let ours =
            serde_json::to_vec(&json!({"env": {"ANTHROPIC_BASE_URL": "https://a"}})).unwrap();
        let plus_theirs = serde_json::to_vec(&json!({
            "env": {"ANTHROPIC_BASE_URL": "https://a", "MY_OWN": "x"},
            "hooks": {"SessionStart": []}
        }))
        .unwrap();
        assert_eq!(
            owned_digest(&ours, &sl()),
            owned_digest(&plus_theirs, &sl()),
            "用户加自己的设置不算外部修改"
        );

        let changed =
            serde_json::to_vec(&json!({"env": {"ANTHROPIC_BASE_URL": "https://b"}})).unwrap();
        assert_ne!(owned_digest(&ours, &sl()), owned_digest(&changed, &sl()));

        // 状态栏归我们管，被外部改过要认出来
        let sl_ours =
            serde_json::to_vec(&json!({"statusLine": {"command": "/data/statusline.sh"}})).unwrap();
        let sl_changed =
            serde_json::to_vec(&json!({"statusLine": {"command": "/x/other.sh"}})).unwrap();
        assert_ne!(
            owned_digest(&sl_ours, &sl()),
            owned_digest(&sl_changed, &sl())
        );
    }

    #[test]
    fn codex_运行参数写成顶层_toml_并进入受管摘要() {
        let out = write_codex_runtime(
            "user_instructions = \"keep\"\n",
            &codex_patch(json!({
                "codex_reasoning_effort": "high",
                "codex_context_window": 1000000,
                "codex_auto_compact_token_limit": 900000,
                "codex_web_search": "live"
            })),
        );
        assert!(out.contains("model_reasoning_effort = \"high\""));
        assert!(out.contains("model_context_window = 1000000"));
        assert!(out.contains("model_auto_compact_token_limit = 900000"));
        assert!(out.contains("web_search = \"live\""));
        assert!(out.contains("user_instructions = \"keep\""));
        let slice = codex_owned_slice(&out, &[]);
        assert_eq!(slice["model_context_window"], "1000000");

        let cleared = write_codex_runtime(
            &out,
            &codex_patch(json!({
                "codex_reasoning_effort": null,
                "codex_context_window": null,
                "codex_auto_compact_token_limit": "",
                "codex_web_search": null
            })),
        );
        assert!(!cleared.contains("model_reasoning_effort"));
        assert!(!cleared.contains("model_context_window"));
        assert!(cleared.contains("user_instructions = \"keep\""));
    }

    #[test]
    fn 当前配置只读脱敏且不改原文件() {
        let root = std::env::temp_dir().join(format!(
            "agent-manager-current-config-{}-{:x}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let claude = root.join("claude");
        let codex = root.join("codex");
        std::fs::create_dir_all(&claude).unwrap();
        std::fs::create_dir_all(&codex).unwrap();
        let statusline = root.join("statusline.sh");
        let claude_bytes = serde_json::to_vec(&json!({
            "env": {
                "ANTHROPIC_BASE_URL": "https://relay.example.com",
                "ANTHROPIC_AUTH_TOKEN": "never-expose",
                "ANTHROPIC_MODEL": "claude-model",
                "ANTHROPIC_SMALL_FAST_MODEL": "claude-small",
                "ANTHROPIC_DEFAULT_OPUS_MODEL": "claude-opus",
                "API_TIMEOUT_MS": "600000",
                "CLAUDE_CODE_EFFORT_LEVEL": "high"
            },
            "statusLine": {"command": statusline_command(&statusline)},
            "attribution": {"commit": "", "pr": ""}
        }))
        .unwrap();
        std::fs::write(claude.join("settings.json"), &claude_bytes).unwrap();
        let codex_bytes = br#"model_provider = "relay"
model = "gpt-test"
model_reasoning_effort = "high"
[model_providers.relay]
base_url = "https://codex.example.com/v1"
env_key = "CODEX_RELAY_KEY"
stream_idle_timeout_ms = 450000
"#;
        std::fs::write(codex.join(CODEX_CONFIG), codex_bytes).unwrap();

        let (claude_status, _) = observe(
            &claude.join("settings.json"),
            Kind::Claude,
            Some(&statusline),
        );
        assert_eq!(claude_status["state"], "ready");
        assert_eq!(
            claude_status["target"],
            claude.join("settings.json").display().to_string()
        );
        assert_eq!(
            claude_status["config"]["base_url"],
            "https://relay.example.com"
        );
        assert_eq!(claude_status["config"]["model"], "claude-model");
        assert_eq!(claude_status["config"]["small_fast_model"], "claude-small");
        assert_eq!(
            claude_status["config"]["model_aliases"]["opus"],
            "claude-opus"
        );
        assert_eq!(claude_status["config"]["timeout_ms"], "600000");
        assert_eq!(claude_status["config"]["credential"]["configured"], true);
        assert_eq!(claude_status["config"]["runtime"]["statusline"], "builtin");
        assert_eq!(
            claude_status["config"]["runtime"]["claude_hide_attribution"],
            true
        );
        assert!(!claude_status.to_string().contains("never-expose"));
        assert_eq!(
            std::fs::read(claude.join("settings.json")).unwrap(),
            claude_bytes
        );

        let (codex_status, _) = observe(&codex.join(CODEX_CONFIG), Kind::Codex, None);
        assert_eq!(codex_status["state"], "ready");
        assert_eq!(
            codex_status["target"],
            codex.join(CODEX_CONFIG).display().to_string()
        );
        assert_eq!(codex_status["config"]["provider"], "relay");
        assert_eq!(
            codex_status["config"]["credential"]["variable"],
            "CODEX_RELAY_KEY"
        );
        assert_eq!(codex_status["config"]["timeout_ms"], 450000);
        assert_eq!(
            std::fs::read(codex.join(CODEX_CONFIG)).unwrap(),
            codex_bytes
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn 当前配置区分本工具状态栏与用户状态栏_署名开关必须两项都为空() {
        let statusline = std::path::Path::new("/tmp/agent-manager/statusline.sh");
        let custom = claude_observed_config(
            br#"{"statusLine":{"command":"/tmp/user/statusline.sh"},"attribution":{"commit":"","pr":"keep"}}"#,
            Some(statusline),
        )
        .unwrap();
        assert_eq!(custom["runtime"]["statusline"], "custom");
        assert_eq!(custom["runtime"]["claude_hide_attribution"], false);

        let builtin = claude_observed_config(
            br#"{"statusLine":{"command":"'/tmp/agent-manager/statusline.sh'"},"attribution":{"commit":"","pr":""}}"#,
            Some(statusline),
        )
        .unwrap();
        assert_eq!(builtin["runtime"]["statusline"], "builtin");
        assert_eq!(builtin["runtime"]["claude_hide_attribution"], true);

        // 旧版写的裸路径：认得是我们的，但单独报出来，界面据此提示重新应用
        let legacy = claude_observed_config(
            br#"{"statusLine":{"command":"/tmp/agent-manager/statusline.sh"}}"#,
            Some(statusline),
        )
        .unwrap();
        assert_eq!(legacy["runtime"]["statusline"], "builtin-legacy");
    }

    #[cfg(unix)]
    #[test]
    fn 当前配置跟随符号链接读取_断链与目录不读_缺失读取零写() {
        use std::os::unix::fs::symlink;
        let root = std::env::temp_dir().join(format!(
            "agent-manager-current-config-safe-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let missing = root.join("missing");
        assert_eq!(
            observe(&missing.join("settings.json"), Kind::Claude, None).0["state"],
            "missing"
        );
        assert!(!missing.exists());

        let target = root.join("target.json");
        let config_dir = root.join("claude");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::write(&target, b"{}").unwrap();
        symlink(&target, config_dir.join("settings.json")).unwrap();
        // dotfiles 仓库常把配置软链过来：按最终目标读
        assert_eq!(
            observe(&config_dir.join("settings.json"), Kind::Claude, None).0["state"],
            "ready"
        );
        // 断链与指向目录的链接不读正文
        symlink(root.join("gone.json"), config_dir.join("dangling.json")).unwrap();
        assert_eq!(
            observe(&config_dir.join("dangling.json"), Kind::Claude, None).0["state"],
            "unsafe"
        );
        symlink(&root, config_dir.join("dir.json")).unwrap();
        assert_eq!(
            observe(&config_dir.join("dir.json"), Kind::Claude, None).0["state"],
            "unsafe"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn 内置状态栏脚本自身可用() {
        // 随脚本一起编译进来，跑一遍语法检查，避免改坏了到运行时才发现
        assert!(STATUSLINE_BODY.starts_with("#!/bin/bash"));
        // 落盘是它存在的理由：落在脚本所在的数据目录下，不往助手配置目录放文件
        assert!(
            STATUSLINE_BODY.contains("am_usage=${am_self%/*}/usage"),
            "落盘是它存在的理由"
        );
        assert!(!STATUSLINE_BODY.contains("agent-manager-usage.json"));
        assert!(
            STATUSLINE_BODY.contains("CLAUDE_CONFIG_DIR"),
            "实例要各存各的"
        );
    }

    #[test]
    fn 运行参数补丁只认该形态的字段与词汇表() {
        // 拼错或跨形态的字段不能静默不生效
        assert!(parse_runtime_patch(Kind::Claude, &json!({"codex_web_search": "live"})).is_err());
        assert!(parse_runtime_patch(Kind::Codex, &json!({"statusline": "builtin"})).is_err());
        assert!(parse_runtime_patch(Kind::Claude, &json!({"automation_mode": null})).is_err());
        assert!(
            parse_runtime_patch(Kind::Claude, &json!({"claude_effort_level": "ultra"})).is_err()
        );
        assert!(parse_runtime_patch(Kind::Claude, &json!({"claude_tool_search": "yes"})).is_err());
        assert!(parse_runtime_patch(Kind::Codex, &json!({"codex_context_window": 0})).is_err());
        assert!(parse_runtime_patch(Kind::Claude, &json!([])).is_err());

        let patch = parse_runtime_patch(
            Kind::Claude,
            &json!({"automation_mode": "full", "statusline": "builtin", "claude_effort_level": ""}),
        )
        .unwrap();
        assert!(patch.sets_full());
        assert!(patch.sets_builtin_statusline());
        assert_eq!(
            patch.keys(),
            vec!["automation_mode", "claude_effort_level", "statusline"]
        );
        assert_eq!(
            codex_patch(json!({"codex_context_window": ""})).0["codex_context_window"],
            RuntimeValue::Number(None)
        );
    }

    #[test]
    fn 补丁外的运行参数原样保留_包括本工具不提供的取值() {
        let existing = serde_json::to_vec(&json!({
            "permissions": {"defaultMode": "acceptEdits", "allow": ["Bash(ls:*)"]},
            "env": {"CLAUDE_CODE_EFFORT_LEVEL": "low"}
        }))
        .unwrap();
        let out = runtime(Some(&existing), json!({"claude_effort_level": "high"}));
        let parsed: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(parsed["permissions"]["defaultMode"], "acceptEdits");
        assert_eq!(parsed["env"]["CLAUDE_CODE_EFFORT_LEVEL"], "high");

        let codex = "approval_policy = \"never\"\nmodel = \"gpt-5\"\n";
        let out = write_codex_runtime(codex, &codex_patch(json!({"codex_web_search": "live"})));
        assert!(out.contains("approval_policy = \"never\""), "{out}");
        assert!(out.contains("model = \"gpt-5\""), "{out}");
        assert!(out.contains("web_search = \"live\""), "{out}");
    }

    #[test]
    fn 实际自动化档位不属于预设时如实报_custom_并给出原值() {
        let claude =
            claude_observed_config(br#"{"permissions":{"defaultMode":"acceptEdits"}}"#, None)
                .unwrap();
        assert_eq!(claude["runtime"]["automation_mode"], "custom");
        assert_eq!(
            claude["runtime"]["automation_raw"]["permissions.defaultMode"],
            "acceptEdits"
        );
        let unset = claude_observed_config(br#"{}"#, None).unwrap();
        assert_eq!(unset["runtime"]["automation_mode"], "manual");
        assert!(unset["runtime"]["automation_raw"].is_null());

        let codex = codex_observed_config(b"approval_policy = \"never\"\n").unwrap();
        assert_eq!(codex["runtime"]["automation_mode"], "custom");
        assert_eq!(
            codex["runtime"]["automation_raw"]["approval_policy"],
            "never"
        );
        let preset = codex_observed_config(
            b"approval_policy = \"on-request\"\nsandbox_mode = \"workspace-write\"\napprovals_reviewer = \"auto_review\"\n",
        )
        .unwrap();
        assert_eq!(preset["runtime"]["automation_mode"], "auto");
    }

    fn claude_live(value: Value) -> Live {
        Live::read(Kind::Claude, &serde_json::to_vec(&value).unwrap()).unwrap()
    }

    #[test]
    fn 按内容识别当前接入_active_只在内容相同的候选间裁决() {
        let settings = |active: Option<&str>| {
            let mut raw = json!({"settings": {"profiles": {
                "official": {"label": "官方"},
                "relay": {"base_url": "https://relay.example.com", "model": "m1"}
            }}});
            if let Some(active) = active {
                raw["settings"]["active"] = json!(active);
            }
            parse(&raw).unwrap()
        };
        // 空字符串与没写等价
        let empty = claude_live(
            json!({"permissions": {"defaultMode": "auto"}, "env": {"ANTHROPIC_MODEL": ""}}),
        );
        let none = BTreeSet::new();
        // 没有接入键：切回默认后是默认账号；上次写的是 official 时就是 official
        assert_eq!(
            identify(&empty, &settings(None), &none).current,
            Current::Default
        );
        assert_eq!(
            identify(&empty, &settings(Some("official")), &none).current,
            Current::Profile("official".into())
        );
        let relay = claude_live(
            json!({"env": {"ANTHROPIC_BASE_URL": "https://relay.example.com", "ANTHROPIC_MODEL": "m1"}}),
        );
        // 库里 active 指向别处也不影响：内容说了算
        assert_eq!(
            identify(&relay, &settings(Some("official")), &none).current,
            Current::Profile("relay".into())
        );
        // 外部只改了模型：未保存，但与上次写入的 relay 身份一致
        let model_changed = claude_live(
            json!({"env": {"ANTHROPIC_BASE_URL": "https://relay.example.com", "ANTHROPIC_MODEL": "m2"}}),
        );
        let identified = identify(&model_changed, &settings(Some("relay")), &none);
        assert_eq!(identified.current, Current::Unsaved);
        assert_eq!(identified.drifted_from.as_deref(), Some("relay"));
        // 地址也换了：身份不同，切换时新建导入项
        let other =
            claude_live(json!({"env": {"ANTHROPIC_BASE_URL": "https://other.example.com"}}));
        let identified = identify(&other, &settings(Some("relay")), &none);
        assert_eq!(identified.current, Current::Unsaved);
        assert_eq!(identified.drifted_from, None);
    }

    fn codex_live(text: &str) -> CodexLive {
        match Live::read(Kind::Codex, text.as_bytes()).unwrap() {
            Live::Codex(live) => live,
            Live::Claude(_) => unreachable!(),
        }
    }

    #[test]
    fn 引用型接入的检查端点取自被引用的表_内置用默认值_表不在就明确报错() {
        let root = std::env::temp_dir().join(format!(
            "am-referenced-endpoint-{}-{:x}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let ctx = Context {
            data_dir: root.join("data"),
            home_dir: root.join("home"),
        };
        std::fs::create_dir_all(ctx.home_dir.join(".codex")).unwrap();
        std::fs::write(
            ctx.home_dir.join(".codex/config.toml"),
            "[model_providers.kimi]\nbase_url = \"http://10.0.0.2:8080/v1\"\nenv_key = \"KIMI_KEY\"\n\n[model_providers.bare]\nname = \"no url\"\n",
        )
        .unwrap();

        assert_eq!(
            codex_referenced_endpoint(&ctx, None, "kimi").unwrap(),
            (
                "http://10.0.0.2:8080/v1".to_string(),
                Some("KIMI_KEY".to_string())
            )
        );
        assert_eq!(
            codex_referenced_endpoint(&ctx, None, "ollama").unwrap(),
            ("http://localhost:11434/v1".to_string(), None)
        );
        assert_eq!(
            codex_referenced_endpoint(&ctx, None, "openai")
                .unwrap()
                .1
                .as_deref(),
            Some("OPENAI_API_KEY")
        );
        let missing = codex_referenced_endpoint(&ctx, None, "gone").unwrap_err();
        assert!(
            missing.to_string().contains("已不在 config.toml 中"),
            "{missing}"
        );
        let bare = codex_referenced_endpoint(&ctx, None, "bare").unwrap_err();
        assert!(bare.to_string().contains("没有 base_url"), "{bare}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn codex_引用型接入只看_model_provider_与模型_切换只写顶层键() {
        let user_table = "model_provider = \"kimi\"\nmodel = \"k2\"\n\n[model_providers.kimi]\nbase_url = \"http://10.0.0.2:8080/v1\"\nhttp_headers = { X = \"1\" }\n";
        let live = codex_live(user_table);
        let reference = profile(json!({"provider_ref": "kimi", "model": "k2"}));
        assert!(codex_matches("imported", &reference, &live));
        assert!(!codex_matches(
            "imported",
            &profile(json!({"provider_ref": "kimi", "model": "k3"})),
            &live
        ));

        // 切到引用型接入：只写 model_provider / model，那张表原样保留、也不记为本工具所有
        let (out, written) = write_codex_connection(
            "model = \"gpt-5\"\n\n[model_providers.kimi]\nbase_url = \"http://10.0.0.2:8080/v1\"\n",
            Some(("imported", &reference)),
            &[],
        )
        .unwrap();
        assert!(written.is_empty());
        assert!(out.contains("model_provider = \"kimi\""), "{out}");
        assert!(out.contains("model = \"k2\""), "{out}");
        assert!(out.contains("[model_providers.kimi]\nbase_url = \"http://10.0.0.2:8080/v1\"\n"));

        // 引用的表不在了、又不是内置的：拒绝；内置 ollama 不需要表
        let err = write_codex_connection("", Some(("imported", &reference)), &[]).unwrap_err();
        assert!(err.to_string().contains("已不在 config.toml 中"), "{err}");
        let ollama = profile(json!({"provider_ref": "ollama"}));
        let (out, _) = write_codex_connection("", Some(("local", &ollama)), &[]).unwrap();
        assert!(out.contains("model_provider = \"ollama\""), "{out}");
    }
}
