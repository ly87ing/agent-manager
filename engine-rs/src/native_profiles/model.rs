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

use crate::native::Context;

/// Codex 叠加配置的固定名字：文件名与 `-p` 参数取值必须一致。
pub(crate) const CODEX_PROFILE_NAME: &str = "agent-manager";
const CODEX_PROFILE_FILE: &str = "agent-manager.config.toml";

/// 实例配置目录里由本工具维护的文件。
///
/// Hooks 的主实例目标是固定路径（`~/.claude`、`~/.codex`），不随实例走；实例目录下的
/// 配置与 Hooks 文件因此由 profiles 域一并渲染。
pub(crate) const FILE_CLAUDE_SETTINGS: &str = "settings.json";
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

/// 本工具在实例 settings.json 里真正拥有的键：env / model / hooks。
/// 其余根级字段是用户自己的（合并写入时原样保留），因此外部修改检测不该看它们。
const CLAUDE_SETTINGS_OWNED_KEYS: [&str; 3] = ["env", "model", "hooks"];

/// 某个受管文件里「本工具拥有的那部分」的摘要。
///
/// settings.json 是合并写入的，用户可以在里面放自己的设置（permissions、statusLine…），
/// 拿整文件做外部修改检测会把用户改这些误判成冲突。其余文件整份归本工具，比整文件。
pub(crate) fn owned_slice_digest(file: &str, bytes: &[u8]) -> String {
    if file != FILE_CLAUDE_SETTINGS {
        return crate::native_mcp::model::sha256_hex_bytes(bytes);
    }
    let owned = serde_json::from_slice::<Value>(bytes)
        .ok()
        .and_then(|v| v.as_object().cloned())
        .map(|obj| {
            let mut slice = Map::new();
            for key in CLAUDE_SETTINGS_OWNED_KEYS {
                if let Some(value) = obj.get(key) {
                    slice.insert(key.to_string(), value.clone());
                }
            }
            Value::Object(slice)
        })
        .unwrap_or(Value::Null);
    crate::native_mcp::model::sha256_hex_bytes(
        serde_json::to_vec(&owned).unwrap_or_default().as_slice(),
    )
}

/// 渲染该实例配置目录下的全部受管文件（不含规则文件，见 FILE_CLAUDE_RULES）。
///
/// `existing` 给出目标文件的现有字节：Claude 的 `settings.json` 采用合并写入，
/// 用户自己加的根级字段原样保留，本工具只覆盖它管的 env/model/hooks 三项。
/// Hooks 的正文复用主实例那套来源与渲染，实例因此和主实例拿到同一份内容。
pub(crate) fn render_all(
    ctx: &Context,
    def: &ProfileDef,
    existing: &dyn Fn(&str) -> Option<Vec<u8>>,
    resolve: &dyn Fn(&str) -> Option<String>,
) -> Result<Vec<RenderedTarget>> {
    let hook_sources = crate::native_sources::current_hook_sources_readonly(ctx)?;

    let mut out = Vec::new();
    match def.kind {
        ProfileKind::Claude => {
            let managed =
                parse_hook_source(&hook_sources, crate::native_sources::KEY_HOOKS_CLAUDE)?;
            // settings.json：既有内容为基底 → 覆盖 env/model → 合入 hooks
            let base = existing(FILE_CLAUDE_SETTINGS)
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                .filter(Value::is_object)
                .unwrap_or_else(|| Value::Object(Map::new()));
            let mut settings = base.as_object().cloned().unwrap_or_default();
            settings.insert("env".into(), Value::Object(resolved_env(def, resolve)?));
            match &def.model {
                Some(model) => {
                    settings.insert("model".into(), Value::String(model.clone()));
                }
                None => {
                    settings.remove("model");
                }
            }
            let merged = crate::native_hooks::model::claude_render(
                &Value::Object(settings),
                &managed.0,
                &managed.1,
            );
            out.push(RenderedTarget {
                file: FILE_CLAUDE_SETTINGS.to_string(),
                content: crate::native_hooks::model::render_bytes(&merged),
            });
        }
        ProfileKind::Codex => {
            let managed = parse_hook_source(&hook_sources, crate::native_sources::KEY_HOOKS_CODEX)?;
            out.push(RenderedTarget {
                file: CODEX_PROFILE_FILE.to_string(),
                content: render_codex_provider(def).into_bytes(),
            });
            out.push(RenderedTarget {
                file: FILE_CODEX_HOOKS.to_string(),
                content: crate::native_hooks::model::render_bytes(
                    &crate::native_hooks::model::codex_render(&managed.0),
                ),
            });
        }
    }
    Ok(out)
}

