//! 助手的运行参数：上游地址、令牌、模型、YOLO。写进助手自己的配置文件，
//! 因此 IDE、别的终端、任何入口启动它都一样生效。
//!
//! 与实例（profiles）同一套密钥语义：库里只存 `${VAR}` 引用，写目标文件时解析成
//! 真值——`settings.json` 的 `env` 是字面量映射，Claude Code 不对它做变量展开，
//! 所以引用写进去不会生效。落地文件里是明文，与 MCP 写助手配置同级，由文件权限保护。
//!
//! 受管粒度按「键路径」而不是整个顶层键：`~/.claude/settings.json` 里还住着 hooks 域
//! 和用户自己的 permissions.allow/deny、别的 env 变量，整键归属会互相覆盖。
//! 外部改过我们拥有的那几个路径就 fail-closed 拒绝覆盖。

use anyhow::{bail, Result};
use serde_json::{Map, Value};

use crate::native::Context;

/// Claude 的目标文件，相对 home。
pub(crate) const CLAUDE_SETTINGS: &str = ".claude/settings.json";

/// 本工具在 settings.json 里拥有的键路径。其余一律是别人的，原样保留。
const CLAUDE_OWNED_PATHS: [&[&str]; 4] = [
    &["env", "ANTHROPIC_BASE_URL"],
    &["env", "ANTHROPIC_AUTH_TOKEN"],
    &["model"],
    &["permissions", "defaultMode"],
];

/// 一个助手的运行参数。字段全部可选：没填的项不写进目标文件，也不从里面删别人的。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct AgentSettings {
    /// 上游接口地址，留空表示用助手的默认地址。
    pub(crate) base_url: Option<String>,
    /// 鉴权取值所在的环境变量名。库里只存这个名字，取值另行解析。
    pub(crate) token_var: Option<String>,
    pub(crate) model: Option<String>,
    /// 跳过权限确认。改的是助手的安全默认值，调用方负责二次确认。
    pub(crate) yolo: bool,
}

