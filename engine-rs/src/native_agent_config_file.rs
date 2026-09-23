//! 助手配置文件原文编辑：`agents:config-file` 读、`agents:edit-config` 写。
//!
//! 表单只覆盖少量常用运行参数；其余设置（权限规则、插件、`modelSettings`、自定义 env……）
//! 直接编辑实际配置文件全文——Claude 的 `~/.claude/settings.json`、Codex 的 `config.toml`。
//! 不在本工具里再实现一遍官方 schema，也不另存副本。
//!
//! 凭据不经过界面：读取时把像凭据的取值（变量名像凭据，或取值本身像令牌）替换为
//! `<hidden>`；写回时 `<hidden>` 只能留在原来的位置，按原路径还原成原值，挪动或改名一律
//! 拒绝。写入基准是整个文件的摘要：打开后文件在别处被改过（包括 Claude Code 自己改）就
//! 拒绝，不覆盖。JSON 按两空格重新排版；TOML 原位修改，注释与格式保留。审计只记改动了
//! 哪些顶层键，不记取值。

use anyhow::{anyhow, bail, Result};
use serde_json::{json, Map, Value};

use crate::native::{Context, DispatchError, Request};
use crate::native_agent_settings::Kind;

/// 界面上代替凭据取值的占位符。
pub(crate) const HIDDEN: &str = "<hidden>";

/// 文件不存在时的写入基准。
const BASE_MISSING: &str = "missing";

/// 取值路径上的一段。
#[derive(Debug, Clone, PartialEq, Eq)]
enum Seg {
    Key(String),
    Index(usize),
}

type Path = Vec<Seg>;

/// 变量名样式（`OPENAI_API_KEY` 这类只写变量名的取值不是凭据）。
fn looks_like_var_name(value: &str) -> bool {
    let mut chars = value.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_uppercase() || c == '_')
        && chars.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

/// 该不该遮住：取值像令牌，或所在键名像凭据且取值不是变量名、`${VAR}` 引用或文件路径。
fn should_hide(key: Option<&str>, value: &str) -> bool {
    if value.is_empty() || value == HIDDEN {
        return false;
    }
    if crate::native::reject_secret_text(value).is_some() {
        return true;
    }
    let Some(key) = key else { return false };
    if !crate::native::credential_like_name(key) {
        return false;
    }
    let reference = value.starts_with("${") && value.ends_with('}');
    let path_like = value.starts_with('/') || value.starts_with("~/") || value.starts_with("./");
    !(looks_like_var_name(value) || reference || path_like)
}

fn sha256(bytes: &[u8]) -> String {
    crate::native_mcp::model::sha256_hex_bytes(bytes)
}

// ---------- JSON（Claude settings.json） ----------

fn mask_json(
    value: &mut Value,
    path: &mut Path,
    key: Option<&str>,
    hidden: &mut Vec<(Path, Value)>,
) {
    match value {
        Value::String(text) => {
            if should_hide(key, text) {
                hidden.push((path.clone(), Value::String(std::mem::take(text))));
                *text = HIDDEN.to_string();
            }
        }
        Value::Array(items) => {
            for (index, item) in items.iter_mut().enumerate() {
                path.push(Seg::Index(index));
                mask_json(item, path, key, hidden);
                path.pop();
            }
        }
        Value::Object(map) => {
            for (name, item) in map.iter_mut() {
                path.push(Seg::Key(name.clone()));
                mask_json(item, path, Some(name.as_str()), hidden);
                path.pop();
            }
        }
        _ => {}
    }
}

fn restore_json(value: &mut Value, path: &mut Path, hidden: &[(Path, Value)]) -> Result<usize> {
    let mut restored = 0;
    match value {
        Value::String(text) if text == HIDDEN => {
            let original = hidden
                .iter()
                .find(|(at, _)| at == path)
                .map(|(_, original)| original.clone())
                .ok_or_else(|| {
                    anyhow!(
                        "{HIDDEN} 只能留在原来的位置（{}）；要改令牌请用接入配置",
                        display_path(path)
                    )
                })?;
            *value = original;
            restored += 1;
        }
        Value::Array(items) => {
            for (index, item) in items.iter_mut().enumerate() {
                path.push(Seg::Index(index));
                restored += restore_json(item, path, hidden)?;
                path.pop();
            }
        }
        Value::Object(map) => {
            for (name, item) in map.iter_mut() {
                path.push(Seg::Key(name.clone()));
                restored += restore_json(item, path, hidden)?;
                path.pop();
            }
        }
        _ => {}
    }
    Ok(restored)
}

