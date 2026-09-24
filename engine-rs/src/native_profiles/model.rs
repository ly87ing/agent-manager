//! profiles 域模型：助手实例（profile）的定义解析、安全校验与目标内容渲染。
//!
//! 一个 profile 就是「同一个助手的另一套独立配置」：自带配置目录，启动时用
//! `CLAUDE_CONFIG_DIR` / `CODEX_HOME` 指过去，因此多个实例可以同时运行互不干扰，
//! 全局 `~/.claude` 与 `~/.codex` 一个字节都不动。
//!
//! 定义存在 `agents.definitions.<id>.profile`：profile 同时是一个 agent 定义，
//! 技能与 MCP 的逐助手分配因此自动适用于它，不需要另一套分配模型。
//!
//! 安全边界（全部 fail-closed，错误不回显取值正文）：
//! - 配置目录必须在 HOME 下，不得是主实例目录本身，不得落在本工具数据目录内，不得是符号链接；
//! - 鉴权类环境变量只接受 `${VAR}` 引用，库里零明文（与 MCP 同级语义）；
//! - 接口地址只接受 https 或本机回环 http。

use anyhow::{bail, Result};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

/// Codex 实例的接入方式 id：实例 `config.toml` 里的 `model_provider` 与
/// `[model_providers.agent-manager]` 用它。
pub(crate) const CODEX_PROVIDER_ID: &str = "agent-manager";
const FILE_CODEX_CONFIG: &str = "config.toml";

/// 实例配置目录里由本工具维护的文件。
pub(crate) const FILE_CLAUDE_SETTINGS: &str = "settings.json";

/// 实例目录下的 hooks：以实际文件为准，归 hooks 域作为 hooks 目标直接编辑。
/// profiles 域不渲染 hooks.json，也不碰 settings.json 的 `hooks` 键。
pub(crate) const FILE_CODEX_HOOKS: &str = "hooks.json";

/// 实例目录下的规则文件：以实际文件为准，归 rules 域作为规则目标直接编辑，
/// profiles 域不渲染、不写、不检测它。
pub(crate) const FILE_CLAUDE_RULES: &str = "CLAUDE.md";
pub(crate) const FILE_CODEX_RULES: &str = "AGENTS.md";

/// 一个待写入的受管文件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RenderedTarget {
    /// 配置目录内的相对文件名。
    pub file: String,
    pub content: Vec<u8>,
}

/// profile 的基础助手类型：决定配置目录变量与目标文件格式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProfileKind {
    /// `CLAUDE_CONFIG_DIR` + `<dir>/settings.json`
    Claude,
    /// `CODEX_HOME` + `<dir>/config.toml`（只改 model、model_provider 与本工具那张接入表）
    Codex,
}

impl ProfileKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            ProfileKind::Claude => "claude",
            ProfileKind::Codex => "codex",
        }
    }

    /// 启动时用来指向独立配置目录的环境变量名。
    pub(crate) fn config_env(self) -> &'static str {
        match self {
            ProfileKind::Claude => "CLAUDE_CONFIG_DIR",
            ProfileKind::Codex => "CODEX_HOME",
        }
    }

    /// 本工具在配置目录里写的目标文件（相对配置目录）。
    ///
    /// Codex 与 MCP 域共用 `config.toml`，两边都按键原位修改：MCP 只动 `mcp_servers`，本域
    /// 只动 `model`、`model_provider` 与 `[model_providers.agent-manager]`。和 CC Switch 一样
    /// 写助手的标准配置文件，启动时不再 `-p` 叠加单独的文件。
    pub(crate) fn target_file(self) -> &'static str {
        match self {
            ProfileKind::Claude => FILE_CLAUDE_SETTINGS,
            ProfileKind::Codex => FILE_CODEX_CONFIG,
        }
    }

    /// 主实例的默认配置目录（HOME 相对），profile 不得指向它。
    fn primary_dir(self) -> &'static str {
        match self {
            ProfileKind::Claude => ".claude",
            ProfileKind::Codex => ".codex",
        }
    }

    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "claude" | "claude-code" => Some(ProfileKind::Claude),
            "codex" => Some(ProfileKind::Codex),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ProfileDef {
    pub id: String,
    pub display_name: String,
    pub base_agent: String,
    pub kind: ProfileKind,
    /// 登记形式（`~/...`），展示与持久化都用它。
    pub config_dir_raw: String,
    /// Claude：写进 settings.json 的 env 映射（值可能是 `${VAR}` 引用）。
    pub env: Map<String, Value>,
    /// Codex：接口地址与凭据所在的变量名（库里只存变量名；写入实例 `config.toml` 时解析成
    /// 取值，写进 `experimental_bearer_token`）。
    pub base_url: Option<String>,
    pub env_key: Option<String>,
    pub wire_api: Option<String>,
    pub model: Option<String>,
    pub enabled: bool,
}

/// profile id 规则：与 alias 名一致（它同时就是启动命令名）。
pub(crate) fn valid_profile_id(id: &str) -> bool {
    let mut chars = id.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_lowercase() || first.is_ascii_digit()) {
        return false;
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn is_placeholder(value: &str) -> bool {
    let inner = match value.strip_prefix("${").and_then(|v| v.strip_suffix('}')) {
        Some(inner) => inner,
        None => return false,
    };
    let mut chars = inner.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// 变量名看起来是否承载凭据：这类值一律只接受 `${VAR}` 引用。
fn looks_like_credential(name: &str) -> bool {
    crate::native::credential_like_name(name)
}

fn valid_env_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// 接口地址：https 或 http（与接入配置同一口径：实际配置里已有的局域网 http 中转不因
/// 本工具的校验而存不下；界面对非 https 提示令牌将明文传输）。错误不回显地址。
fn validate_base_url(url: &str) -> Result<()> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"));
    match rest {
        Some(host) if !host.is_empty() && !host.starts_with('/') => Ok(()),
        _ => bail!("接口地址只允许 https:// 或 http:// 开头的完整地址"),
    }
}

/// 展开 `~/` 前缀；其余原样。
pub(crate) fn expand_home(path: &str, home_dir: &Path) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        return home_dir.join(rest);
    }
    if path == "~" {
        return home_dir.to_path_buf();
    }
    PathBuf::from(path)
}

/// 配置目录安全校验：必须在 HOME 下、不是主实例目录、不在数据目录内、不是符号链接。
pub(crate) fn validate_config_dir(
    def: &ProfileDef,
    home_dir: &Path,
    data_dir: &Path,
) -> Result<PathBuf> {
    let raw = def.config_dir_raw.trim();
    if raw.is_empty() {
        bail!("[{}] 配置目录不能为空", def.id);
    }
    if raw.contains('\0') || raw.contains('\n') {
        bail!("[{}] 配置目录含非法字符", def.id);
    }
    let path = expand_home(raw, home_dir);
    if !path.is_absolute() {
        bail!("[{}] 配置目录必须是绝对路径或以 ~/ 开头", def.id);
    }
    if path.components().any(|c| c.as_os_str() == "..") {
        bail!("[{}] 配置目录不允许包含 ..", def.id);
    }
    if !path.starts_with(home_dir) {
        bail!("[{}] 配置目录必须位于用户主目录下", def.id);
    }
    if path == home_dir {
        bail!("[{}] 配置目录不能是主目录本身", def.id);
    }
    if path == home_dir.join(def.kind.primary_dir()) {
        bail!(
            "[{}] 配置目录不能是主实例目录本身（那会改到你正在用的那套配置）",
            def.id
        );
    }
    if path.starts_with(data_dir) {
        bail!("[{}] 配置目录不能放在本工具的数据目录里", def.id);
    }
    if std::fs::symlink_metadata(&path)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
    {
        bail!("[{}] 配置目录不能是符号链接", def.id);
    }
    if path.exists() && !path.is_dir() {
        bail!("[{}] 配置目录已被一个同名文件占用", def.id);
    }
    Ok(path)
}

