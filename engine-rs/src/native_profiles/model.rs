//! Immutable extension target metadata and credential-ownership validation.
//!
//! Historical profile records locate extension files and reserve credential names.
//! This module neither reads login state nor renders or writes account settings.

use anyhow::{bail, Result};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

/// 实例目录下的规则文件：以实际文件为准，归 rules 域作为规则目标直接编辑，
/// profiles 域不渲染、不写、不检测它。
pub(crate) const FILE_CLAUDE_RULES: &str = "CLAUDE.md";
pub(crate) const FILE_CODEX_RULES: &str = "AGENTS.md";

/// profile 的基础助手类型：决定配置目录变量与目标文件格式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProfileKind {
    /// `CLAUDE_CONFIG_DIR` + `<dir>/settings.json`
    Claude,
    /// Historical Codex configuration directory.
    Codex,
}

impl ProfileKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            ProfileKind::Claude => "claude",
            ProfileKind::Codex => "codex",
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
    pub kind: ProfileKind,
    /// 登记形式（`~/...`），展示与持久化都用它。
    pub config_dir_raw: String,
    /// Historical environment references reserve their credential variables.
    pub env: Map<String, Value>,
    /// Historical endpoint and variable metadata; values are never rendered.
    pub base_url: Option<String>,
    pub env_key: Option<String>,
    pub enabled: bool,
}

/// Historical registration identifier syntax.
pub(crate) fn valid_profile_id(id: &str) -> bool {
    if matches!(id, "claude" | "claude-code" | "codex" | "opencode") {
        return false;
    }
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
        Some(host) if !host.is_empty() && !host.starts_with('/') => {}
        _ => bail!("接口地址只允许 https:// 或 http:// 开头的完整地址"),
    }
    if url
        .chars()
        .any(|character| character.is_control() || character.is_whitespace())
    {
        bail!("接口地址包含非法空白或控制字符");
    }
    if crate::native::url_has_userinfo(url) {
        bail!("接口地址里不能带账号密码：凭据请用令牌变量");
    }
    Ok(())
}

/// 展开 `~/` 前缀；其余原样。
pub(crate) fn expand_home(path: &str, home_dir: &Path) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        return crate::platform::join_slash(home_dir, rest);
    }
    if path == "~" {
        return home_dir.to_path_buf();
    }
    PathBuf::from(path)
}

/// Validate the actual directory, including existing ancestors and protected boundaries.
pub(crate) fn validate_config_dir(
    def: &ProfileDef,
    home_dir: &Path,
    data_dir: &Path,
) -> Result<PathBuf> {
    super::paths::validate_config_dir(def, home_dir, data_dir)
}