fn parse_json_object(bytes: &[u8], what: &str) -> Result<Map<String, Value>> {
    serde_json::from_slice::<Value>(bytes)
        .ok()
        .and_then(|value| value.as_object().cloned())
        .ok_or_else(|| anyhow!("{what}不是合法的 JSON 对象"))
}

fn pretty(map: &Map<String, Value>) -> Result<String> {
    let mut text = serde_json::to_string_pretty(&Value::Object(map.clone()))?;
    text.push('\n');
    Ok(text)
}

// ---------- TOML（Codex config.toml，toml_edit 原位修改保留注释与格式） ----------

fn hide_toml_value(
    value: &mut toml_edit::Value,
    path: &Path,
    key: Option<&str>,
    hidden: &mut Vec<(Path, String)>,
) {
    match value {
        toml_edit::Value::String(text) => {
            let original = text.value().clone();
            if should_hide(key, &original) {
                let decor = value.decor().clone();
                *value = toml_edit::Value::from(HIDDEN);
                *value.decor_mut() = decor;
                hidden.push((path.clone(), original));
            }
        }
        toml_edit::Value::Array(items) => {
            for (index, item) in items.iter_mut().enumerate() {
                let mut next = path.clone();
                next.push(Seg::Index(index));
                hide_toml_value(item, &next, key, hidden);
            }
        }
        toml_edit::Value::InlineTable(table) => {
            for (name, item) in table.iter_mut() {
                let name = name.get().to_string();
                let mut next = path.clone();
                next.push(Seg::Key(name.clone()));
                hide_toml_value(item, &next, Some(&name), hidden);
            }
        }
        _ => {}
    }
}

fn hide_toml_table(table: &mut toml_edit::Table, path: &Path, hidden: &mut Vec<(Path, String)>) {
    for (name, item) in table.iter_mut() {
        let name = name.get().to_string();
        let mut next = path.clone();
        next.push(Seg::Key(name.clone()));
        match item {
            toml_edit::Item::Value(value) => hide_toml_value(value, &next, Some(&name), hidden),
            toml_edit::Item::Table(inner) => hide_toml_table(inner, &next, hidden),
            toml_edit::Item::ArrayOfTables(tables) => {
                for (index, inner) in tables.iter_mut().enumerate() {
                    let mut at = next.clone();
                    at.push(Seg::Index(index));
                    hide_toml_table(inner, &at, hidden);
                }
            }
            toml_edit::Item::None => {}
        }
    }
}