impl AgentSettings {
    /// 一项都没填 = 不接管这个助手的配置文件。
    pub(crate) fn is_empty(&self) -> bool {
        self.base_url.is_none() && self.token_var.is_none() && self.model.is_none() && !self.yolo
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

/// 变量名规则与 `${VAR}` 占位符可用的形状一致，和 secrets 域同一把尺子。
fn valid_var_name(name: &str) -> bool {
    crate::secret_values::valid_var_name(name)
}

/// 从助手定义里读运行参数。缺 `settings` 段就是没配过。
pub(crate) fn parse(spec: &Value) -> Result<AgentSettings> {
    let Some(raw) = spec.get("settings") else {
        return Ok(AgentSettings::default());
    };
    let Some(obj) = raw.as_object() else {
        bail!("settings 必须是对象");
    };
    let base_url = non_empty(obj.get("base_url"));
    if let Some(url) = base_url.as_deref() {
        // 与 MCP 远程定义同一条线：只允许 https，或本机回环的 http
        let loopback = url.starts_with("http://localhost") || url.starts_with("http://127.0.0.1");
        if !url.starts_with("https://") && !loopback {
            bail!("settings.base_url 只允许 https://，或本机回环的 http://localhost / http://127.0.0.1");
        }
    }
    let token_var = non_empty(obj.get("token_var"));
    if let Some(name) = token_var.as_deref() {
        if !valid_var_name(name) {
            bail!("settings.token_var 不是合法的环境变量名：要以字母或下划线开头，只能用字母、数字与下划线");
        }
    }
    Ok(AgentSettings {
        base_url,
        token_var,
        model: non_empty(obj.get("model")),
        yolo: obj.get("yolo").and_then(Value::as_bool).unwrap_or(false),
    })
}

/// 取某条路径上的值。
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
pub(crate) fn owned_slice(bytes: &[u8]) -> Value {
    let root = serde_json::from_slice::<Value>(bytes).unwrap_or(Value::Null);
    let mut slice = Map::new();
    for path in CLAUDE_OWNED_PATHS {
        if let Some(found) = path_get(&root, path) {
            path_set(&mut slice, path, found.clone());
        }
    }
    Value::Object(slice)
}

pub(crate) fn owned_digest(bytes: &[u8]) -> String {
    crate::native_mcp::model::sha256_hex_bytes(
        serde_json::to_vec(&owned_slice(bytes)).unwrap_or_default().as_slice(),
    )
}

/// 渲染 Claude 的 settings.json：既有内容为基底，只改本工具拥有的那几条路径。
///
/// `resolve` 把变量名换成取值；取不到就报错而不是写一个空令牌上去——
/// 空令牌会让助手以为配好了，实际每次请求都被拒。
pub(crate) fn render_claude(
    existing: Option<&[u8]>,
    settings: &AgentSettings,
    resolve: &dyn Fn(&str) -> Option<String>,
) -> Result<Vec<u8>> {
    let base = existing
        .and_then(|bytes| serde_json::from_slice::<Value>(bytes).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| Value::Object(Map::new()));
    let mut root = base.as_object().cloned().unwrap_or_default();

    match settings.base_url.as_deref() {
        Some(url) => path_set(&mut root, &["env", "ANTHROPIC_BASE_URL"], Value::String(url.to_string())),
        None => path_remove(&mut root, &["env", "ANTHROPIC_BASE_URL"]),
    }
    match settings.token_var.as_deref() {
        Some(name) => {
            let Some(value) = resolve(name) else {
                bail!("环境变量 {name} 还没有取值：先在密钥页填上，或在启动环境里 export 它");
            };
            path_set(&mut root, &["env", "ANTHROPIC_AUTH_TOKEN"], Value::String(value));
        }
        None => path_remove(&mut root, &["env", "ANTHROPIC_AUTH_TOKEN"]),
    }
    match settings.model.as_deref() {
        Some(model) => path_set(&mut root, &["model"], Value::String(model.to_string())),
        None => path_remove(&mut root, &["model"]),
    }
    if settings.yolo {
        path_set(
            &mut root,
            &["permissions", "defaultMode"],
            Value::String("bypassPermissions".into()),
        );
    } else {
        path_remove(&mut root, &["permissions", "defaultMode"]);
    }

    let mut out = serde_json::to_vec_pretty(&Value::Object(root))?;
    out.push(b'\n');
    Ok(out)
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn no_secrets(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn 解析拒绝非法地址与变量名() {
        assert!(parse(&json!({"settings": {"base_url": "http://evil.example.com"}})).is_err());
        assert!(parse(&json!({"settings": {"base_url": "http://localhost:8080"}})).is_ok());
        assert!(parse(&json!({"settings": {"base_url": "https://api.example.com"}})).is_ok());
        assert!(parse(&json!({"settings": {"token_var": "has-dash"}})).is_err());
        assert!(parse(&json!({"settings": {"token_var": "MY_TOKEN"}})).is_ok());
        // 没有 settings 段 = 没配过，不是错误
        assert!(parse(&json!({"bin": "claude"})).unwrap().is_empty());
    }

    #[test]
    fn 只改自己那几条路径_别人的原样保留() {
        let existing = serde_json::to_vec(&json!({
            "hooks": {"SessionStart": [{"hooks": [{"type": "command", "command": "x"}]}]},
            "statusLine": {"type": "command", "command": "/x/statusline.sh"},
            "env": {"MY_OWN": "keep-me"},
            "permissions": {"allow": ["Bash(ls:*)"]}
        }))
        .unwrap();
        let settings = AgentSettings {
            base_url: Some("https://api.example.com".into()),
            token_var: None,
            model: Some("claude-opus-5".into()),
            yolo: true,
        };
        let out = render_claude(Some(&existing), &settings, &no_secrets).unwrap();
        let parsed: Value = serde_json::from_slice(&out).unwrap();

        // 别人的东西一个都没动
        assert!(parsed["hooks"]["SessionStart"].is_array());
        assert_eq!(parsed["statusLine"]["command"], "/x/statusline.sh");
        assert_eq!(parsed["env"]["MY_OWN"], "keep-me");
        assert_eq!(parsed["permissions"]["allow"][0], "Bash(ls:*)");
        // 自己的写上了
        assert_eq!(parsed["env"]["ANTHROPIC_BASE_URL"], "https://api.example.com");
        assert_eq!(parsed["model"], "claude-opus-5");
        assert_eq!(parsed["permissions"]["defaultMode"], "bypassPermissions");
    }

    #[test]
    fn 清空某项只删自己那条_不留空壳也不碰同级() {
        let existing = serde_json::to_vec(&json!({
            "env": {"ANTHROPIC_BASE_URL": "https://old.example.com", "MY_OWN": "keep-me"},
            "model": "old-model",
            "permissions": {"defaultMode": "bypassPermissions", "allow": ["Bash(ls:*)"]}
        }))
        .unwrap();
        let out = render_claude(Some(&existing), &AgentSettings::default(), &no_secrets).unwrap();
        let parsed: Value = serde_json::from_slice(&out).unwrap();

        assert!(parsed["env"].get("ANTHROPIC_BASE_URL").is_none());
        assert_eq!(parsed["env"]["MY_OWN"], "keep-me", "同级的别人的变量要留着");
        assert!(parsed.get("model").is_none());
        assert!(parsed["permissions"].get("defaultMode").is_none());
        assert_eq!(parsed["permissions"]["allow"][0], "Bash(ls:*)", "同级的 allow 要留着");
    }

    #[test]
    fn 父对象空了才收走空壳() {
        let existing = serde_json::to_vec(&json!({
            "env": {"ANTHROPIC_BASE_URL": "https://old.example.com"},
            "permissions": {"defaultMode": "bypassPermissions"}
        }))
        .unwrap();
        let out = render_claude(Some(&existing), &AgentSettings::default(), &no_secrets).unwrap();
        let parsed: Value = serde_json::from_slice(&out).unwrap();
        assert!(parsed.get("env").is_none(), "只剩我们那一条时整个 env 收走");
        assert!(parsed.get("permissions").is_none());
    }

    #[test]
    fn 令牌取不到值时拒绝写入_而不是写个空的上去() {
        let settings = AgentSettings {
            token_var: Some("MISSING_TOKEN".into()),
            ..Default::default()
        };
        let err = render_claude(None, &settings, &no_secrets).unwrap_err();
        assert!(err.to_string().contains("MISSING_TOKEN"));

        let resolved = render_claude(None, &settings, &|name| {
            (name == "MISSING_TOKEN").then(|| "real-value".to_string())
        })
        .unwrap();
        let parsed: Value = serde_json::from_slice(&resolved).unwrap();
        // settings.json 的 env 不做变量展开，只能落真值
        assert_eq!(parsed["env"]["ANTHROPIC_AUTH_TOKEN"], "real-value");
    }

    #[test]
    fn 外部修改检测只看自己那几条路径() {
        let ours = serde_json::to_vec(&json!({"env": {"ANTHROPIC_BASE_URL": "https://a"}})).unwrap();
        let plus_theirs = serde_json::to_vec(&json!({
            "env": {"ANTHROPIC_BASE_URL": "https://a", "MY_OWN": "x"},
            "statusLine": {"command": "y"}
        }))
        .unwrap();
        assert_eq!(
            owned_digest(&ours),
            owned_digest(&plus_theirs),
            "用户加自己的设置不算外部修改"
        );

        let changed = serde_json::to_vec(&json!({"env": {"ANTHROPIC_BASE_URL": "https://b"}})).unwrap();
        assert_ne!(owned_digest(&ours), owned_digest(&changed), "改了我们的值要认出来");
    }
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
/// 他手写的地址和令牌顶掉。
pub(crate) fn external_change(ctx: &Context, agent_id: &str, path: &Path) -> Result<bool> {
    if !path.is_file() {
        return Ok(false);
    }
    let bytes = std::fs::read(path)?;
    let current = owned_digest(&bytes);
    match applied_digest_of(ctx, agent_id)? {
        Some(known) => Ok(known != current),
        None => Ok(!owned_slice(&bytes)
            .as_object()
            .is_some_and(Map::is_empty)),
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

/// 真实写入：`agents:configure --only <id> [--allow-takeover] [--allow-yolo]`。
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
    if !is_claude_like(&def.bin) {
        return Err(DispatchError::new(
            "AGENT_ERROR",
            "目前只有 Claude Code 形态的助手支持写运行参数".to_string(),
        ));
    }

    let settings = parse(def.spec_ref())
        .map_err(|err| DispatchError::new("AGENT_ERROR", format!("{err:#}")))?;
    if settings.yolo && !req.flag("--allow-yolo") {
        return Err(DispatchError::new(
            "AGENT_ERROR",
            "跳过权限确认会改掉助手的安全默认值，需要显式确认后再执行".to_string(),
        ));
    }

    let path = claude_target(ctx);
    let takeover = external_change(ctx, &id, &path)
        .map_err(|err| DispatchError::new("AGENT_ERROR", format!("{err:#}")))?;
    if takeover && !req.flag("--allow-takeover") {
        return Err(DispatchError::new(
            "AGENT_ERROR",
            format!(
                "{} 里这几项被本工具以外改过，拒绝覆盖；确认要用这里的内容替换请选择接管",
                CLAUDE_SETTINGS
            ),
        ));
    }

    let existing = std::fs::read(&path).ok();
    let content = render_claude(existing.as_deref(), &settings, &crate::secret_values::lookup)
        .map_err(|err| DispatchError::new("AGENT_ERROR", format!("{err:#}")))?;

    atomic_write(&path, &content)
        .map_err(|err| DispatchError::new("AGENT_ERROR", format!("{err:#}")))?;

    let digest = owned_digest(&content);
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
                CLAUDE_SETTINGS,
                &digest,
                chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
            ],
        )?;
        // 审计只记改了哪几项，不记取值本身
        store::record_operation(
            &tx,
            Some(&req.run_id),
            "agents:configure",
            Some(&id),
            Some(&json!({
                "file": CLAUDE_SETTINGS,
                "takeover": takeover,
                "base_url": settings.base_url.is_some(),
                "token_var": settings.token_var.as_deref(),
                "model": settings.model.as_deref(),
                "yolo": settings.yolo,
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
        "takeover": takeover,
    }))
}