/// 解析单个 profile 定义；结构非法即 fail-closed，消息不含取值正文。
pub(crate) fn parse_definition(id: &str, raw: &Value, enabled: bool) -> Result<ProfileDef> {
    if !valid_profile_id(id) {
        bail!("profile 名只能用小写字母、数字与连字符，且以字母或数字开头");
    }
    let obj = raw
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("[{}] 定义不是对象", id))?;
    let profile = obj
        .get("profile")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow::anyhow!("[{}] 缺少 profile 段", id))?;

    let base_agent = profile
        .get("base_agent")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("[{}] profile 缺少 base_agent", id))?
        .to_string();
    let kind_raw = profile
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or(base_agent.as_str());
    let kind = ProfileKind::parse(kind_raw).ok_or_else(|| {
        anyhow::anyhow!("[{}] 暂不支持该助手的实例（只支持 Claude 与 Codex）", id)
    })?;
    let config_dir_raw = profile
        .get("config_dir")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("[{}] profile 缺少 config_dir", id))?
        .to_string();

    let mut env = Map::new();
    if let Some(raw_env) = profile.get("env") {
        let map = raw_env
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("[{}] profile.env 必须是对象", id))?;
        for (name, value) in map {
            if !valid_env_name(name) {
                bail!("[{}] 环境变量名不合法（只允许字母、数字与下划线）", id);
            }
            let text = value
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("[{}] 环境变量值必须是字符串", id))?;
            if looks_like_credential(name) && !is_placeholder(text) {
                bail!(
                    "[{}] 变量 {} 承载凭据，只接受 ${{变量名}} 引用，不接受直接写值",
                    id,
                    name
                );
            }
            if name.ends_with("BASE_URL") && !is_placeholder(text) {
                validate_base_url(text)?;
            }
            env.insert(name.clone(), Value::String(text.to_string()));
        }
    }

    if kind == ProfileKind::Claude {
        check_claude_credentials(id, &env)?;
    }

    let base_url = profile
        .get("base_url")
        .and_then(Value::as_str)
        .map(str::to_string);
    if let Some(url) = &base_url {
        validate_base_url(url)?;
    }
    let env_key = profile
        .get("env_key")
        .and_then(Value::as_str)
        .map(str::to_string);
    if let Some(key) = &env_key {
        if !valid_env_name(key) {
            bail!("[{}] env_key 不是合法的环境变量名", id);
        }
        if base_url.is_none() {
            bail!("[{}] Codex 实例的凭据要和接口地址一起设置", id);
        }
    }
    let wire_api = profile
        .get("wire_api")
        .and_then(Value::as_str)
        .map(str::to_string);
    if let Some(api) = &wire_api {
        if api != CODEX_WIRE_API {
            bail!(
                "[{}] wire_api 只支持 responses：Codex 已移除 chat（Chat Completions）接入方式，写进配置会让 Codex 读不了整份配置",
                id
            );
        }
    }
    let model = profile
        .get("model")
        .and_then(Value::as_str)
        .map(str::to_string);

    Ok(ProfileDef {
        id: id.to_string(),
        display_name: obj
            .get("display_name")
            .and_then(Value::as_str)
            .unwrap_or(id)
            .to_string(),
        base_agent,
        kind,
        config_dir_raw,
        env,
        base_url,
        env_key,
        wire_api,
        model,
        enabled,
    })
}

/// Claude 第三方凭据变量（官方网关文档给出的两种写法）与官方账号的长期令牌变量。
const CLAUDE_THIRD_PARTY_CREDENTIALS: [&str; 2] = ["ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_API_KEY"];
const CLAUDE_OFFICIAL_TOKEN: &str = "CLAUDE_CODE_OAUTH_TOKEN";
const CLAUDE_BASE_URL: &str = "ANTHROPIC_BASE_URL";
/// 云厂商开关：官方认证优先级里排在所有凭据之前，任一为真就改走对应云厂商
/// （认证文档与环境变量参考列出的五个）。
pub(crate) const CLAUDE_CLOUD_PROVIDER_SWITCHES: [&str; 5] = [
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_VERTEX",
    "CLAUDE_CODE_USE_FOUNDRY",
    "CLAUDE_CODE_USE_ANTHROPIC_AWS",
    "CLAUDE_CODE_USE_MANTLE",
];
/// 附加请求头：可以带 `Authorization` 等凭据头，从 shell 继承进来就会连同请求发给实例的接口。
const CLAUDE_CUSTOM_HEADERS: &str = "ANTHROPIC_CUSTOM_HEADERS";

/// 官方账号凭据与第三方凭据不能出现在同一个实例里，第三方实例只能有一种凭据：
/// - 两种第三方凭据同时声明时 Claude 按优先级取其一，结果取决于规则而不是定义，拒绝；
/// - 第三方地址不带凭据时，Claude 会把目录里的官方登录发往该地址，拒绝；
/// - 第三方实例里声明官方长期令牌，就是两种凭据混用，拒绝。
fn check_claude_credentials(id: &str, env: &Map<String, Value>) -> Result<()> {
    let declared: Vec<&str> = CLAUDE_THIRD_PARTY_CREDENTIALS
        .into_iter()
        .filter(|name| env.contains_key(*name))
        .collect();
    if declared.len() > 1 {
        bail!("[{id}] ANTHROPIC_AUTH_TOKEN 与 ANTHROPIC_API_KEY 只能声明一个，请按接口要求的请求头二选一");
    }
    let third_party = !declared.is_empty() || env.contains_key(CLAUDE_BASE_URL);
    if third_party && declared.is_empty() {
        bail!("[{id}] 设置了 ANTHROPIC_BASE_URL 却没有凭据：Claude 会把官方账号登录发往这个地址，请声明 ANTHROPIC_AUTH_TOKEN 或 ANTHROPIC_API_KEY");
    }
    if third_party && env.contains_key(CLAUDE_OFFICIAL_TOKEN) {
        bail!("[{id}] {CLAUDE_OFFICIAL_TOKEN} 是官方账号凭据，不能和第三方凭据放在同一个实例");
    }
    if let Some(name) = CLAUDE_CLOUD_PROVIDER_SWITCHES.into_iter().find(|name| {
        third_party
            && env
                .get(*name)
                .and_then(Value::as_str)
                .is_some_and(|value| !value.is_empty())
    }) {
        bail!("[{id}] {name} 会让 Claude 改走云厂商、不用这个实例的第三方凭据，两者不能放在同一个实例");
    }
    Ok(())
}

/// 实例是否使用第三方凭据或第三方地址：这类实例的配置目录里不允许有官方账号登录。
pub(crate) fn is_third_party(def: &ProfileDef) -> bool {
    match def.kind {
        ProfileKind::Claude => {
            def.env.contains_key(CLAUDE_BASE_URL)
                || CLAUDE_THIRD_PARTY_CREDENTIALS
                    .iter()
                    .any(|name| def.env.contains_key(*name))
        }
        ProfileKind::Codex => def.base_url.is_some() || def.env_key.is_some(),
    }
}

/// Claude 实例的 settings.json 里要置为空串的变量：会压过实例自己凭据或登录的外部设置，
/// 实例没声明的一律取消——云厂商开关、两种第三方凭据、官方长期令牌、接口地址与附加请求头。
///
/// 官方设置参考（`env` 一节）：settings 的 env 覆盖 shell 里的同名变量，置为 `""` 即取消 shell
/// 导出，Claude Code 在选择凭据时把空值当作未设置。第三方实例因此只可能用自己声明的那一个
/// 凭据发往自己的地址；官方账号实例只可能用这个目录里的登录（或它自己声明的长期令牌），shell
/// 里给别的账号或中转导出的变量进不来。真实 claude 2.1.281 实测：不取消时 shell 的
/// `CLAUDE_CODE_USE_BEDROCK=1` 把第三方实例改走 Bedrock、shell 的地址与 key 把官方账号实例
/// 发往中转、shell 的附加请求头随请求发给实例的接口；取消后都不再发生。
fn claude_cancelled_env(def: &ProfileDef) -> Vec<&'static str> {
    if def.kind != ProfileKind::Claude {
        return Vec::new();
    }
    CLAUDE_CLOUD_PROVIDER_SWITCHES
        .into_iter()
        .chain(CLAUDE_THIRD_PARTY_CREDENTIALS)
        .chain([
            CLAUDE_OFFICIAL_TOKEN,
            CLAUDE_BASE_URL,
            CLAUDE_CUSTOM_HEADERS,
        ])
        .filter(|name| !def.env.contains_key(*name))
        .collect()
}