fn restore_toml_value(
    value: &mut toml_edit::Value,
    path: &Path,
    hidden: &[(Path, String)],
    restored: &mut usize,
) -> Result<()> {
    match value {
        toml_edit::Value::String(text) if text.value() == HIDDEN => {
            let original = hidden
                .iter()
                .find(|(at, _)| at == path)
                .map(|(_, original)| original.clone())
                .ok_or_else(|| {
                    anyhow!(
                        "{HIDDEN} 只能留在原来的位置（{}）；要改令牌请用接入配置",
                        display_path(path)
                    )
                })?;
            let decor = value.decor().clone();
            *value = toml_edit::Value::from(original);
            *value.decor_mut() = decor;
            *restored += 1;
        }
        toml_edit::Value::Array(items) => {
            for (index, item) in items.iter_mut().enumerate() {
                let mut next = path.clone();
                next.push(Seg::Index(index));
                restore_toml_value(item, &next, hidden, restored)?;
            }
        }
        toml_edit::Value::InlineTable(table) => {
            for (name, item) in table.iter_mut() {
                let mut next = path.clone();
                next.push(Seg::Key(name.get().to_string()));
                restore_toml_value(item, &next, hidden, restored)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn restore_toml_table(
    table: &mut toml_edit::Table,
    path: &Path,
    hidden: &[(Path, String)],
    restored: &mut usize,
) -> Result<()> {
    for (name, item) in table.iter_mut() {
        let mut next = path.clone();
        next.push(Seg::Key(name.get().to_string()));
        match item {
            toml_edit::Item::Value(value) => restore_toml_value(value, &next, hidden, restored)?,
            toml_edit::Item::Table(inner) => restore_toml_table(inner, &next, hidden, restored)?,
            toml_edit::Item::ArrayOfTables(tables) => {
                for (index, inner) in tables.iter_mut().enumerate() {
                    let mut at = next.clone();
                    at.push(Seg::Index(index));
                    restore_toml_table(inner, &at, hidden, restored)?;
                }
            }
            toml_edit::Item::None => {}
        }
    }
    Ok(())
}

fn parse_toml(text: &str, what: &str) -> Result<toml_edit::Document> {
    text.parse::<toml_edit::Document>()
        .map_err(|_| anyhow!("{what}不是合法的 TOML"))
}

fn display_path(path: &Path) -> String {
    let mut out = String::new();
    for seg in path {
        match seg {
            Seg::Key(key) => {
                if !out.is_empty() {
                    out.push('.');
                }
                out.push_str(key);
            }
            Seg::Index(index) => out.push_str(&format!("[{index}]")),
        }
    }
    out
}

// ---------- 读 / 写 ----------

/// 现有文件 → （遮住凭据后的全文, 被遮住的路径与原值）。文件不存在时为空配置。
struct Masked {
    text: String,
    hidden_json: Vec<(Path, Value)>,
    hidden_toml: Vec<(Path, String)>,
}

fn mask_existing(kind: Kind, existing: Option<&[u8]>) -> Result<Masked> {
    match kind {
        Kind::Claude => {
            let mut root = match existing {
                Some(bytes) => Value::Object(parse_json_object(bytes, "settings.json ")?),
                None => Value::Object(Map::new()),
            };
            let mut hidden = Vec::new();
            mask_json(&mut root, &mut Vec::new(), None, &mut hidden);
            let text = pretty(root.as_object().expect("object"))?;
            Ok(Masked {
                text,
                hidden_json: hidden,
                hidden_toml: Vec::new(),
            })
        }
        Kind::Codex => {
            let text = match existing {
                Some(bytes) => std::str::from_utf8(bytes)
                    .map_err(|_| anyhow!("config.toml 不是合法 UTF-8"))?
                    .to_string(),
                None => String::new(),
            };
            let mut doc = parse_toml(&text, "config.toml ")?;
            let mut hidden = Vec::new();
            hide_toml_table(doc.as_table_mut(), &Vec::new(), &mut hidden);
            Ok(Masked {
                text: doc.to_string(),
                hidden_json: Vec::new(),
                hidden_toml: hidden,
            })
        }
    }
}

fn element_id(req: &Request) -> Result<String> {
    Ok(req
        .value("--only")
        .ok_or_else(|| anyhow!("需要 --only agents:<助手>"))?
        .trim_start_matches("agents:")
        .to_string())
}

fn format_of(kind: Kind) -> &'static str {
    match kind {
        Kind::Claude => "json",
        Kind::Codex => "toml",
    }
}

fn tildify(ctx: &Context, path: &std::path::Path) -> String {
    match path.strip_prefix(&ctx.home_dir) {
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

/// `agents:config-file --only agents:<id>`：只读零写。
pub(crate) fn read(ctx: &Context, req: &Request) -> Result<Value> {
    for arg in req.args.iter().filter(|arg| arg.starts_with("--")) {
        if arg != "--only" {
            bail!("agents:config-file 只接受 --only");
        }
    }
    let id = element_id(req)?;
    let (kind, path, _real, existing) = crate::native_agent_settings::editable_target(ctx, &id)?;
    let base = existing
        .as_deref()
        .map_or_else(|| BASE_MISSING.to_string(), sha256);
    let (content, hidden, blocked) = match mask_existing(kind, existing.as_deref()) {
        Ok(masked) => {
            let count = masked.hidden_json.len() + masked.hidden_toml.len();
            (Some(masked.text), count, None)
        }
        // 读不懂的文件不给原文（里面可能有凭据），请用户在编辑器里手动修
        Err(err) => (
            None,
            0,
            Some(format!("{err:#}，请先在编辑器里修好再来编辑")),
        ),
    };
    Ok(json!({
        "domain": "agents",
        "element": format!("agents:{id}"),
        "path": tildify(ctx, &path),
        "format": format_of(kind),
        "exists": existing.is_some(),
        "content": content,
        "hidden": hidden,
        "base": base,
        "blocked_reason": blocked,
    }))
}

fn top_level_changes_json(before: &Map<String, Value>, after: &Map<String, Value>) -> Vec<String> {
    let mut keys: Vec<String> = before
        .keys()
        .chain(after.keys())
        .filter(|key| before.get(*key) != after.get(*key))
        .cloned()
        .collect();
    keys.sort();
    keys.dedup();
    keys
}

fn top_level_changes_toml(
    before: &toml_edit::Document,
    after: &toml_edit::Document,
) -> Vec<String> {
    let render = |doc: &toml_edit::Document, key: &str| doc.get(key).map(|item| item.to_string());
    let mut keys: Vec<String> = before
        .iter()
        .map(|(key, _)| key.to_string())
        .chain(after.iter().map(|(key, _)| key.to_string()))
        .filter(|key| render(before, key) != render(after, key))
        .collect();
    keys.sort();
    keys.dedup();
    keys
}

/// `agents:edit-config --only agents:<id> --file-base <base> --value <全文>`：整文件 CAS 写入。
pub(crate) fn write(ctx: &Context, req: &Request) -> Result<Value, DispatchError> {
    let usage = |message: &str| DispatchError::new("USAGE_ERROR", message.to_string());
    let agent_error = |err: anyhow::Error| DispatchError::new("AGENT_ERROR", format!("{err:#}"));
    let mut iter = req.args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--only" | "--file-base" | "--value" => {
                iter.next();
            }
            _ => {
                return Err(usage(
                    "agents:edit-config 只接受 --only、--file-base 与 --value",
                ))
            }
        }
    }
    let id = element_id(req).map_err(agent_error)?;
    let base = req
        .value("--file-base")
        .ok_or_else(|| {
            usage("agents:edit-config 需要 --file-base：先读 agents:config-file，把 base 原样带回")
        })?
        .to_string();
    let text = req
        .value("--value")
        .ok_or_else(|| usage("agents:edit-config 需要 --value（配置文件全文）"))?
        .to_string();

    let (kind, path, real, existing) =
        crate::native_agent_settings::editable_target(ctx, &id).map_err(agent_error)?;
    let file_label = tildify(ctx, &path);
    let current = existing
        .as_deref()
        .map_or_else(|| BASE_MISSING.to_string(), sha256);
    if current != base {
        return Err(DispatchError::new(
            "AGENT_CONFIG_CHANGED",
            format!(
                "{file_label} 已在外部修改（与打开时读到的内容不同），没有写入；请刷新后重新编辑"
            ),
        ));
    }
    let masked = mask_existing(kind, existing.as_deref()).map_err(agent_error)?;

    let (new_bytes, changed, restored) = match kind {
        Kind::Claude => {
            let mut edited =
                Value::Object(parse_json_object(text.as_bytes(), "新内容").map_err(agent_error)?);
            let restored = restore_json(&mut edited, &mut Vec::new(), &masked.hidden_json)
                .map_err(agent_error)?;
            let edited = edited.as_object().expect("object").clone();
            let before = existing
                .as_deref()
                .map(|bytes| parse_json_object(bytes, "settings.json "))
                .transpose()
                .map_err(agent_error)?
                .unwrap_or_default();
            let changed = top_level_changes_json(&before, &edited);
            (
                pretty(&edited).map_err(agent_error)?.into_bytes(),
                changed,
                restored,
            )
        }
        Kind::Codex => {
            let mut edited = parse_toml(&text, "新内容").map_err(agent_error)?;
            let mut restored = 0;
            restore_toml_table(
                edited.as_table_mut(),
                &Vec::new(),
                &masked.hidden_toml,
                &mut restored,
            )
            .map_err(agent_error)?;
            let before_text = existing
                .as_deref()
                .map(|bytes| String::from_utf8_lossy(bytes).to_string())
                .unwrap_or_default();
            let before = parse_toml(&before_text, "config.toml ").map_err(agent_error)?;
            let changed = top_level_changes_toml(&before, &edited);
            (edited.to_string().into_bytes(), changed, restored)
        }
    };

    if existing.as_deref() == Some(new_bytes.as_slice())
        || (existing.is_none() && changed.is_empty())
    {
        return Ok(json!({
            "domain": "agents", "element": format!("agents:{id}"), "path": file_label,
            "changed": [], "base": current,
        }));
    }
    if let Some(parent) = real.parent() {
        std::fs::create_dir_all(parent).map_err(|err| agent_error(err.into()))?;
    }
    if let Err(err) = crate::native_mcp::rollback::atomic_write_bytes_checked(
        &real,
        &new_bytes,
        existing.as_deref(),
    ) {
        // 只有文件确实变了才算外部修改；权限、磁盘等错误如实报出
        let now = crate::native_mcp::rollback::read_existing_bytes(&real).ok();
        if now.as_ref() != Some(&existing) {
            return Err(DispatchError::new(
                "AGENT_CONFIG_CHANGED",
                format!("{file_label} 已在外部修改（写入前核对不一致），没有写入"),
            ));
        }
        return Err(agent_error(err));
    }
    let audit = (|| -> Result<()> {
        let conn = crate::store::open_write(ctx)?;
        crate::store::record_operation(
            &conn,
            Some(&req.run_id),
            "agents:edit-config",
            Some(&format!("agents:{id}")),
            Some(&json!({"action": "edit-config", "changed": changed, "hidden_kept": restored})),
        )?;
        Ok(())
    })();
    if let Err(err) = audit {
        // 审计写不进去：把文件还原，不留下没有记录的改动
        let _ = match existing.as_deref() {
            Some(bytes) => std::fs::write(&real, bytes),
            None => std::fs::remove_file(&real),
        };
        return Err(agent_error(err));
    }
    Ok(json!({
        "domain": "agents",
        "element": format!("agents:{id}"),
        "path": file_label,
        "changed": changed,
        "base": sha256(&new_bytes),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 凭据按键名或取值遮住_变量名与路径照常显示() {
        assert!(should_hide(
            Some("ANTHROPIC_AUTH_TOKEN"),
            "custom-token-9f3a"
        ));
        assert!(should_hide(
            Some("theme"),
            "sk-abcdefghijklmnopqrstuvwxyz0123"
        ));
        assert!(!should_hide(Some("env_key"), "OPENAI_API_KEY"));
        assert!(!should_hide(
            Some("apiKeyHelper"),
            "/usr/local/bin/get-key.sh"
        ));
        assert!(!should_hide(Some("GITHUB_TOKEN"), "${GITHUB_TOKEN}"));
        assert!(!should_hide(Some("theme"), "dark"));
    }

    #[test]
    fn json_遮住后原位写回还原_挪动占位符被拒() {
        let file = br#"{"env":{"ANTHROPIC_AUTH_TOKEN":"tok-secret-1","MY":"1"},"theme":"dark"}"#;
        let masked = mask_existing(Kind::Claude, Some(file)).unwrap();
        assert!(masked.text.contains(HIDDEN) && !masked.text.contains("tok-secret-1"));
        let mut edited: Value =
            serde_json::from_str(&masked.text.replace("dark", "light")).unwrap();
        assert_eq!(
            restore_json(&mut edited, &mut Vec::new(), &masked.hidden_json).unwrap(),
            1
        );
        assert_eq!(edited["env"]["ANTHROPIC_AUTH_TOKEN"], "tok-secret-1");
        assert_eq!(edited["theme"], "light");
        // 占位符挪到别的键：拒绝
        let mut moved: Value =
            serde_json::from_str(r#"{"env":{"OTHER":"<hidden>","MY":"1"},"theme":"dark"}"#)
                .unwrap();
        assert!(restore_json(&mut moved, &mut Vec::new(), &masked.hidden_json).is_err());
    }

    #[test]
    fn toml_遮住后原位还原_注释与格式保留() {
        let file = "# 我的配置\nmodel = \"o3\" # 行尾注释\n\n[mcp_servers.gh]\ncommand = \"gh\"\n[mcp_servers.gh.env]\nGITHUB_TOKEN = \"tok-secret-2\"\n";
        let masked = mask_existing(Kind::Codex, Some(file.as_bytes())).unwrap();
        assert!(
            masked.text.contains("GITHUB_TOKEN = \"<hidden>\""),
            "{}",
            masked.text
        );
        assert!(!masked.text.contains("tok-secret-2"));
        let edited_text = masked.text.replace("\"o3\"", "\"o4\"");
        let mut edited = parse_toml(&edited_text, "x").unwrap();
        let mut restored = 0;
        restore_toml_table(
            edited.as_table_mut(),
            &Vec::new(),
            &masked.hidden_toml,
            &mut restored,
        )
        .unwrap();
        assert_eq!(restored, 1);
        let out = edited.to_string();
        assert!(out.contains("GITHUB_TOKEN = \"tok-secret-2\""), "{out}");
        assert!(out.contains("model = \"o4\" # 行尾注释"), "{out}");
        assert!(out.starts_with("# 我的配置\n"), "{out}");
    }
}
