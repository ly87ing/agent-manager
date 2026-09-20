//! 助手的运行参数：接入配置（地址、认证、模型）、权限与客户端运行偏好。
//! 写进助手自己的配置文件，因此 IDE、别的终端、任何入口启动它都一样生效。
//!
//! 一个助手可以存多套**接入配置**，同一时刻一套生效——官方、各家中转站各存一套，
//! 切换只是改 `active` 再写一次文件，不用每次重填地址和令牌。
//!
//! 密钥语义与实例、MCP 一致：库里只存变量名，写目标文件时解析成真值。
//! `settings.json` 的 `env` 是字面量映射，Claude Code 不对它做变量展开，
//! 引用写进去不生效；落地文件里是明文，与 MCP 写助手配置同级，由文件权限保护。
//!
//! 受管粒度按「键路径」而不是整个顶层键：`~/.claude/settings.json` 里还住着 hooks 域
//! 和用户自己的 permissions.allow/deny、别的 env 变量，整键归属会互相覆盖。
//! 外部改过我们拥有的那几个路径就 fail-closed 拒绝覆盖。

use std::collections::BTreeMap;

use anyhow::{bail, Result};
use serde_json::{Map, Value};

use crate::native::Context;

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

/// 状态栏是**条件受管**：只有 `statusLine.command` 指向本工具部署的那份脚本时才算我们的。
/// 用户自己定义的状态栏与本工具无关，既不清空也不算外部修改。
const STATUSLINE_PATH: &[&str] = &["statusLine"];

fn statusline_is_ours(root: &Value, statusline_path: &std::path::Path) -> bool {
    path_get(root, &["statusLine", "command"])
        .and_then(Value::as_str)
        .is_some_and(|cmd| std::path::Path::new(cmd) == statusline_path)
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
}

/// 一个助手的全部运行参数。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct AgentSettings {
    /// 当前生效的接入配置名。
    pub(crate) active: Option<String>,
    pub(crate) profiles: BTreeMap<String, ConnectionProfile>,
    /// 跳过权限确认。改的是助手的安全默认值，调用方负责二次确认。
    pub(crate) yolo: bool,
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