/// 从有效策略里取出全部 profile 定义（带 profile 段的 agent 定义），按 id 排序。
pub(crate) fn load_definitions(policy: &Value) -> Vec<Result<ProfileDef>> {
    let mut out = Vec::new();
    let Some(defs) = policy
        .get("agents")
        .and_then(|v| v.get("definitions"))
        .and_then(Value::as_object)
    else {
        return out;
    };
    for (id, raw) in defs {
        if raw.get("_delete") == Some(&Value::Bool(true)) {
            continue;
        }
        if raw.get("profile").is_none() {
            continue;
        }
        let enabled = policy
            .get("agents")
            .and_then(|v| v.get(id))
            .and_then(|v| v.get("enabled"))
            .and_then(Value::as_bool)
            .unwrap_or(true);
        out.push(parse_definition(id, raw, enabled));
    }
    out
}

/// 实例 settings.json 里与本工具有关的键只有 env / model：`hooks` 归 hooks 域，其余根级
/// 字段是用户自己的（合并写入时原样保留）。env 只写实例定义里声明的那几个变量，
/// 用户自己加的 env 变量原样保留。Codex 实例 config.toml 同理，只动下面这几项。
const CODEX_MODEL_PROVIDER: &str = "model_provider";

fn codex_provider_path() -> String {
    format!("model_providers.{CODEX_PROVIDER_ID}")
}

/// 文件不存在时的写入基准。
pub(crate) const BASE_MISSING: &str = "missing";

/// 本次定义要写的路径：`env.<变量名>` 与（定义了模型时）`model`。
/// 与库里记下的上次写入路径求并，就是这次写入可能改动的全部位置。
pub(crate) fn owned_paths(def: &ProfileDef) -> Vec<String> {
    let mut paths: Vec<String> = match def.kind {
        ProfileKind::Claude => def
            .env
            .keys()
            .map(String::as_str)
            .chain(claude_cancelled_env(def))
            .map(|key| format!("env.{key}"))
            .collect(),
        ProfileKind::Codex if def.base_url.is_some() => {
            vec![CODEX_MODEL_PROVIDER.to_string(), codex_provider_path()]
        }
        ProfileKind::Codex => Vec::new(),
    };
    if def.model.is_some() {
        paths.push("model".to_string());
    }
    paths
}

/// 实例目标文件的写入基准：Claude 取 settings.json 里 env 与 model 两项的摘要（用户改
/// permissions、hooks 等其他键不影响它）；Codex 取 config.toml 里 model、model_provider 与
/// 本工具那张接入表的摘要（MCP 条目等其他内容不影响它）。文件不存在为 `missing`；文件
/// 解析不了时报错（不给基准，也就不能写）。
pub(crate) fn live_base(kind: ProfileKind, bytes: Option<&[u8]>) -> Result<String> {
    let Some(bytes) = bytes else {
        return Ok(BASE_MISSING.to_string());
    };
    match kind {
        ProfileKind::Codex => {
            let doc = parse_codex_config(bytes)?;
            let pick = |value: Option<&toml::Value>| {
                value
                    .and_then(|value| serde_json::to_value(value).ok())
                    .unwrap_or(Value::Null)
            };
            let slice = serde_json::json!({
                "model": pick(doc.get("model")),
                "model_provider": pick(doc.get(CODEX_MODEL_PROVIDER)),
                "provider": pick(doc.get("model_providers").and_then(|p| p.get(CODEX_PROVIDER_ID))),
            });
            Ok(crate::native_mcp::model::sha256_hex(
                &crate::native_mcp::model::canonical_compact(&slice),
            ))
        }
        ProfileKind::Claude => {
            let root = parse_settings(bytes)?;
            let slice = serde_json::json!({
                "env": root.get("env").cloned().unwrap_or(Value::Null),
                "model": root.get("model").cloned().unwrap_or(Value::Null),
            });
            Ok(crate::native_mcp::model::sha256_hex(
                &crate::native_mcp::model::canonical_compact(&slice),
            ))
        }
    }
}

/// 解析实例 settings.json：必须是 JSON 对象，`env` 若存在必须是对象。不当成空文件覆盖。
fn parse_settings(bytes: &[u8]) -> Result<Map<String, Value>> {
    let root = serde_json::from_slice::<Value>(bytes)
        .ok()
        .and_then(|value| value.as_object().cloned())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "实例的 settings.json 不是合法的 JSON 对象，拒绝读写（不会当成空文件覆盖）"
            )
        })?;
    if root.get("env").is_some_and(|env| !env.is_object()) {
        bail!("实例 settings.json 里的 env 不是对象，拒绝读写");
    }
    Ok(root)
}

/// 解析实例 config.toml：必须是合法 TOML。不当成空文件覆盖。
fn parse_codex_config(bytes: &[u8]) -> Result<toml::Table> {
    std::str::from_utf8(bytes)
        .ok()
        .and_then(|text| text.parse::<toml::Table>().ok())
        .ok_or_else(|| {
            anyhow::anyhow!("实例的 config.toml 不是合法的 TOML，拒绝读写（不会当成空文件覆盖）")
        })
}

/// 展示实际取值：凭据类变量或看起来像凭据的取值一律不输出。
fn display_value(name: &str, value: &str) -> Value {
    if looks_like_credential(name) || crate::native::reject_secret_text(value).is_some() {
        Value::String("<hidden>".to_string())
    } else {
        Value::String(value.to_string())
    }
}

/// 逐字段比较实例定义与实际文件：`same` / `differs` / `missing`（实际里没有）/
/// `unresolved`（定义引用的变量取不到值，无法判断）。不输出凭据取值。
pub(crate) fn compare_fields(
    def: &ProfileDef,
    bytes: Option<&[u8]>,
    resolve: &dyn Fn(&str) -> Option<String>,
) -> Result<Vec<Value>> {
    let mut fields = Vec::new();
    match def.kind {
        ProfileKind::Claude => {
            let root = match bytes {
                Some(bytes) => parse_settings(bytes)?,
                None => Map::new(),
            };
            let live_env = root
                .get("env")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            for (name, value) in &def.env {
                let text = value.as_str().unwrap_or("");
                let expected = match text.strip_prefix("${").and_then(|v| v.strip_suffix('}')) {
                    Some(var) => resolve(var),
                    None => Some(text.to_string()),
                };
                let live = live_env.get(name).and_then(Value::as_str);
                let state = match (live, &expected) {
                    (None, _) => "missing",
                    (Some(_), None) => "unresolved",
                    (Some(live), Some(expected)) if live == expected => "same",
                    _ => "differs",
                };
                fields.push(serde_json::json!({
                    "field": format!("env.{name}"),
                    "state": state,
                    "definition": text,
                    "live": live.map(|v| display_value(name, v)).unwrap_or(Value::Null),
                }));
            }
            for name in claude_cancelled_env(def) {
                let live = live_env.get(name).and_then(Value::as_str);
                fields.push(serde_json::json!({
                    "field": format!("env.{name}"),
                    "state": match live {
                        None => "missing",
                        Some("") => "same",
                        Some(_) => "differs",
                    },
                    "definition": "",
                    "live": live.map(|v| display_value(name, v)).unwrap_or(Value::Null),
                    // 取消 shell 继承的隔离项：界面合并成一句说明，不逐条铺开
                    "purpose": "isolation",
                }));
            }
            if let Some(model) = &def.model {
                let live = root.get("model").and_then(Value::as_str);
                fields.push(serde_json::json!({
                    "field": "model",
                    "state": match live {
                        None => "missing",
                        Some(live) if live == model => "same",
                        Some(_) => "differs",
                    },
                    "definition": model,
                    "live": live,
                }));
            }
        }
        ProfileKind::Codex => {
            let live = bytes.map(codex_fields).unwrap_or_default();
            let mut expected: Vec<(String, String, Option<String>)> = Vec::new();
            if let Some(model) = &def.model {
                expected.push(("model".into(), model.clone(), Some(model.clone())));
            }
            if let Some(url) = &def.base_url {
                for (field, value) in [
                    (CODEX_MODEL_PROVIDER, CODEX_PROVIDER_ID),
                    ("provider.name", def.display_name.as_str()),
                    ("provider.base_url", url.as_str()),
                    ("provider.wire_api", codex_wire_api(def)),
                ] {
                    expected.push((field.into(), value.into(), Some(value.into())));
                }
                if let Some(var) = &def.env_key {
                    expected.push((
                        format!("provider.{CODEX_BEARER_TOKEN}"),
                        format!("${{{var}}}"),
                        resolve(var),
                    ));
                }
            }
            for (field, definition, want) in expected {
                let have = live.get(&field);
                let secret = field.ends_with(CODEX_BEARER_TOKEN);
                fields.push(serde_json::json!({
                    "field": field,
                    "state": match (have, &want) {
                        (None, _) => "missing",
                        (Some(_), None) => "unresolved",
                        (Some(have), Some(want)) if have == want => "same",
                        _ => "differs",
                    },
                    "definition": definition,
                    "live": match have {
                        Some(_) if secret => Value::String("<hidden>".into()),
                        Some(have) => Value::String(have.clone()),
                        None => Value::Null,
                    },
                }));
            }
        }
    }
    Ok(fields)
}