/// 解析 hooks 来源；结构非法即 fail-closed，不回显来源正文。
fn parse_hook_source(
    sources: &std::collections::BTreeMap<String, String>,
    key: &str,
) -> Result<(
    crate::native_hooks::model::HookMap,
    Vec<crate::native_hooks::model::RemovedRule>,
)> {
    let raw = sources
        .get(key)
        .cloned()
        .unwrap_or_else(|| "{}".to_string());
    let value: Value = serde_json::from_str(&raw)
        .map_err(|_| anyhow::anyhow!("hooks 来源 {key} 不是合法 JSON 对象"))?;
    let obj = value
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("hooks 来源 {key} 不是合法 JSON 对象"))?;
    crate::native_hooks::model::validate_source(key, obj)
        .map_err(|err| anyhow::anyhow!("hooks 来源 {key} 结构非法: {err:#}"))
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

/// Codex 供应商叠加配置正文。
fn render_codex_provider(def: &ProfileDef) -> String {
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
    out
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

    #[test]
    fn codex_provider_only_writes_the_variable_name() {
        let codex = parse_definition(
            "cx-work",
            &json!({
                "profile": {"base_agent": "codex", "config_dir": "~/.codex-work",
                            "base_url": "https://api.example.com/v1", "env_key": "MY_TOKEN", "model": "gpt-5.5"}
            }),
            true,
        )
        .unwrap();
        let rendered = render_codex_provider(&codex);
        assert!(rendered.contains("env_key = \"MY_TOKEN\""));
        assert!(rendered.contains("model_provider = \"agent-manager\""));
        // 取值只留在环境变量里，永远不进这个文件
        assert!(!rendered.contains("resolved-value"));
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

#[cfg(test)]
mod owned_slice_tests {
    use super::*;
    use serde_json::json;

    fn settings(extra: Value) -> Vec<u8> {
        let mut obj = serde_json::json!({
            "env": {"A": "1"},
            "model": "claude-opus-5",
            "hooks": {}
        });
        if let (Some(dst), Some(src)) = (obj.as_object_mut(), extra.as_object()) {
            for (k, v) in src {
                dst.insert(k.clone(), v.clone());
            }
        }
        serde_json::to_vec(&obj).unwrap()
    }

    #[test]
    fn user_own_settings_do_not_change_the_owned_digest() {
        let before = owned_slice_digest(FILE_CLAUDE_SETTINGS, &settings(json!({})));
        let after = owned_slice_digest(
            FILE_CLAUDE_SETTINGS,
            &settings(
                json!({"statusLine": {"type": "command"}, "permissions": {"allow": ["Bash"]}}),
            ),
        );
        assert_eq!(before, after, "用户自己的设置不该被判成本工具那部分变了");
    }

    #[test]
    fn touching_an_owned_key_changes_the_digest() {
        let before = owned_slice_digest(FILE_CLAUDE_SETTINGS, &settings(json!({})));
        let after = owned_slice_digest(FILE_CLAUDE_SETTINGS, &settings(json!({"env": {"A": "2"}})));
        assert_ne!(before, after, "本工具拥有的键被改必须检出");
    }

    #[test]
    fn other_files_are_compared_whole() {
        // hooks.json 等整份归本工具：任何一个字节变了都算变了
        let a = owned_slice_digest(FILE_CODEX_HOOKS, b"x");
        let b = owned_slice_digest(FILE_CODEX_HOOKS, b"y");
        assert_ne!(a, b);
    }
}
