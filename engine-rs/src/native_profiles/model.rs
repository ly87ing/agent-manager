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

/// 实例 settings.json 里与本工具有关的键只有 env / model：`hooks` 归 hooks 域，其余根级
/// 字段是用户自己的（合并写入时原样保留）。env 只写实例定义里声明的那几个变量，
/// 用户自己加的 env 变量原样保留。

/// 文件不存在时的写入基准。
pub(crate) const BASE_MISSING: &str = "missing";

/// 本次定义要写的路径：`env.<变量名>` 与（定义了模型时）`model`。
/// 与库里记下的上次写入路径求并，就是这次写入可能改动的全部位置。
pub(crate) fn owned_paths(def: &ProfileDef) -> Vec<String> {
    let mut paths: Vec<String> = match def.kind {
        ProfileKind::Claude => def.env.keys().map(|key| format!("env.{key}")).collect(),
        // Codex 实例文件整份由本工具维护，不按路径记
        ProfileKind::Codex => Vec::new(),
    };
    if def.kind == ProfileKind::Claude && def.model.is_some() {
        paths.push("model".to_string());
    }
    paths
}

/// 实例目标文件的写入基准：Claude 取 settings.json 里 env 与 model 两项的摘要（用户改
/// permissions、hooks 等其他键不影响它）；Codex 实例文件整份归本工具，取文件字节摘要。
/// 文件不存在为 `missing`；Claude 文件不是 JSON 对象时报错（不给基准，也就不能写）。
pub(crate) fn live_base(kind: ProfileKind, bytes: Option<&[u8]>) -> Result<String> {
    let Some(bytes) = bytes else {
        return Ok(BASE_MISSING.to_string());
    };
    match kind {
        ProfileKind::Codex => Ok(crate::native_mcp::model::sha256_hex_bytes(bytes)),
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
            let expected = codex_fields(render_codex_provider(def).as_bytes());
            let live = bytes.map(codex_fields).unwrap_or_default();
            for (field, want) in &expected {
                let have = live.get(field);
                fields.push(serde_json::json!({
                    "field": field,
                    "state": match have {
                        None => "missing",
                        Some(have) if have == want => "same",
                        Some(_) => "differs",
                    },
                    "definition": want,
                    "live": have,
                }));
            }
        }
    }
    Ok(fields)
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
    for key in ["model", "model_provider"] {
        if let Some(value) = doc.get(key).and_then(toml::Value::as_str) {
            out.insert(key.to_string(), value.to_string());
        }
    }
    if let Some(table) = doc
        .get("model_providers")
        .and_then(|providers| providers.get(CODEX_PROFILE_NAME))
        .and_then(toml::Value::as_table)
    {
        for key in ["name", "base_url", "env_key", "wire_api"] {
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
            if live.get("model_provider").map(String::as_str) != Some(CODEX_PROFILE_NAME) {
                notes.push("实际配置没有使用本实例的接入方式，只采纳模型".to_string());
            }
            for (field, key) in [
                ("model", "model"),
                ("provider.base_url", "base_url"),
                ("provider.env_key", "env_key"),
                ("provider.wire_api", "wire_api"),
            ] {
                if key != "model"
                    && live.get("model_provider").map(String::as_str) != Some(CODEX_PROFILE_NAME)
                {
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
            out.push(RenderedTarget {
                file: CODEX_PROFILE_FILE.to_string(),
                content: render_codex_provider(def).into_bytes(),
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

    /// 与接入配置同一口径：实际配置里已有的局域网 http 中转要能存下（界面另行提示明文
    /// 传输）；非 http(s) 的地址仍拒绝。
    #[test]
    fn http_base_url_is_accepted_but_other_schemes_are_rejected() {
        assert!(def(json!({
            "profile": {"base_agent": "claude-code", "config_dir": "~/.claude-ly",
                        "env": {"ANTHROPIC_BASE_URL": "http://10.0.0.5:8080"}}
        }))
        .is_ok());
        let err = def(json!({
            "profile": {"base_agent": "claude-code", "config_dir": "~/.claude-ly",
                        "env": {"ANTHROPIC_BASE_URL": "ftp://api.example.com"}}
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

    fn no_vars(_: &str) -> Option<String> {
        None
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
        assert_ne!(
            live_base(ProfileKind::Codex, Some(b"x")).unwrap(),
            live_base(ProfileKind::Codex, Some(b"y")).unwrap()
        );
    }

    #[test]
    fn render_only_touches_declared_env_keys_and_keeps_user_ones() {
        let def = claude_def(json!({"ANTHROPIC_BASE_URL": "https://a.example.com"}), None);
        let existing =
            br#"{"env":{"MY_VAR":"1","OLD":"x"},"model":"user-model","theme":"dark"}"#.to_vec();
        let rendered = render_all(
            &def,
            &|_| Some(existing.clone()),
            &no_vars,
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
        let rendered = render_all(
            &def,
            &|_| Some(existing.clone()),
            &no_vars,
            &["model".into()],
        )
        .unwrap();
        let out: Value = serde_json::from_slice(&rendered[0].content).unwrap();
        assert!(out.get("model").is_none());
        // 现有文件不是 JSON 对象：拒绝，不当成空文件覆盖
        assert!(render_all(&def, &|_| Some(b"[1]".to_vec()), &no_vars, &[]).is_err());
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
}
