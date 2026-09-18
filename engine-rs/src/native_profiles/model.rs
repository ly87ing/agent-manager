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

/// Codex 叠加配置的固定名字：文件名与 `-p` 参数取值必须一致。
pub(crate) const CODEX_PROFILE_NAME: &str = "agent-manager";
const CODEX_PROFILE_FILE: &str = "agent-manager.config.toml";

/// profile 的基础助手类型：决定配置目录变量与目标文件格式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProfileKind {
    /// `CLAUDE_CONFIG_DIR` + `<dir>/settings.json`
    Claude,
    /// `CODEX_HOME` + `<dir>/agent-manager.config.toml`（经 `-p` 叠加，避开归 MCP 域的 config.toml）
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

    /// 本工具在配置目录里全权拥有的目标文件（相对配置目录）。
    ///
    /// Codex 特意不写 `config.toml`：那个文件归 MCP 域写 `mcp_servers`，两个域整文件写入
    /// 会互相抹掉。改用 Codex 官方的叠加机制——单独一份 `<name>.config.toml`，启动时 `-p` 叠加。
    pub(crate) fn target_file(self) -> &'static str {
        match self {
            ProfileKind::Claude => "settings.json",
            ProfileKind::Codex => CODEX_PROFILE_FILE,
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
    /// Codex：接口地址与鉴权变量名（`env_key` 只写变量名，永不写值）。
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
    let upper = name.to_ascii_uppercase();
    ["TOKEN", "KEY", "SECRET", "PASSWORD", "CREDENTIAL", "AUTH"]
        .iter()
        .any(|needle| upper.contains(needle))
}

fn valid_env_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// 接口地址：仅 https，或本机回环 http（与 MCP 远程定义同一口径）。错误不回显地址。
fn validate_base_url(url: &str) -> Result<()> {
    if url.starts_with("https://") {
        return Ok(());
    }
    if url.starts_with("http://localhost") || url.starts_with("http://127.0.0.1") {
        return Ok(());
    }
    bail!("接口地址只允许 https，或本机回环 http://localhost / http://127.0.0.1");
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
    }
    let wire_api = profile
        .get("wire_api")
        .and_then(Value::as_str)
        .map(str::to_string);
    if let Some(api) = &wire_api {
        if !matches!(api.as_str(), "responses" | "chat") {
            bail!("[{}] wire_api 只支持 responses 或 chat", id);
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

/// 渲染目标文件内容。Claude 的 env 值在此解析 `${VAR}`（缺变量即 fail-closed）；
/// Codex 只写 `env_key` 变量名，鉴权值永不进入文件。
pub(crate) fn render_target(
    def: &ProfileDef,
    resolve: &dyn Fn(&str) -> Option<String>,
) -> Result<String> {
    match def.kind {
        ProfileKind::Claude => {
            let mut env = Map::new();
            for (name, value) in &def.env {
                let text = value.as_str().unwrap_or("");
                let resolved =
                    if let Some(var) = text.strip_prefix("${").and_then(|v| v.strip_suffix('}')) {
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
            let mut root = Map::new();
            root.insert("env".into(), Value::Object(env));
            if let Some(model) = &def.model {
                root.insert("model".into(), Value::String(model.clone()));
            }
            Ok(serde_json::to_string_pretty(&Value::Object(root))? + "\n")
        }
        ProfileKind::Codex => {
            let provider = CODEX_PROFILE_NAME;
            let mut out = String::new();
            if let Some(model) = &def.model {
                out.push_str(&format!("model = {}\n", toml_string(model)));
            }
            out.push_str(&format!("model_provider = {}\n", toml_string(provider)));
            if let Some(url) = &def.base_url {
                out.push_str(&format!("\n[model_providers.{provider}]\n"));
                out.push_str(&format!(
                    "name = {}\n",
                    toml_string(def.display_name.as_str())
                ));
                out.push_str(&format!("base_url = {}\n", toml_string(url)));
                if let Some(key) = &def.env_key {
                    out.push_str(&format!("env_key = {}\n", toml_string(key)));
                }
                out.push_str(&format!(
                    "wire_api = {}\n",
                    toml_string(def.wire_api.as_deref().unwrap_or("responses"))
                ));
            }
            Ok(out)
        }
    }
}

/// TOML 基本字符串字面量：只需转义反斜杠与双引号（取值里的控制字符已在校验层拒绝）。
fn toml_string(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

/// 启动命令取值：`<配置目录变量>="<目录>" <命令> <扩展参数>`，与别名域的取值格式一致。
pub(crate) fn launch_value(def: &ProfileDef, bin: &str, extra_args: &str) -> String {
    let mut out = format!(
        "{}=\"{}\" {}",
        def.kind.config_env(),
        def.config_dir_raw,
        bin
    );
    if def.kind == ProfileKind::Codex {
        out.push_str(&format!(" -p {CODEX_PROFILE_NAME}"));
    }
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

    #[test]
    fn plain_http_base_url_is_rejected() {
        let err = def(json!({
            "profile": {"base_agent": "claude-code", "config_dir": "~/.claude-ly",
                        "env": {"ANTHROPIC_BASE_URL": "http://api.example.com"}}
        }))
        .unwrap_err();
        assert!(format!("{err:#}").contains("https"));
    }

    #[test]
    fn claude_target_resolves_references_and_codex_never_writes_values() {
        let claude = def(json!({
            "profile": {"base_agent": "claude-code", "config_dir": "~/.claude-ly",
                        "env": {"ANTHROPIC_AUTH_TOKEN": "${MY_TOKEN}"}}
        }))
        .unwrap();
        let rendered = render_target(&claude, &|name| {
            (name == "MY_TOKEN").then(|| "resolved-value".to_string())
        })
        .unwrap();
        assert!(rendered.contains("resolved-value"));

        let codex = parse_definition(
            "cx-work",
            &json!({
                "profile": {"base_agent": "codex", "config_dir": "~/.codex-work",
                            "base_url": "https://api.example.com/v1", "env_key": "MY_TOKEN", "model": "gpt-5.5"}
            }),
            true,
        )
        .unwrap();
        let rendered = render_target(&codex, &|_| Some("resolved-value".to_string())).unwrap();
        assert!(rendered.contains("env_key = \"MY_TOKEN\""));
        assert!(
            !rendered.contains("resolved-value"),
            "Codex 目标不得出现解析后的值"
        );
    }

    #[test]
    fn missing_variable_fails_closed() {
        let claude = def(json!({
            "profile": {"base_agent": "claude-code", "config_dir": "~/.claude-ly",
                        "env": {"ANTHROPIC_AUTH_TOKEN": "${MISSING}"}}
        }))
        .unwrap();
        assert!(render_target(&claude, &|_| None).is_err());
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
    fn codex_launch_value_layers_its_own_profile_file() {
        let codex = parse_definition(
            "cx-work",
            &json!({"profile": {"base_agent": "codex", "config_dir": "~/.codex-work"}}),
            true,
        )
        .unwrap();
        assert_eq!(codex.kind.target_file(), "agent-manager.config.toml");
        assert_eq!(
            launch_value(&codex, "codex", ""),
            "CODEX_HOME=\"~/.codex-work\" codex -p agent-manager"
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
    fn profile_id_rules() {
        assert!(valid_profile_id("cc-ly"));
        assert!(valid_profile_id("work2"));
        assert!(!valid_profile_id("-bad"));
        assert!(!valid_profile_id("Bad"));
        assert!(!valid_profile_id("has_underscore"));
    }
}