/// Codex 接入表里放第三方凭据的字段。官方说明：取值作为 `Authorization: Bearer` 请求头
/// 发出，以程序方式配置时使用；0.48 起就有，CC Switch 保留官方登录时也写这个字段。
const CODEX_BEARER_TOKEN: &str = "experimental_bearer_token";

/// Codex 唯一支持的接入协议。`wire_api = "chat"` 在当前 Codex 里是反序列化错误
/// （`model-provider-info` 的 `CHAT_WIRE_API_REMOVED_ERROR`），整份 config.toml 都会读不了。
const CODEX_WIRE_API: &str = "responses";

fn codex_wire_api(def: &ProfileDef) -> &str {
    def.wire_api.as_deref().unwrap_or(CODEX_WIRE_API)
}

/// Codex 实例文件里与定义对应的字段（解析失败时为空：整份都算不一致）。
fn codex_fields(bytes: &[u8]) -> std::collections::BTreeMap<String, String> {
    let mut out = std::collections::BTreeMap::new();
    let Some(doc) = std::str::from_utf8(bytes)
        .ok()
        .and_then(|text| text.parse::<toml::Value>().ok())
    else {
        return out;
    };
    for key in ["model", CODEX_MODEL_PROVIDER] {
        if let Some(value) = doc.get(key).and_then(toml::Value::as_str) {
            out.insert(key.to_string(), value.to_string());
        }
    }
    if let Some(table) = doc
        .get("model_providers")
        .and_then(|providers| providers.get(CODEX_PROVIDER_ID))
        .and_then(toml::Value::as_table)
    {
        for key in ["name", "base_url", CODEX_BEARER_TOKEN, "wire_api"] {
            if let Some(value) = table.get(key).and_then(toml::Value::as_str) {
                out.insert(format!("provider.{key}"), value.to_string());
            }
        }
    }
    out
}

/// 采纳实际配置：把实际文件里的值写回实例定义的 profile 段（只改库，不动文件）。
/// 返回新的 profile 段与提示；令牌等凭据不以明文进库——实际取值与引用变量不一致时只提示。
pub(crate) fn adopt_profile(
    def: &ProfileDef,
    raw_profile: &Map<String, Value>,
    bytes: Option<&[u8]>,
    resolve: &dyn Fn(&str) -> Option<String>,
) -> Result<(Map<String, Value>, Vec<String>)> {
    let Some(bytes) = bytes else {
        bail!("[{}] 实例配置文件还不存在，没有可采纳的实际配置", def.id);
    };
    let mut profile = raw_profile.clone();
    let mut notes = Vec::new();
    match def.kind {
        ProfileKind::Claude => {
            let root = parse_settings(bytes)?;
            let live_env = root
                .get("env")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            let mut env = profile
                .get("env")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            for (name, value) in &def.env {
                let text = value.as_str().unwrap_or("");
                let Some(live) = live_env.get(name).and_then(Value::as_str) else {
                    notes.push(format!("实际配置里没有 {name}，保留定义里的值"));
                    continue;
                };
                match text.strip_prefix("${").and_then(|v| v.strip_suffix('}')) {
                    Some(var) => {
                        if resolve(var).as_deref() != Some(live) {
                            notes.push(format!(
                                "{name} 的实际取值与变量 {var} 不同；凭据不会以明文写进库，未采纳"
                            ));
                        }
                    }
                    None if live != text => {
                        if looks_like_credential(name)
                            || crate::native::reject_secret_text(live).is_some()
                        {
                            notes.push(format!("{name} 的实际取值像是凭据，未采纳"));
                        } else {
                            env.insert(name.clone(), Value::String(live.to_string()));
                        }
                    }
                    None => {}
                }
            }
            profile.insert("env".into(), Value::Object(env));
            match root.get("model").and_then(Value::as_str) {
                Some(model) => {
                    profile.insert("model".into(), Value::String(model.to_string()));
                }
                None => {
                    profile.remove("model");
                }
            }
        }
        ProfileKind::Codex => {
            let live = codex_fields(bytes);
            let ours =
                live.get(CODEX_MODEL_PROVIDER).map(String::as_str) == Some(CODEX_PROVIDER_ID);
            if !ours {
                notes.push("实际配置没有使用本实例的接入方式，只采纳模型".to_string());
            }
            for (field, key) in [
                ("model", "model"),
                ("provider.base_url", "base_url"),
                ("provider.wire_api", "wire_api"),
            ] {
                if key != "model" && !ours {
                    continue;
                }
                if key == "wire_api" && live.get(field).is_some_and(|value| value != CODEX_WIRE_API)
                {
                    notes.push(format!(
                        "实际配置的 wire_api 不是 {CODEX_WIRE_API}（Codex 已不支持其他取值），未采纳"
                    ));
                    continue;
                }
                match live.get(field) {
                    Some(value) => {
                        profile.insert(key.into(), Value::String(value.clone()));
                    }
                    None => {
                        profile.remove(key);
                    }
                }
            }
            if let (true, Some(var)) = (ours, &def.env_key) {
                let token = live.get(&format!("provider.{CODEX_BEARER_TOKEN}"));
                if token.is_some() && resolve(var).as_ref() != token {
                    notes.push(format!(
                        "{CODEX_BEARER_TOKEN} 的实际取值与变量 {var} 不同；凭据不会以明文写进库，未采纳"
                    ));
                }
            }
        }
    }
    Ok((profile, notes))
}