/// 解析单个 profile 定义；结构非法即 fail-closed，消息不含取值正文。
pub(crate) fn parse_definition(id: &str, raw: &Value, enabled: bool) -> Result<ProfileDef> {
    if matches!(id, "claude" | "claude-code" | "codex" | "opencode") {
        bail!("该名称保留给主账号或内置助手，拒绝登记为独立账号");
    }
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
    let canonical_base = match kind {
        ProfileKind::Claude => base_agent == "claude-code" || base_agent == "claude",
        ProfileKind::Codex => base_agent == "codex",
    };
    if !canonical_base {
        bail!("[{}] 基础助手与账号类型不一致，拒绝操作", id);
    }
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
            if crate::native_account_context::process_override_env(name)
                || [
                    "HOME",
                    "USERPROFILE",
                    "HOMEDRIVE",
                    "HOMEPATH",
                    "PATH",
                    "NODE_OPTIONS",
                    "LD_PRELOAD",
                    "DYLD_INSERT_LIBRARIES",
                    "CLAUDE_CONFIG_DIR",
                    "CLAUDE_SECURESTORAGE_CONFIG_DIR",
                    "ANTHROPIC_CONFIG_DIR",
                    "CODEX_HOME",
                ]
                .contains(&name.to_ascii_uppercase().as_str())
            {
                bail!("[{}] 定义不能覆盖受控账号目录或运行时上下文", id);
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
    if kind == ProfileKind::Codex && !env.is_empty() {
        bail!(
            "[{}] Codex 账号只允许通过 env_key 声明凭据，不能叠加 env 认证来源",
            id
        );
    }
    if kind == ProfileKind::Claude && (base_url.is_some() || env_key.is_some()) {
        bail!(
            "[{}] Claude 账号必须在 env 中明确声明接口与凭据，不能混用 Codex 接入字段",
            id
        );
    }
    if kind == ProfileKind::Codex && base_url.is_some() && env_key.is_none() {
        bail!("[{}] Codex API 账号必须同时声明接口地址与独占凭据变量", id);
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
    Ok(ProfileDef {
        id: id.to_string(),
        display_name: obj
            .get("display_name")
            .and_then(Value::as_str)
            .unwrap_or(id)
            .to_string(),
        kind,
        config_dir_raw,
        env,
        base_url,
        env_key,
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
        env.get(*name)
            .and_then(Value::as_str)
            .is_some_and(|value| !value.is_empty())
    }) {
        bail!("[{id}] {name} 会改变认证服务商；当前受控账号入口不能可靠绑定该模式，拒绝操作");
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

/// 从有效策略里取出全部 profile 定义（带 profile 段的 agent 定义），按 id 排序。
pub(crate) fn load_definitions(policy: &Value) -> Vec<Result<ProfileDef>> {
    load_definition_entries(policy)
        .into_iter()
        .map(|(_, _, parsed)| parsed)
        .collect()
}

/// Keep invalid historical registrations visible to extension target validation.
pub(crate) fn load_definition_entries(policy: &Value) -> Vec<(String, Value, Result<ProfileDef>)> {
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
        out.push((id.clone(), raw.clone(), parse_definition(id, raw, enabled)));
    }
    out
}

const CODEX_WIRE_API: &str = "responses";
const MAX_ACCOUNT_CONFIG_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AccountAuthMode {
    Official,
    Api,
    OfficialToken,
}

pub(crate) fn account_auth_mode(def: &ProfileDef) -> AccountAuthMode {
    if is_third_party(def) {
        AccountAuthMode::Api
    } else if def.kind == ProfileKind::Claude
        && def
            .env
            .get(CLAUDE_OFFICIAL_TOKEN)
            .and_then(Value::as_str)
            .is_some_and(|value| !value.is_empty())
    {
        AccountAuthMode::OfficialToken
    } else {
        AccountAuthMode::Official
    }
}

pub(crate) fn account_secret_names(def: &ProfileDef) -> std::collections::BTreeSet<String> {
    let mut names: std::collections::BTreeSet<String> = def
        .env
        .values()
        .filter_map(Value::as_str)
        .filter_map(|text| text.strip_prefix("${")?.strip_suffix('}'))
        .map(str::to_string)
        .collect();
    names.extend(def.env_key.clone());
    names
}

pub(crate) fn validate_secret_ownership_in_policy(
    ctx: &crate::native::Context,
    def: &ProfileDef,
    policy: &Value,
) -> Result<()> {
    let mut names = crate::native_secrets::account_reference_names(ctx, &def.id, policy)?;
    names.extend(account_secret_names(def));
    if names.is_empty() {
        return Ok(());
    }
    let owners = crate::native_secrets::account_secret_owners_in_policy(ctx, policy)?;
    for name in names {
        if owners
            .get(&name.to_ascii_uppercase())
            .is_some_and(|ids| ids.iter().any(|id| id != &def.id))
        {
            bail!(
                "[{}] 密钥变量被另一账号或已退役代际引用，拒绝操作；请显式使用账号独占变量",
                def.id
            );
        }
    }
    Ok(())
}

pub(crate) fn read_account_file(path: &Path) -> Result<Option<Vec<u8>>> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => bail!("账号配置或认证文件不可读取，拒绝操作"),
    };
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > MAX_ACCOUNT_CONFIG_BYTES as u64
    {
        bail!("账号配置或认证文件不是普通文件或超过安全读取上限，拒绝操作");
    }
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|_| anyhow::anyhow!("账号配置或认证文件不可读取，拒绝操作"))?
        .take(MAX_ACCOUNT_CONFIG_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| anyhow::anyhow!("账号配置或认证文件不可读取，拒绝操作"))?;
    if bytes.len() > MAX_ACCOUNT_CONFIG_BYTES {
        bail!("账号配置或认证文件超过安全读取上限，拒绝操作");
    }
    Ok(Some(bytes))
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
    fn config_dir_guards() {
        // 各平台都是绝对路径（Windows 上 `/home/u` 不算绝对路径）
        let home = std::env::temp_dir().join("am-home-u");
        let data = home.join("Library/Application Support/agent-manager");
        let mk = |dir: &str| ProfileDef {
            id: "cc-ly".into(),
            display_name: "x".into(),
            kind: ProfileKind::Claude,
            config_dir_raw: dir.into(),
            env: Map::new(),
            base_url: None,
            env_key: None,
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
        // An explicit blank switch is inert. Cloud authentication is not inferred as official.
        assert!(claude(json!({"ANTHROPIC_BASE_URL": "https://a.example.com",
            "ANTHROPIC_AUTH_TOKEN": "${A}", "CLAUDE_CODE_USE_BEDROCK": ""}))
        .is_ok());
        assert!(claude(json!({"CLAUDE_CODE_USE_BEDROCK": "1"})).is_err());
    }

    #[test]
    fn profile_id_rules() {
        assert!(valid_profile_id("cc-ly"));
        assert!(valid_profile_id("work2"));
        for id in ["claude", "claude-code", "codex", "opencode"] {
            assert!(!valid_profile_id(id));
        }
        assert!(!valid_profile_id("-bad"));
        assert!(!valid_profile_id("Bad"));
        assert!(!valid_profile_id("has_underscore"));
    }
}