impl AgentSettings {
    /// 当前生效的那套接入配置；没选就返回 None。
    pub(crate) fn current(&self) -> Option<&ConnectionProfile> {
        self.profiles.get(self.active.as_deref()?)
    }
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
        // 与 MCP 远程定义同一条线：只允许 https，或本机回环的 http
        let loopback = url.starts_with("http://localhost") || url.starts_with("http://127.0.0.1");
        if !url.starts_with("https://") && !loopback {
            bail!("接入配置 {id} 的地址只允许 https://，或本机回环的 http://localhost / http://127.0.0.1");
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
    Ok(ConnectionProfile {
        label: non_empty(obj.get("label")),
        base_url,
        auth: AuthKind::parse(obj.get("auth"))?,
        token_var,
        model: non_empty(obj.get("model")),
        small_fast_model: non_empty(obj.get("small_fast_model")),
        model_aliases,
        timeout_ms,
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
        yolo: obj.get("yolo").and_then(Value::as_bool).unwrap_or(false),
        statusline: StatusLine::parse(obj.get("statusline"))?,
        claude_tool_search: obj
            .get("claude_tool_search")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        claude_agent_teams: obj
            .get("claude_agent_teams")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        claude_effort_level: optional_choice(
            obj,
            "claude_effort_level",
            &["auto", "low", "medium", "high", "xhigh", "max"],
        )?,
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
            &["minimal", "low", "medium", "high", "xhigh"],
        )?,
        codex_context_window: optional_positive_u64(obj, "codex_context_window")?,
        codex_auto_compact_token_limit: optional_positive_u64(
            obj,
            "codex_auto_compact_token_limit",
        )?,
        codex_web_search: optional_choice(
            obj,
            "codex_web_search",
            &["disabled", "cached", "indexed", "live"],
        )?,
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

// ---------- 渲染 ----------

/// 渲染 Claude 的 settings.json：既有内容为基底，只改本工具拥有的那几条路径。
///
/// `resolve` 把变量名换成取值；取不到就报错而不是写一个空令牌上去——
/// 空令牌会让助手以为配好了，实际每次请求都被拒。
pub(crate) fn render_claude(
    existing: Option<&[u8]>,
    settings: &AgentSettings,
    resolve: &dyn Fn(&str) -> Option<String>,
    statusline_path: &std::path::Path,
) -> Result<Vec<u8>> {
    let base = existing
        .and_then(|bytes| serde_json::from_slice::<Value>(bytes).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| Value::Object(Map::new()));
    let mut root = base.as_object().cloned().unwrap_or_default();

    // 先把本工具拥有的每条路径清干净，再按当前生效的那套写回去：
    // 切换接入配置时，上一套留下的变量必须消失，否则两套混着生效。
    for path in CLAUDE_OWNED_PATHS {
        path_remove(&mut root, path);
    }

    if let Some(profile) = settings.current() {
        if let Some(url) = profile.base_url.as_deref() {
            path_set(
                &mut root,
                &["env", "ANTHROPIC_BASE_URL"],
                Value::String(url.to_string()),
            );
        }
        if let Some(name) = profile.token_var.as_deref() {
            let Some(value) = resolve(name) else {
                bail!("环境变量 {name} 还没有取值：先在密钥页填上，或在启动环境里 export 它");
            };
            path_set(
                &mut root,
                &["env", profile.auth.env_key()],
                Value::String(value),
            );
        }
        if let Some(model) = profile.model.as_deref() {
            path_set(
                &mut root,
                &["env", "ANTHROPIC_MODEL"],
                Value::String(model.to_string()),
            );
        }
        if let Some(model) = profile.small_fast_model.as_deref() {
            path_set(
                &mut root,
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
            path_set(&mut root, &["env", key], Value::String(target.clone()));
        }
        if let Some(ms) = profile.timeout_ms {
            // Claude Code 读的是字符串形式的环境变量
            path_set(
                &mut root,
                &["env", "API_TIMEOUT_MS"],
                Value::String(ms.to_string()),
            );
        }
    }

    if settings.yolo {
        path_set(
            &mut root,
            &["permissions", "defaultMode"],
            Value::String("bypassPermissions".into()),
        );
    }

    if settings.claude_tool_search {
        path_set(
            &mut root,
            &["env", "ENABLE_TOOL_SEARCH"],
            Value::String("true".into()),
        );
    }
    if settings.claude_agent_teams {
        path_set(
            &mut root,
            &["env", "CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS"],
            Value::String("1".into()),
        );
    }
    if let Some(level) = settings.claude_effort_level.as_deref() {
        path_set(
            &mut root,
            &["env", "CLAUDE_CODE_EFFORT_LEVEL"],
            Value::String(level.to_string()),
        );
    }
    if settings.claude_disable_artifact {
        path_set(
            &mut root,
            &["env", "CLAUDE_CODE_DISABLE_ARTIFACT"],
            Value::String("1".into()),
        );
    }
    if settings.claude_hide_attribution {
        path_set(
            &mut root,
            &["attribution", "commit"],
            Value::String(String::new()),
        );
        path_set(
            &mut root,
            &["attribution", "pr"],
            Value::String(String::new()),
        );
    }

    // 状态栏：选内置就指过去；选「不碰」时只撤掉我们自己指过去的那条，
    // 用户手配的原样留着——它本来就不归我们管。
    if settings.statusline == StatusLine::Builtin {
        let mut entry = Map::new();
        entry.insert("type".into(), Value::String("command".into()));
        entry.insert(
            "command".into(),
            Value::String(statusline_path.display().to_string()),
        );
        entry.insert("padding".into(), Value::Number(0.into()));
        path_set(&mut root, STATUSLINE_PATH, Value::Object(entry));
    } else if statusline_is_ours(&Value::Object(root.clone()), statusline_path) {
        path_remove(&mut root, STATUSLINE_PATH);
    }

    let mut out = serde_json::to_vec_pretty(&Value::Object(root))?;
    out.push(b'\n');
    Ok(out)
}

// ---------- Codex：config.toml ----------

/// Codex 的目标文件，相对 CODEX_HOME。
pub(crate) const CODEX_CONFIG: &str = "config.toml";

/// 受管标记：与 MCP 写进同一个文件的标记一致，只有带它的表段才是本工具的。
const MANAGED_MARK: &str = "# [agent-manager:managed]";

/// 本工具在 config.toml 里拥有的顶层键。
const CODEX_OWNED_KEYS: &[&str] = &[
    "model_provider",
    "model",
    "approval_policy",
    "sandbox_mode",
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

/// 删掉所有带受管标记、且表头以 `prefix.` 开头的表段，其余原样保留。
fn toml_remove_managed_tables(content: &str, prefix: &str) -> String {
    let mut out = String::new();
    let mut skip = false;
    let mut pending_mark = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == MANAGED_MARK {
            // 先攥着：下一行是我们的表头就连标记一起丢，否则原样放回
            pending_mark = true;
            continue;
        }
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            let header = trimmed.trim_matches(|c| c == '[' || c == ']');
            let ours = pending_mark && header.starts_with(&format!("{prefix}."));
            skip = ours;
            if !ours && pending_mark {
                out.push_str(MANAGED_MARK);
                out.push('\n');
            }
            pending_mark = false;
        } else if pending_mark {
            out.push_str(MANAGED_MARK);
            out.push('\n');
            pending_mark = false;
        }
        if !skip {
            out.push_str(line);
            out.push('\n');
        }
    }
    if pending_mark {
        out.push_str(MANAGED_MARK);
        out.push('\n');
    }
    out
}

/// 收集带受管标记的表段正文，用于外部修改检测。
fn toml_managed_tables(content: &str, prefix: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut current: Option<String> = None;
    let mut pending_mark = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == MANAGED_MARK {
            pending_mark = true;
            continue;
        }
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            if let Some(block) = current.take() {
                found.push(block);
            }
            let header = trimmed.trim_matches(|c| c == '[' || c == ']');
            if pending_mark && header.starts_with(&format!("{prefix}.")) {
                current = Some(format!("{trimmed}\n"));
            }
            pending_mark = false;
            continue;
        }
        pending_mark = false;
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

/// config.toml 里本工具拥有的那部分：四个顶层键 + 受管的 provider 段。
pub(crate) fn codex_owned_slice(content: &str) -> Value {
    let mut slice = Map::new();
    for key in CODEX_OWNED_KEYS {
        if let Some(value) = toml_root_key_value(content, key) {
            slice.insert((*key).to_string(), Value::String(value));
        }
    }
    let tables = toml_managed_tables(content, "model_providers");
    if !tables.is_empty() {
        slice.insert(
            "model_providers".into(),
            Value::Array(tables.into_iter().map(Value::String).collect()),
        );
    }
    Value::Object(slice)
}

pub(crate) fn codex_owned_digest(content: &str) -> String {
    crate::native_mcp::model::sha256_hex_bytes(
        serde_json::to_vec(&codex_owned_slice(content))
            .unwrap_or_default()
            .as_slice(),
    )
}

/// 渲染 Codex 的 config.toml：既有内容为基底，只改本工具拥有的那几项。
///
/// 令牌不落盘：Codex 的 provider 用 `env_key` 指向环境变量名，由它自己去读，
/// 因此这里只写变量名——与 Claude 那边被迫写真值不同。
pub(crate) fn render_codex(existing: &str, settings: &AgentSettings) -> Result<String> {
    // 先撤干净本工具写过的东西，再按当前生效的那套写回去
    let mut text = toml_remove_managed_tables(existing, "model_providers");
    for key in CODEX_OWNED_KEYS {
        text = toml_set_root_key(&text, key, None);
    }

    let active_name = settings.active.clone();
    if let (Some(name), Some(profile)) = (active_name.as_deref(), settings.current()) {
        if CODEX_RESERVED_PROVIDERS.contains(&name) {
            bail!("{name} 是 Codex 内置的接入方式，换个名字（内置的不能覆盖）");
        }
        // 只有真配了地址才值得建一个 provider；否则就是用 Codex 自带的登录
        if profile.base_url.is_some() || profile.token_var.is_some() {
            let Some(url) = profile.base_url.as_deref() else {
                bail!("接入配置 {name} 要填接口地址，Codex 的自定义接入方式没有默认地址");
            };
            let mut table = format!(
                "\n{MANAGED_MARK}\n[model_providers.{}]\nname = {}\nbase_url = {}\n",
                name,
                crate::native_mcp::codecs::toml_basic_string(
                    profile.label.as_deref().unwrap_or(name)
                ),
                crate::native_mcp::codecs::toml_basic_string(url),
            );
            if let Some(var) = profile.token_var.as_deref() {
                table.push_str(&format!(
                    "env_key = {}\n",
                    crate::native_mcp::codecs::toml_basic_string(var)
                ));
            }
            // wire_api 目前只有 responses 这一种，写死避免用户填错
            table.push_str("wire_api = \"responses\"\n");
            if let Some(ms) = profile.timeout_ms {
                table.push_str(&format!("stream_idle_timeout_ms = {ms}\n"));
            }
            if !text.ends_with('\n') && !text.is_empty() {
                text.push('\n');
            }
            text.push_str(&table);
            text = toml_set_root_key(
                &text,
                "model_provider",
                Some(&crate::native_mcp::codecs::toml_basic_string(name)),
            );
        }
        if let Some(model) = profile.model.as_deref() {
            text = toml_set_root_key(
                &text,
                "model",
                Some(&crate::native_mcp::codecs::toml_basic_string(model)),
            );
        }
    }

    if settings.yolo {
        text = toml_set_root_key(&text, "approval_policy", Some("\"never\""));
        text = toml_set_root_key(&text, "sandbox_mode", Some("\"danger-full-access\""));
    }
    if let Some(value) = settings.codex_reasoning_effort.as_deref() {
        text = toml_set_root_key(
            &text,
            "model_reasoning_effort",
            Some(&crate::native_mcp::codecs::toml_basic_string(value)),
        );
    }
    if let Some(value) = settings.codex_context_window {
        text = toml_set_root_key(&text, "model_context_window", Some(&value.to_string()));
    }
    if let Some(value) = settings.codex_auto_compact_token_limit {
        text = toml_set_root_key(
            &text,
            "model_auto_compact_token_limit",
            Some(&value.to_string()),
        );
    }
    if let Some(value) = settings.codex_web_search.as_deref() {
        text = toml_set_root_key(
            &text,
            "web_search",
            Some(&crate::native_mcp::codecs::toml_basic_string(value)),
        );
    }

    Ok(text)
}

/// Codex 的目标文件绝对路径：配置文件就在 CODEX_HOME 下。
pub(crate) fn codex_target(ctx: &Context, config_path: Option<&str>) -> std::path::PathBuf {
    match config_path {
        Some(raw) => crate::native_mcp::targets::expand_home(raw, &ctx.home_dir),
        None => ctx.home_dir.join(".codex").join(CODEX_CONFIG),
    }
}

/// 助手形态：决定写哪个配置文件、用哪套渲染。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Claude,
    Codex,
}

/// 只认可执行名：codex 形态。
fn is_codex_like(bin: &str) -> bool {
    std::path::Path::new(bin)
        .file_stem()
        .and_then(|name| name.to_str())
        == Some("codex")
}

/// 只认可执行名，路径和 .exe 后缀都剥掉——自定义助手可能把 bin 写成绝对路径。
fn is_claude_like(bin: &str) -> bool {
    std::path::Path::new(bin)
        .file_stem()
        .and_then(|name| name.to_str())
        == Some("claude")
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

// ---------- 落地 ----------

use crate::native::{DispatchError, Request};
use crate::store;
use rusqlite::{Connection, TransactionBehavior};
use serde_json::json;
use std::path::Path;

const BINDINGS_DDL: &str = "CREATE TABLE IF NOT EXISTS agent_settings_bindings (
    agent_id TEXT PRIMARY KEY NOT NULL,
    file TEXT NOT NULL,
    applied_digest TEXT NOT NULL,
    updated_at TEXT NOT NULL
)";

fn ensure_table(conn: &Connection) -> Result<()> {
    conn.execute_batch(BINDINGS_DDL)?;
    Ok(())
}

/// 上次本工具写进去的那部分的摘要；没写过就是 None。
fn applied_digest_of(ctx: &Context, agent_id: &str) -> Result<Option<String>> {
    let Some(conn) = store::open_readonly(ctx)? else {
        return Ok(None);
    };
    let exists: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='agent_settings_bindings'",
        [],
        |row| row.get(0),
    )?;
    if exists == 0 {
        return Ok(None);
    }
    let mut stmt =
        conn.prepare("SELECT applied_digest FROM agent_settings_bindings WHERE agent_id = ?1")?;
    let mut rows = stmt.query([agent_id])?;
    match rows.next()? {
        Some(row) => Ok(Some(row.get(0)?)),
        None => Ok(None),
    }
}

/// 目标文件里本工具那部分是不是被外部改过。
///
/// 没写过而文件里已经有这些键 = 用户自己配过，同样按外部内容对待：不问自取会把
/// 他手写的地址、令牌和状态栏顶掉。
pub(crate) fn external_change(
    ctx: &Context,
    agent_id: &str,
    kind: Kind,
    path: &Path,
) -> Result<bool> {
    if !path.is_file() {
        return Ok(false);
    }
    let bytes = std::fs::read(path)?;
    let (current, empty) = match kind {
        Kind::Claude => {
            let statusline_path = statusline_target(ctx);
            (
                owned_digest(&bytes, &statusline_path),
                owned_slice(&bytes, &statusline_path)
                    .as_object()
                    .is_some_and(Map::is_empty),
            )
        }
        Kind::Codex => {
            let text = String::from_utf8_lossy(&bytes);
            (
                codex_owned_digest(&text),
                codex_owned_slice(&text)
                    .as_object()
                    .is_some_and(Map::is_empty),
            )
        }
    };
    match applied_digest_of(ctx, agent_id)? {
        Some(known) => Ok(known != current),
        None => Ok(!empty),
    }
}

fn atomic_write(target: &Path, content: &[u8]) -> Result<()> {
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = target.with_extension(format!(
        "am-tmp-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    std::fs::write(&tmp, content)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // 里面可能有明文令牌，按私密文件落权限
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(&tmp, target)?;
    Ok(())
}

/// 真实写入：`agents:configure --only <id> [--use <接入配置>] [--allow-takeover] [--allow-yolo]`。
///
/// `--use` 顺手把生效的接入配置切过去再写，切换因此是一步操作：库里的 active 和
/// 目标文件要么一起变，要么都不变。
///
/// YOLO 改的是助手的安全默认值，必须由调用方显式带 `--allow-yolo`——界面在那之前
/// 弹二次确认。少了这个标志就拒绝，不靠界面自觉。
pub(crate) fn apply(ctx: &Context, req: &Request) -> Result<Value, DispatchError> {
    let id = req
        .value("--only")
        .ok_or_else(|| DispatchError::new("USAGE_ERROR", "agents:configure 需要 --only <助手>"))?
        .trim_start_matches("agents:")
        .to_string();

    let defs = crate::native_agents::load_definitions(ctx)
        .map_err(|err| DispatchError::new("AGENT_ERROR", format!("{err:#}")))?;
    let def = defs
        .iter()
        .find(|d| d.id == id)
        .ok_or_else(|| DispatchError::new("AGENT_ERROR", format!("没有这个助手：{id}")))?;
    let kind = if is_claude_like(&def.bin) {
        Kind::Claude
    } else if is_codex_like(&def.bin) {
        Kind::Codex
    } else {
        return Err(DispatchError::new(
            "AGENT_ERROR",
            "只有 Claude Code 与 Codex 形态的助手支持写运行参数".to_string(),
        ));
    };

    let mut settings = parse(def.spec_ref())
        .map_err(|err| DispatchError::new("AGENT_ERROR", format!("{err:#}")))?;

    // 切换生效的接入配置：--use <名字> 切过去，--use-default 切回助手自带的账号与地址。
    // 指向不存在的配置一律拒绝，不静默落到「没有生效配置」。
    let use_default = req.flag("--use-default");
    let switch_to = req.value("--use").map(str::to_string);
    if use_default && switch_to.is_some() {
        return Err(DispatchError::new(
            "USAGE_ERROR",
            "--use 与 --use-default 不能同时给".to_string(),
        ));
    }
    let switched = use_default || switch_to.is_some();
    if use_default {
        settings.active = None;
    } else if let Some(name) = switch_to.as_deref() {
        if !settings.profiles.contains_key(name) {
            return Err(DispatchError::new(
                "AGENT_ERROR",
                format!("{id} 没有名为 {name} 的接入配置"),
            ));
        }
        settings.active = Some(name.to_string());
    }

    if settings.yolo && !req.flag("--allow-yolo") {
        return Err(DispatchError::new(
            "AGENT_ERROR",
            "跳过权限确认会改掉助手的安全默认值，需要显式确认后再执行".to_string(),
        ));
    }

    let path = match kind {
        Kind::Claude => claude_target(ctx),
        Kind::Codex => codex_target(ctx, def.config_path()),
    };
    let file_label = match kind {
        Kind::Claude => CLAUDE_SETTINGS,
        Kind::Codex => CODEX_CONFIG,
    };
    let takeover = external_change(ctx, &id, kind, &path)
        .map_err(|err| DispatchError::new("AGENT_ERROR", format!("{err:#}")))?;
    if takeover && !req.flag("--allow-takeover") {
        return Err(DispatchError::new(
            "AGENT_ERROR",
            format!(
                "{file_label} 里这几项被本工具以外改过，拒绝覆盖；确认要用这里的内容替换请选择接管"
            ),
        ));
    }

    let statusline_path = statusline_target(ctx);
    // 脚本先就位再写 settings.json：反过来的话中途失败会留下一个指向不存在文件的状态栏
    if kind == Kind::Claude && settings.statusline == StatusLine::Builtin {
        deploy_statusline(&statusline_path)
            .map_err(|err| DispatchError::new("AGENT_ERROR", format!("{err:#}")))?;
    }

    let existing = std::fs::read(&path).ok();
    let content = match kind {
        Kind::Claude => render_claude(
            existing.as_deref(),
            &settings,
            &crate::secret_values::lookup,
            &statusline_path,
        ),
        Kind::Codex => {
            let text = existing
                .as_deref()
                .map(String::from_utf8_lossy)
                .unwrap_or_default()
                .to_string();
            render_codex(&text, &settings).map(String::into_bytes)
        }
    }
    .map_err(|err| DispatchError::new("AGENT_ERROR", format!("{err:#}")))?;

    atomic_write(&path, &content)
        .map_err(|err| DispatchError::new("AGENT_ERROR", format!("{err:#}")))?;

    let digest = match kind {
        Kind::Claude => owned_digest(&content, &statusline_path),
        Kind::Codex => codex_owned_digest(&String::from_utf8_lossy(&content)),
    };
    let active = settings.active.clone();
    let persisted = (|| -> Result<()> {
        let mut conn = store::open_write(ctx)?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        ensure_table(&tx)?;
        tx.execute(
            "INSERT INTO agent_settings_bindings (agent_id, file, applied_digest, updated_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(agent_id) DO UPDATE SET file=excluded.file,
                 applied_digest=excluded.applied_digest, updated_at=excluded.updated_at",
            rusqlite::params![
                &id,
                file_label,
                &digest,
                chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
            ],
        )?;
        // --use 切换要落库，否则下次读定义又切回去了
        if switched {
            let mut policy = store::policy_from_connection(&tx)?;
            let target = policy
                .get_mut("agents")
                .and_then(|v| v.get_mut("definitions"))
                .and_then(|v| v.get_mut(&id));
            if let Some(definition) = target {
                if let Some(obj) = definition.as_object_mut() {
                    let entry = obj
                        .entry("settings".to_string())
                        .or_insert_with(|| Value::Object(Map::new()));
                    if let Some(settings_obj) = entry.as_object_mut() {
                        match &active {
                            Some(name) => {
                                settings_obj.insert("active".into(), Value::String(name.clone()));
                            }
                            None => {
                                settings_obj.remove("active");
                            }
                        }
                    }
                }
                store::write_policy(&tx, &policy)?;
            }
        }
        // 审计只记改了哪几项，不记取值本身
        store::record_operation(
            &tx,
            Some(&req.run_id),
            "agents:configure",
            Some(&id),
            Some(&json!({
                "file": file_label,
                "takeover": takeover,
                "active": active,
                "profiles": settings.profiles.len(),
                "token_var": settings.current().and_then(|p| p.token_var.clone()),
                "yolo": settings.yolo,
                "statusline": kind == Kind::Claude && settings.statusline == StatusLine::Builtin,
            })),
        )?;
        tx.commit()?;
        Ok(())
    })();
    if let Err(err) = persisted {
        // 文件已经写出去了，库没记上：下次会按「外部修改」拦住，不会默默覆盖
        return Err(DispatchError::new("AGENT_ERROR", format!("{err:#}")));
    }

    Ok(json!({
        "domain": "agents",
        "action": "configure",
        "element": format!("agents:{id}"),
        "target": path.display().to_string(),
        "active": settings.active,
        "takeover": takeover,
        "statusline": kind == Kind::Claude && settings.statusline == StatusLine::Builtin,
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

    fn one_profile(entry: Value) -> AgentSettings {
        parse(&json!({"settings": {"active": "p", "profiles": {"p": entry}}})).unwrap()
    }

    #[test]
    fn 解析拒绝非法地址_变量名_配置名与认证方式() {
        let bad_url =
            json!({"settings": {"profiles": {"p": {"base_url": "http://evil.example.com"}}}});
        assert!(parse(&bad_url).is_err());
        assert!(parse(
            &json!({"settings": {"profiles": {"p": {"base_url": "http://localhost:8080"}}}})
        )
        .is_ok());
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
        let settings = one_profile(json!({
            "base_url": "https://api.example.com",
            "model": "claude-opus-5"
        }));
        let out = render_claude(Some(&existing), &settings, &no_secrets, &sl()).unwrap();
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
        let settings = parse(&json!({"settings": {
            "claude_tool_search": true,
            "claude_agent_teams": true,
            "claude_effort_level": "max",
            "claude_disable_artifact": true,
            "claude_hide_attribution": true
        }}))
        .unwrap();
        let existing = serde_json::to_vec(&json!({
            "env": {"MY_OWN": "keep"},
            "attribution": {"other": "keep"}
        }))
        .unwrap();
        let out = render_claude(Some(&existing), &settings, &no_secrets, &sl()).unwrap();
        let parsed: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(parsed["env"]["ENABLE_TOOL_SEARCH"], "true");
        assert_eq!(parsed["env"]["CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS"], "1");
        assert_eq!(parsed["env"]["CLAUDE_CODE_EFFORT_LEVEL"], "max");
        assert_eq!(parsed["env"]["CLAUDE_CODE_DISABLE_ARTIFACT"], "1");
        assert_eq!(parsed["attribution"]["commit"], "");
        assert_eq!(parsed["attribution"]["pr"], "");

        let cleared =
            render_claude(Some(&out), &AgentSettings::default(), &no_secrets, &sl()).unwrap();
        let parsed: Value = serde_json::from_slice(&cleared).unwrap();
        assert_eq!(parsed["env"]["MY_OWN"], "keep");
        assert_eq!(parsed["attribution"]["other"], "keep");
        assert!(parsed["env"].get("ENABLE_TOOL_SEARCH").is_none());
        assert!(parsed["attribution"].get("commit").is_none());
    }

    #[test]
    fn 两种认证方式写不同的环境变量() {
        let resolve = |name: &str| (name == "TOK").then(|| "secret".to_string());
        let bearer = one_profile(json!({"token_var": "TOK"}));
        let parsed: Value =
            serde_json::from_slice(&render_claude(None, &bearer, &resolve, &sl()).unwrap())
                .unwrap();
        assert_eq!(parsed["env"]["ANTHROPIC_AUTH_TOKEN"], "secret");
        assert!(parsed["env"].get("ANTHROPIC_API_KEY").is_none());

        let api_key = one_profile(json!({"token_var": "TOK", "auth": "api_key"}));
        let parsed: Value =
            serde_json::from_slice(&render_claude(None, &api_key, &resolve, &sl()).unwrap())
                .unwrap();
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
        let first = render_claude(None, &settings, &no_secrets, &sl()).unwrap();
        let parsed: Value = serde_json::from_slice(&first).unwrap();
        assert_eq!(
            parsed["env"]["ANTHROPIC_BASE_URL"],
            "https://relay.example.com"
        );
        assert_eq!(parsed["env"]["ANTHROPIC_SMALL_FAST_MODEL"], "relay-small");
        assert_eq!(parsed["env"]["ANTHROPIC_DEFAULT_OPUS_MODEL"], "relay-opus");
        assert_eq!(parsed["env"]["API_TIMEOUT_MS"], "600000");

        // 切到什么都没配的 official：上一套留下的变量一个都不许剩
        let mut switched = settings.clone();
        switched.active = Some("official".into());
        let second = render_claude(Some(&first), &switched, &no_secrets, &sl()).unwrap();
        let parsed: Value = serde_json::from_slice(&second).unwrap();
        assert!(parsed.get("env").is_none(), "官方配置不需要任何环境变量");
    }

    #[test]
    fn 清空某项只删自己那条_不碰同级也不留空壳() {
        let existing = serde_json::to_vec(&json!({
            "env": {"ANTHROPIC_BASE_URL": "https://old.example.com", "MY_OWN": "keep-me"},
            "permissions": {"defaultMode": "bypassPermissions", "allow": ["Bash(ls:*)"]}
        }))
        .unwrap();
        let out = render_claude(
            Some(&existing),
            &AgentSettings::default(),
            &no_secrets,
            &sl(),
        )
        .unwrap();
        let parsed: Value = serde_json::from_slice(&out).unwrap();

        assert!(parsed["env"].get("ANTHROPIC_BASE_URL").is_none());
        assert_eq!(parsed["env"]["MY_OWN"], "keep-me", "同级的别人的变量要留着");
        assert!(parsed["permissions"].get("defaultMode").is_none());
        assert_eq!(
            parsed["permissions"]["allow"][0], "Bash(ls:*)",
            "同级的 allow 要留着"
        );

        // 只剩我们那一条时整个父对象收走
        let only_ours =
            serde_json::to_vec(&json!({"env": {"ANTHROPIC_BASE_URL": "https://x"}})).unwrap();
        let out = render_claude(
            Some(&only_ours),
            &AgentSettings::default(),
            &no_secrets,
            &sl(),
        )
        .unwrap();
        let parsed: Value = serde_json::from_slice(&out).unwrap();
        assert!(parsed.get("env").is_none());
    }

    #[test]
    fn 令牌取不到值时拒绝写入_而不是写个空的上去() {
        let settings = one_profile(json!({"token_var": "MISSING_TOKEN"}));
        let err = render_claude(None, &settings, &no_secrets, &sl()).unwrap_err();
        assert!(err.to_string().contains("MISSING_TOKEN"));
    }

    #[test]
    fn 选了内置状态栏就指向本工具部署的那份_不选则不碰() {
        let builtin = parse(&json!({"settings": {"statusline": "builtin"}})).unwrap();
        let out = render_claude(None, &builtin, &no_secrets, &sl()).unwrap();
        let parsed: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(parsed["statusLine"]["type"], "command");
        assert_eq!(parsed["statusLine"]["command"], "/data/statusline.sh");

        // 从内置切回「不碰」：把我们指过去的那条撤掉，用户之后自己配什么是他的事
        let back =
            render_claude(Some(&out), &AgentSettings::default(), &no_secrets, &sl()).unwrap();
        let parsed: Value = serde_json::from_slice(&back).unwrap();
        assert!(parsed.get("statusLine").is_none());
    }

    #[test]
    fn 用户自己的状态栏不归我们管_不清空也不算冲突() {
        let mine = serde_json::to_vec(&json!({
            "statusLine": {"type": "command", "command": "/x/mine.sh"},
            "env": {"ANTHROPIC_BASE_URL": "https://a"}
        }))
        .unwrap();
        // 「不碰」模式下渲染：用户那条原样留着
        let out =
            render_claude(Some(&mine), &AgentSettings::default(), &no_secrets, &sl()).unwrap();
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
        let settings = parse(&json!({"settings": {
            "codex_reasoning_effort": "high",
            "codex_context_window": 1000000,
            "codex_auto_compact_token_limit": 900000,
            "codex_web_search": "live"
        }}))
        .unwrap();
        let out = render_codex("user_instructions = \"keep\"\n", &settings).unwrap();
        assert!(out.contains("model_reasoning_effort = \"high\""));
        assert!(out.contains("model_context_window = 1000000"));
        assert!(out.contains("model_auto_compact_token_limit = 900000"));
        assert!(out.contains("web_search = \"live\""));
        assert!(out.contains("user_instructions = \"keep\""));
        let slice = codex_owned_slice(&out);
        assert_eq!(slice["model_context_window"], "1000000");

        let cleared = render_codex(&out, &AgentSettings::default()).unwrap();
        assert!(!cleared.contains("model_reasoning_effort"));
        assert!(!cleared.contains("model_context_window"));
        assert!(cleared.contains("user_instructions = \"keep\""));
    }

    #[test]
    fn 内置状态栏脚本自身可用() {
        // 随脚本一起编译进来，跑一遍语法检查，避免改坏了到运行时才发现
        assert!(STATUSLINE_BODY.starts_with("#!/bin/bash"));
        assert!(
            STATUSLINE_BODY.contains("agent-manager-usage.json"),
            "落盘是它存在的理由"
        );
        assert!(
            STATUSLINE_BODY.contains("CLAUDE_CONFIG_DIR"),
            "实例要各存各的"
        );
    }
}