/// 渲染该实例配置目录下的全部受管文件（不含规则与 hooks，见 FILE_CLAUDE_RULES /
/// FILE_CODEX_HOOKS）。
///
/// `existing` 给出目标文件的现有字节：Claude 的 `settings.json` 采用合并写入，只改实例
/// 定义声明的 env 变量与 model；用户自己加的 env 变量、根级字段与 hooks 原样保留。
/// `recorded` 是上次写入记下的路径（`env.<名>` / `model`）：定义里已删掉的那几项在这次撤掉，
/// 从没由本工具写过的键不碰。现有文件不是 JSON 对象时拒绝，不当成空文件覆盖。
pub(crate) fn render_all(
    def: &ProfileDef,
    existing: &dyn Fn(&str) -> Option<Vec<u8>>,
    resolve: &dyn Fn(&str) -> Option<String>,
    recorded: &[String],
) -> Result<Vec<RenderedTarget>> {
    let mut out = Vec::new();
    match def.kind {
        ProfileKind::Claude => {
            let mut settings = match existing(FILE_CLAUDE_SETTINGS) {
                Some(bytes) => parse_settings(&bytes)?,
                None => Map::new(),
            };
            let mut env = settings
                .get("env")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            for path in recorded {
                if let Some(name) = path.strip_prefix("env.") {
                    if !def.env.contains_key(name) {
                        env.remove(name);
                    }
                }
            }
            for (name, value) in resolved_env(def, resolve)? {
                env.insert(name, value);
            }
            for name in claude_cancelled_env(def) {
                env.insert(name.to_string(), Value::String(String::new()));
            }
            if env.is_empty() {
                settings.remove("env");
            } else {
                settings.insert("env".into(), Value::Object(env));
            }
            match &def.model {
                Some(model) => {
                    settings.insert("model".into(), Value::String(model.clone()));
                }
                None if recorded.iter().any(|path| path == "model") => {
                    settings.remove("model");
                }
                None => {}
            }
            out.push(RenderedTarget {
                file: FILE_CLAUDE_SETTINGS.to_string(),
                content: render_json(&Value::Object(settings)),
            });
        }
        ProfileKind::Codex => {
            let text = match existing(FILE_CODEX_CONFIG) {
                Some(bytes) => {
                    parse_codex_config(&bytes)?;
                    String::from_utf8(bytes).expect("已校验为 UTF-8")
                }
                None => String::new(),
            };
            let mut doc = text
                .parse::<toml_edit::Document>()
                .map_err(|_| anyhow::anyhow!("实例的 config.toml 不是合法的 TOML，拒绝读写"))?;
            let owned_before = |path: &str| recorded.iter().any(|p| p == path);
            match &def.model {
                Some(model) => doc["model"] = toml_edit::value(model.as_str()),
                None if owned_before("model") => {
                    doc.remove("model");
                }
                None => {}
            }
            match codex_provider_table(def, resolve)? {
                Some(table) => {
                    doc[CODEX_MODEL_PROVIDER] = toml_edit::value(CODEX_PROVIDER_ID);
                    let providers = doc
                        .entry("model_providers")
                        .or_insert_with(|| {
                            let mut table = toml_edit::Table::new();
                            table.set_implicit(true);
                            toml_edit::Item::Table(table)
                        })
                        .as_table_mut()
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "实例 config.toml 里的 model_providers 不是普通表，拒绝读写"
                            )
                        })?;
                    providers.insert(CODEX_PROVIDER_ID, toml_edit::Item::Table(table));
                }
                None => {
                    if owned_before(CODEX_MODEL_PROVIDER) {
                        doc.remove(CODEX_MODEL_PROVIDER);
                    }
                    if owned_before(&codex_provider_path()) {
                        if let Some(providers) = doc
                            .get_mut("model_providers")
                            .and_then(toml_edit::Item::as_table_mut)
                        {
                            providers.remove(CODEX_PROVIDER_ID);
                            if providers.is_empty() {
                                doc.remove("model_providers");
                            }
                        }
                    }
                }
            }
            out.push(RenderedTarget {
                file: FILE_CODEX_CONFIG.to_string(),
                content: doc.to_string().into_bytes(),
            });
        }
    }
    Ok(out)
}

/// 写盘字节：pretty JSON（2 空格）+ 单个换行。
fn render_json(payload: &Value) -> Vec<u8> {
    let mut text = serde_json::to_string_pretty(payload).expect("render");
    text.push('\n');
    text.into_bytes()
}

/// env 引用解析：缺变量即 fail-closed，不写出不完整的配置。
fn resolved_env(
    def: &ProfileDef,
    resolve: &dyn Fn(&str) -> Option<String>,
) -> Result<Map<String, Value>> {
    let mut env = Map::new();
    for (name, value) in &def.env {
        let text = value.as_str().unwrap_or("");
        let resolved = if let Some(var) = text.strip_prefix("${").and_then(|v| v.strip_suffix('}'))
        {
            resolve(var).ok_or_else(|| {
                anyhow::anyhow!(
                    "[{}] 环境变量 {} 在当前环境里没有取值，拒绝写入不完整的配置",
                    def.id,
                    var
                )
            })?
        } else {
            text.to_string()
        };
        env.insert(name.clone(), Value::String(resolved));
    }
    Ok(env)
}

/// 第三方 Codex 实例的接入表（整张归本工具）。凭据取值写进 `experimental_bearer_token`，
/// 和 Claude 实例把令牌写进自己的 settings.json 一样只落在这个实例目录里，启动时不依赖
/// shell 里导出变量。变量取不到值时 fail-closed。
fn codex_provider_table(
    def: &ProfileDef,
    resolve: &dyn Fn(&str) -> Option<String>,
) -> Result<Option<toml_edit::Table>> {
    let Some(url) = &def.base_url else {
        return Ok(None);
    };
    let mut table = toml_edit::Table::new();
    table.insert("name", toml_edit::value(def.display_name.as_str()));
    table.insert("base_url", toml_edit::value(url.as_str()));
    if let Some(var) = &def.env_key {
        let token = resolve(var).ok_or_else(|| {
            anyhow::anyhow!(
                "[{}] 环境变量 {} 在当前环境里没有取值，拒绝写入不完整的配置",
                def.id,
                var
            )
        })?;
        table.insert(CODEX_BEARER_TOKEN, toml_edit::value(token));
    }
    table.insert("wire_api", toml_edit::value(codex_wire_api(def)));
    Ok(Some(table))
}

/// Codex 实例启动时清空的变量：Codex 把它们当作官方账号侧的凭据，优先于目录里存的登录
/// （`rust-v0.156.1` 源码 `login/src/auth/manager.rs`：`CODEX_ACCESS_TOKEN` 所有入口都读，
/// `CODEX_API_KEY` 由 `codex exec` 等非交互入口读），空值等同未设置。官方账号实例同样清空：
/// shell 里给别的账号导出的这两个变量会压过这个目录里的登录，多账号隔离就失效了。
pub(crate) const CODEX_CLEARED_ENV: [&str; 2] = ["CODEX_API_KEY", "CODEX_ACCESS_TOKEN"];

/// 启动命令取值：`<配置目录变量>="<目录>" <命令> <扩展参数>`，与别名域的取值格式一致。
/// Codex 实例在前面加上清空官方凭据变量的赋值。
pub(crate) fn launch_value(def: &ProfileDef, bin: &str, extra_args: &str) -> String {
    let mut out = String::new();
    if def.kind == ProfileKind::Codex {
        for name in CODEX_CLEARED_ENV {
            out.push_str(&format!("{name}= "));
        }
    }
    out.push_str(&format!(
        "{}=\"{}\" {}",
        def.kind.config_env(),
        def.config_dir_raw,
        bin
    ));
    let extra = extra_args.trim();
    if !extra.is_empty() {
        out.push(' ');
        out.push_str(extra);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn def(raw: Value) -> Result<ProfileDef> {
        parse_definition("cc-ly", &raw, true)
    }

    #[test]
    fn credential_env_must_be_a_reference() {
        let err = def(json!({
            "profile": {"base_agent": "claude-code", "config_dir": "~/.claude-ly",
                        "env": {"ANTHROPIC_AUTH_TOKEN": "plain-secret-value"}}
        }))
        .unwrap_err();
        let text = format!("{err:#}");
        assert!(text.contains("只接受"), "{text}");
        assert!(!text.contains("plain-secret-value"), "错误不得回显取值");
    }

    #[test]
    fn placeholder_credential_is_accepted_and_non_secret_literal_is_fine() {
        let parsed = def(json!({
            "profile": {"base_agent": "claude-code", "config_dir": "~/.claude-ly",
                        "env": {"ANTHROPIC_AUTH_TOKEN": "${MY_TOKEN}", "ANTHROPIC_BASE_URL": "https://api.example.com"}}
        }))
        .unwrap();
        assert_eq!(parsed.kind, ProfileKind::Claude);
        assert_eq!(parsed.env["ANTHROPIC_AUTH_TOKEN"], "${MY_TOKEN}");
    }

    /// 与接入配置同一口径：实际配置里已有的局域网 http 中转要能存下（界面另行提示明文
    /// 传输）；非 http(s) 的地址仍拒绝。
    #[test]
    fn http_base_url_is_accepted_but_other_schemes_are_rejected() {
        assert!(def(json!({
            "profile": {"base_agent": "claude-code", "config_dir": "~/.claude-ly",
                        "env": {"ANTHROPIC_BASE_URL": "http://10.0.0.5:8080", "ANTHROPIC_AUTH_TOKEN": "${T}"}}
        }))
        .is_ok());
        let err = def(json!({
            "profile": {"base_agent": "claude-code", "config_dir": "~/.claude-ly",
                        "env": {"ANTHROPIC_BASE_URL": "ftp://api.example.com", "ANTHROPIC_AUTH_TOKEN": "${T}"}}
        }))
        .unwrap_err();
        assert!(format!("{err:#}").contains("https"));
    }

    #[test]
    fn env_references_resolve_and_missing_variable_fails_closed() {
        let claude = def(json!({
            "profile": {"base_agent": "claude-code", "config_dir": "~/.claude-ly",
                        "env": {"ANTHROPIC_AUTH_TOKEN": "${MY_TOKEN}"}}
        }))
        .unwrap();
        let resolved = resolved_env(&claude, &|name| {
            (name == "MY_TOKEN").then(|| "resolved-value".to_string())
        })
        .unwrap();
        assert_eq!(resolved["ANTHROPIC_AUTH_TOKEN"], "resolved-value");
        // 变量取不到值时拒绝写出不完整配置（整份渲染在集成测试里覆盖）
        assert!(resolved_env(&claude, &|_| None).is_err());
    }

    /// 和 Claude 实例一样把凭据取值写进实例自己的配置文件：只用 `env_key` 时 Codex 要从
    /// 启动进程的环境变量里取值，别名不导出这个变量就报「Missing environment variable」。
    #[test]
    fn codex_instance_config_gets_resolved_token_and_keeps_other_entries() {
        let codex = parse_definition(
            "cx-work",
            &json!({
                "profile": {"base_agent": "codex", "config_dir": "~/.codex-work",
                            "base_url": "https://api.example.com/v1", "env_key": "MY_TOKEN", "model": "gpt-5.5"}
            }),
            true,
        )
        .unwrap();
        let existing =
            b"# mine\napproval_policy = \"never\"\n\n[mcp_servers.fs]\ncommand = \"npx\"\n"
                .to_vec();
        let token = |name: &str| (name == "MY_TOKEN").then(|| "resolved-value".to_string());
        let rendered = render_all(&codex, &|_| Some(existing.clone()), &token, &[]).unwrap();
        assert_eq!(rendered[0].file, "config.toml");
        let text = String::from_utf8(rendered[0].content.clone()).unwrap();
        let doc: toml::Table = text.parse().unwrap();
        assert_eq!(doc["model"].as_str(), Some("gpt-5.5"));
        assert_eq!(doc["model_provider"].as_str(), Some("agent-manager"));
        let provider = &doc["model_providers"]["agent-manager"];
        assert_eq!(
            provider["base_url"].as_str(),
            Some("https://api.example.com/v1")
        );
        assert_eq!(
            provider["experimental_bearer_token"].as_str(),
            Some("resolved-value")
        );
        assert_eq!(provider["wire_api"].as_str(), Some("responses"));
        assert!(
            provider.get("env_key").is_none(),
            "不再依赖启动时的环境变量"
        );
        assert_eq!(
            doc["approval_policy"].as_str(),
            Some("never"),
            "用户的设置原样保留"
        );
        assert_eq!(
            doc["mcp_servers"]["fs"]["command"].as_str(),
            Some("npx"),
            "MCP 条目原样保留"
        );
        assert!(text.starts_with("# mine\n"), "注释保留");
        // 变量取不到值时拒绝写出不完整配置
        assert!(render_all(&codex, &|_| Some(existing.clone()), &|_| None, &[]).is_err());
        // 现有文件不是合法 TOML：拒绝，不当成空文件覆盖
        assert!(render_all(&codex, &|_| Some(b"[[".to_vec()), &token, &[]).is_err());
    }

    #[test]
    fn codex_official_instance_leaves_provider_alone_and_drops_what_it_wrote_before() {
        let official = parse_definition(
            "cx-work",
            &json!({"profile": {"base_agent": "codex", "config_dir": "~/.codex-work"}}),
            true,
        )
        .unwrap();
        assert!(owned_paths(&official).is_empty());
        let existing = b"model_provider = \"agent-manager\"\n\n[model_providers.agent-manager]\nbase_url = \"https://a.example.com\"\n\n[model_providers.other]\nbase_url = \"https://b.example.com\"\n".to_vec();
        let untouched = render_all(&official, &|_| Some(existing.clone()), &|_| None, &[]).unwrap();
        assert_eq!(untouched[0].content, existing, "没写过的不碰");
        let recorded = [
            "model_provider".to_string(),
            "model_providers.agent-manager".to_string(),
        ];
        let rendered =
            render_all(&official, &|_| Some(existing.clone()), &|_| None, &recorded).unwrap();
        let doc: toml::Table = String::from_utf8(rendered[0].content.clone())
            .unwrap()
            .parse()
            .unwrap();
        assert!(doc.get("model_provider").is_none());
        assert!(doc["model_providers"].get("agent-manager").is_none());
        assert!(
            doc["model_providers"].get("other").is_some(),
            "别的接入表不碰"
        );
        // 凭据要和接口地址一起设置，否则 model_provider 会指向不存在的接入表
        let err = parse_definition(
            "cx-work",
            &json!({"profile": {"base_agent": "codex", "config_dir": "~/.codex-work", "env_key": "T"}}),
            true,
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("接口地址"), "{err:#}");
    }

    #[test]
    fn config_dir_guards() {
        let home = PathBuf::from("/home/u");
        let data = home.join("Library/Application Support/agent-manager");
        let mk = |dir: &str| ProfileDef {
            id: "cc-ly".into(),
            display_name: "x".into(),
            base_agent: "claude-code".into(),
            kind: ProfileKind::Claude,
            config_dir_raw: dir.into(),
            env: Map::new(),
            base_url: None,
            env_key: None,
            wire_api: None,
            model: None,
            enabled: true,
        };
        assert!(validate_config_dir(&mk("~/.claude-ly"), &home, &data).is_ok());
        for bad in [
            "~/.claude",
            "~",
            "/etc/x",
            "~/../escape",
            "~/Library/Application Support/agent-manager/x",
        ] {
            assert!(
                validate_config_dir(&mk(bad), &home, &data).is_err(),
                "{bad} 必须被拒绝"
            );
        }
    }

    #[test]
    fn codex_launch_value_only_points_codex_home_at_the_instance() {
        let codex = parse_definition(
            "cx-work",
            &json!({"profile": {"base_agent": "codex", "config_dir": "~/.codex-work"}}),
            true,
        )
        .unwrap();
        assert_eq!(codex.kind.target_file(), "config.toml");
        assert_eq!(
            launch_value(&codex, "codex", ""),
            "CODEX_API_KEY= CODEX_ACCESS_TOKEN= CODEX_HOME=\"~/.codex-work\" codex"
        );
    }

    #[test]
    fn launch_value_matches_alias_format() {
        let parsed = def(json!({
            "profile": {"base_agent": "claude-code", "config_dir": "~/.claude-ly"}
        }))
        .unwrap();
        assert_eq!(
            launch_value(&parsed, "claude", "--dangerously-skip-permissions"),
            "CLAUDE_CONFIG_DIR=\"~/.claude-ly\" claude --dangerously-skip-permissions"
        );
    }

    #[test]
    fn claude_instance_never_mixes_official_and_third_party_credentials() {
        let claude = |env: Value| {
            def(
                json!({"profile": {"base_agent": "claude-code", "config_dir": "~/.claude-ly", "env": env}}),
            )
        };
        let err = claude(json!({"ANTHROPIC_BASE_URL": "https://a.example.com"})).unwrap_err();
        assert!(format!("{err:#}").contains("官方账号登录发往"), "{err:#}");
        let err = claude(json!({"ANTHROPIC_AUTH_TOKEN": "${A}", "ANTHROPIC_API_KEY": "${B}"}))
            .unwrap_err();
        assert!(format!("{err:#}").contains("只能声明一个"), "{err:#}");
        let err =
            claude(json!({"ANTHROPIC_AUTH_TOKEN": "${A}", "CLAUDE_CODE_OAUTH_TOKEN": "${O}"}))
                .unwrap_err();
        assert!(format!("{err:#}").contains("官方账号凭据"), "{err:#}");
        // 官方账号实例可以用官方长期令牌；只带凭据、用默认地址也算第三方
        assert!(!is_third_party(
            &claude(json!({"CLAUDE_CODE_OAUTH_TOKEN": "${O}"})).unwrap()
        ));
        assert!(is_third_party(
            &claude(json!({"ANTHROPIC_API_KEY": "${K}"})).unwrap()
        ));
    }

    /// 实例没声明的凭据、云厂商开关、地址与附加请求头一律置空，取消 shell 继承：第三方实例只用
    /// 自己声明的那一个凭据，官方账号实例只用目录里的登录。
    #[test]
    fn claude_instances_cancel_every_inherited_credential_they_do_not_declare() {
        let cloud = [
            "env.CLAUDE_CODE_USE_ANTHROPIC_AWS",
            "env.CLAUDE_CODE_USE_BEDROCK",
            "env.CLAUDE_CODE_USE_FOUNDRY",
            "env.CLAUDE_CODE_USE_MANTLE",
            "env.CLAUDE_CODE_USE_VERTEX",
        ];
        let third = def(
            json!({"profile": {"base_agent": "claude-code", "config_dir": "~/.claude-ly",
            "env": {"ANTHROPIC_BASE_URL": "https://a.example.com", "ANTHROPIC_API_KEY": "${K}"}}}),
        )
        .unwrap();
        let mut paths = owned_paths(&third);
        paths.sort();
        let mut expected = vec![
            "env.ANTHROPIC_API_KEY",
            "env.ANTHROPIC_AUTH_TOKEN",
            "env.ANTHROPIC_BASE_URL",
            "env.ANTHROPIC_CUSTOM_HEADERS",
            "env.CLAUDE_CODE_OAUTH_TOKEN",
        ];
        expected.extend(cloud);
        expected.sort();
        assert_eq!(paths, expected);

        let official =
            def(json!({"profile": {"base_agent": "claude-code", "config_dir": "~/.claude-ly"}}))
                .unwrap();
        let mut paths = owned_paths(&official);
        paths.sort();
        let mut expected = vec![
            "env.ANTHROPIC_API_KEY",
            "env.ANTHROPIC_AUTH_TOKEN",
            "env.ANTHROPIC_BASE_URL",
            "env.ANTHROPIC_CUSTOM_HEADERS",
            "env.CLAUDE_CODE_OAUTH_TOKEN",
        ];
        expected.extend(cloud);
        expected.sort();
        assert_eq!(paths, expected);
        let rendered = render_all(&official, &|_| None, &|_| None, &[]).unwrap();
        let settings: Value = serde_json::from_slice(&rendered[0].content).unwrap();
        for path in &expected {
            let name = path.strip_prefix("env.").unwrap();
            assert_eq!(settings["env"][name], "", "{name} 必须置空取消 shell 继承");
        }
        assert_eq!(
            launch_value(&official, "claude", ""),
            "CLAUDE_CONFIG_DIR=\"~/.claude-ly\" claude"
        );

        // 官方账号实例自己声明的长期令牌照常写取值，不被置空
        let token = def(
            json!({"profile": {"base_agent": "claude-code", "config_dir": "~/.claude-ly",
            "env": {"CLAUDE_CODE_OAUTH_TOKEN": "${SETUP_TOKEN}"}}}),
        )
        .unwrap();
        let rendered = render_all(
            &token,
            &|_| None,
            &|name| (name == "SETUP_TOKEN").then(|| "tok".to_string()),
            &[],
        )
        .unwrap();
        let settings: Value = serde_json::from_slice(&rendered[0].content).unwrap();
        assert_eq!(settings["env"]["CLAUDE_CODE_OAUTH_TOKEN"], "tok");
        assert_eq!(settings["env"]["ANTHROPIC_API_KEY"], "");
    }

    /// 第三方实例里打开云厂商开关，Claude 会改走云厂商、不用实例的第三方凭据：定义就拒绝。
    #[test]
    fn third_party_claude_instance_rejects_a_cloud_provider_switch() {
        let claude = |env: Value| {
            def(
                json!({"profile": {"base_agent": "claude-code", "config_dir": "~/.claude-ly", "env": env}}),
            )
        };
        let err = claude(json!({"ANTHROPIC_BASE_URL": "https://a.example.com",
            "ANTHROPIC_AUTH_TOKEN": "${A}", "CLAUDE_CODE_USE_MANTLE": "1"}))
        .unwrap_err();
        assert!(
            format!("{err:#}").contains("CLAUDE_CODE_USE_MANTLE"),
            "{err:#}"
        );
        // 显式置空是允许的；不带第三方凭据的 Bedrock 实例也照常
        assert!(claude(json!({"ANTHROPIC_BASE_URL": "https://a.example.com",
            "ANTHROPIC_AUTH_TOKEN": "${A}", "CLAUDE_CODE_USE_BEDROCK": ""}))
        .is_ok());
        let bedrock = claude(json!({"CLAUDE_CODE_USE_BEDROCK": "1"})).unwrap();
        let rendered = render_all(&bedrock, &|_| None, &|_| None, &[]).unwrap();
        let settings: Value = serde_json::from_slice(&rendered[0].content).unwrap();
        assert_eq!(
            settings["env"]["CLAUDE_CODE_USE_BEDROCK"], "1",
            "自己声明的开关不被置空"
        );
        assert_eq!(settings["env"]["CLAUDE_CODE_USE_VERTEX"], "");
    }

    #[test]
    fn codex_launch_clears_env_credentials_for_every_instance() {
        let third = def(
            json!({"profile": {"base_agent": "codex", "config_dir": "~/.codex-w",
            "base_url": "http://127.0.0.1:11434/v1"}}),
        )
        .unwrap();
        assert_eq!(
            launch_value(&third, "codex", ""),
            "CODEX_API_KEY= CODEX_ACCESS_TOKEN= CODEX_HOME=\"~/.codex-w\" codex"
        );
        let official =
            def(json!({"profile": {"base_agent": "codex", "config_dir": "~/.codex-w"}})).unwrap();
        // 官方账号实例同样清空：shell 里给别的账号导出的访问令牌会压过这个目录里的登录
        assert_eq!(
            launch_value(&official, "codex", ""),
            "CODEX_API_KEY= CODEX_ACCESS_TOKEN= CODEX_HOME=\"~/.codex-w\" codex"
        );
    }

    #[test]
    fn profile_id_rules() {
        assert!(valid_profile_id("cc-ly"));
        assert!(valid_profile_id("work2"));
        assert!(!valid_profile_id("-bad"));
        assert!(!valid_profile_id("Bad"));
        assert!(!valid_profile_id("has_underscore"));
    }
}

#[cfg(test)]
mod live_tests {
    use super::*;
    use serde_json::json;

    fn claude_def(env: Value, model: Option<&str>) -> ProfileDef {
        let mut profile =
            json!({"base_agent": "claude-code", "config_dir": "~/.claude-ly", "env": env});
        if let Some(model) = model {
            profile["model"] = json!(model);
        }
        parse_definition("cc-ly", &json!({ "profile": profile }), true).unwrap()
    }

    #[test]
    fn base_ignores_user_keys_outside_env_and_model() {
        let a = br#"{"env":{"A":"1"},"model":"m","theme":"dark"}"#;
        let b = br#"{"env":{"A":"1"},"model":"m","theme":"light","hooks":{"Stop":[]}}"#;
        let c = br#"{"env":{"A":"2"},"model":"m"}"#;
        assert_eq!(
            live_base(ProfileKind::Claude, Some(a)).unwrap(),
            live_base(ProfileKind::Claude, Some(b)).unwrap()
        );
        assert_ne!(
            live_base(ProfileKind::Claude, Some(a)).unwrap(),
            live_base(ProfileKind::Claude, Some(c)).unwrap()
        );
        assert_eq!(live_base(ProfileKind::Claude, None).unwrap(), BASE_MISSING);
        assert!(live_base(ProfileKind::Claude, Some(b"not json")).is_err());
        // Codex：MCP 条目等其他内容不影响基准，本工具那几项变了才变
        let codex = |text: &str| live_base(ProfileKind::Codex, Some(text.as_bytes())).unwrap();
        let ours = "model = \"m\"\n[model_providers.agent-manager]\nbase_url = \"https://a.example.com\"\n";
        assert_eq!(
            codex(ours),
            codex(&format!("{ours}\n[mcp_servers.fs]\ncommand = \"npx\"\n"))
        );
        assert_ne!(codex(ours), codex(&ours.replace("a.example", "b.example")));
        assert!(live_base(ProfileKind::Codex, Some(b"[[")).is_err());
    }

    #[test]
    fn codex_compare_hides_the_token_and_reports_unresolved() {
        let def = parse_definition(
            "cx-work",
            &json!({"profile": {"base_agent": "codex", "config_dir": "~/.codex-work",
                    "base_url": "https://a.example.com/v1", "env_key": "TOK"}}),
            true,
        )
        .unwrap();
        let live = b"model_provider = \"agent-manager\"\n[model_providers.agent-manager]\nname = \"cx-work\"\nbase_url = \"https://a.example.com/v1\"\nexperimental_bearer_token = \"sk-live-secret-123456\"\nwire_api = \"responses\"\n";
        let fields = |resolve: &dyn Fn(&str) -> Option<String>| {
            compare_fields(&def, Some(live), resolve).unwrap()
        };
        let token = |fields: &[Value]| {
            fields
                .iter()
                .find(|f| f["field"] == "provider.experimental_bearer_token")
                .unwrap()
                .clone()
        };
        let same = fields(&|_| Some("sk-live-secret-123456".into()));
        assert!(same.iter().all(|f| f["state"] == "same"), "{same:?}");
        assert_eq!(token(&same)["live"], "<hidden>");
        assert_eq!(token(&same)["definition"], "${TOK}");
        assert_eq!(
            token(&fields(&|_| Some("other".into())))["state"],
            "differs"
        );
        assert_eq!(token(&fields(&|_| None))["state"], "unresolved");
        assert!(!serde_json::to_string(&fields(&|_| Some("other".into())))
            .unwrap()
            .contains("sk-live-secret"));
    }

    #[test]
    fn render_only_touches_declared_env_keys_and_keeps_user_ones() {
        let def = claude_def(
            json!({"ANTHROPIC_BASE_URL": "https://a.example.com", "ANTHROPIC_AUTH_TOKEN": "${T}"}),
            None,
        );
        let t_var = |name: &str| (name == "T").then(|| "resolved".to_string());
        let existing =
            br#"{"env":{"MY_VAR":"1","OLD":"x"},"model":"user-model","theme":"dark"}"#.to_vec();
        let rendered = render_all(
            &def,
            &|_| Some(existing.clone()),
            &t_var,
            &["env.OLD".to_string()],
        )
        .unwrap();
        let out: Value = serde_json::from_slice(&rendered[0].content).unwrap();
        assert_eq!(out["env"]["MY_VAR"], "1", "用户自己的 env 变量保留");
        assert!(
            out["env"].get("OLD").is_none(),
            "定义里删掉、上次由本工具写的变量撤掉"
        );
        assert_eq!(out["env"]["ANTHROPIC_BASE_URL"], "https://a.example.com");
        assert_eq!(
            out["model"], "user-model",
            "定义没管 model 时不碰用户的 model"
        );
        assert_eq!(out["theme"], "dark");
        // 上次由本工具写过 model、定义里去掉了 → 撤掉
        let rendered =
            render_all(&def, &|_| Some(existing.clone()), &t_var, &["model".into()]).unwrap();
        let out: Value = serde_json::from_slice(&rendered[0].content).unwrap();
        assert!(out.get("model").is_none());
        // 现有文件不是 JSON 对象：拒绝，不当成空文件覆盖
        assert!(render_all(&def, &|_| Some(b"[1]".to_vec()), &t_var, &[]).is_err());
    }

    #[test]
    fn compare_reports_differences_without_leaking_credentials() {
        let def = claude_def(
            json!({"ANTHROPIC_BASE_URL": "https://a.example.com", "ANTHROPIC_AUTH_TOKEN": "${TOK}"}),
            Some("m1"),
        );
        let live = br#"{"env":{"ANTHROPIC_BASE_URL":"https://b.example.com","ANTHROPIC_AUTH_TOKEN":"sk-live-secret-123456"},"model":"m1"}"#;
        let fields = compare_fields(&def, Some(live), &|var| {
            (var == "TOK").then(|| "other".to_string())
        })
        .unwrap();
        let by = |f: &str| fields.iter().find(|x| x["field"] == f).unwrap().clone();
        assert_eq!(by("env.ANTHROPIC_BASE_URL")["state"], "differs");
        assert_eq!(
            by("env.ANTHROPIC_BASE_URL")["live"],
            "https://b.example.com"
        );
        assert_eq!(by("env.ANTHROPIC_AUTH_TOKEN")["state"], "differs");
        assert_eq!(by("env.ANTHROPIC_AUTH_TOKEN")["live"], "<hidden>");
        assert_eq!(by("model")["state"], "same");
        assert!(!serde_json::to_string(&fields)
            .unwrap()
            .contains("sk-live-secret"));
    }

    #[test]
    fn adopt_takes_plain_values_but_never_plaintext_credentials() {
        let def = claude_def(
            json!({"ANTHROPIC_BASE_URL": "https://a.example.com", "ANTHROPIC_AUTH_TOKEN": "${TOK}"}),
            Some("m1"),
        );
        let raw = json!({"base_agent": "claude-code", "config_dir": "~/.claude-ly",
            "env": {"ANTHROPIC_BASE_URL": "https://a.example.com", "ANTHROPIC_AUTH_TOKEN": "${TOK}"}, "model": "m1"});
        let live = br#"{"env":{"ANTHROPIC_BASE_URL":"http://10.0.0.5:8080","ANTHROPIC_AUTH_TOKEN":"sk-live-secret-123456"},"model":"m2"}"#;
        let (profile, notes) = adopt_profile(&def, raw.as_object().unwrap(), Some(live), &|_| {
            Some("other".into())
        })
        .unwrap();
        assert_eq!(profile["env"]["ANTHROPIC_BASE_URL"], "http://10.0.0.5:8080");
        assert_eq!(
            profile["env"]["ANTHROPIC_AUTH_TOKEN"], "${TOK}",
            "凭据只保留引用"
        );
        assert_eq!(profile["model"], "m2");
        assert!(notes.iter().any(|n| n.contains("ANTHROPIC_AUTH_TOKEN")));
        assert!(!serde_json::to_string(&profile)
            .unwrap()
            .contains("sk-live-secret"));
        // 采纳结果仍是合法定义（局域网 http 地址按接入配置同一口径放行）
        let mut full = serde_json::Map::new();
        full.insert("profile".into(), Value::Object(profile));
        assert!(parse_definition("cc-ly", &Value::Object(full), true).is_ok());
    }

    /// Codex 已移除 `wire_api = "chat"`：写进去整份 config.toml 都读不了，定义里就拒绝。
    #[test]
    fn codex_wire_api_only_accepts_responses() {
        let codex = |api: &str| {
            parse_definition(
                "cx-relay",
                &json!({"profile": {"base_agent": "codex", "config_dir": "~/.codex-relay",
                    "base_url": "https://a.example.com/v1", "env_key": "RELAY_KEY", "wire_api": api}}),
                true,
            )
        };
        assert!(codex("responses").is_ok());
        let err = codex("chat").unwrap_err();
        assert!(format!("{err:#}").contains("responses"), "{err:#}");
        assert!(codex("anything").is_err());
    }

    /// 实际文件里残留 `wire_api = "chat"` 时，采纳不把它搬进定义（否则定义本身就解析不了）。
    #[test]
    fn adopt_does_not_take_a_removed_codex_wire_api() {
        let raw = json!({"base_agent": "codex", "config_dir": "~/.codex-relay",
            "base_url": "https://a.example.com/v1", "env_key": "RELAY_KEY", "wire_api": "responses"});
        let def = parse_definition("cx-relay", &json!({"profile": raw.clone()}), true).unwrap();
        let live = b"model_provider = \"agent-manager\"\n[model_providers.agent-manager]\nname = \"cx-relay\"\nbase_url = \"https://b.example.com/v1\"\nwire_api = \"chat\"\n";
        let (profile, notes) =
            adopt_profile(&def, raw.as_object().unwrap(), Some(live), &|_| None).unwrap();
        assert_eq!(profile["base_url"], "https://b.example.com/v1");
        assert_eq!(profile["wire_api"], "responses", "保留定义里的取值");
        assert!(notes.iter().any(|n| n.contains("wire_api")), "{notes:?}");
        let mut full = serde_json::Map::new();
        full.insert("profile".into(), Value::Object(profile));
        assert!(parse_definition("cx-relay", &Value::Object(full), true).is_ok());
    }
}
