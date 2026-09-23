//! 单条目写入：在内存里算出目标文件的新字节，并在落盘前自检。
//!
//! - JSON（Claude/OpenCode）：在解析出的文档上替换/插入/删除该键（键顺序保留），
//!   整份以 2 空格缩进重新序列化，与既有行为一致。
//! - Codex TOML：已有条目用 `toml_edit` 原位同步（只动变化的键，注释、未建模字段、
//!   其他表逐字节保留）；新增条目在文件末尾追加文本块；表头形式的条目按既有文本规则
//!   删除（保留表头上方的用户注释、连带删除紧邻的旧版注释）。
//!
//! 自检：新文本必须可解析；目标条目恰为预期；同一容器里其他条目与文件其余部分
//! 解析后的内容与写前完全相同。任何一项不满足都拒绝写入。

use anyhow::{anyhow, bail, Result};
use serde_json::{Map, Value};

use super::codecs::{json_doc_from_bytes, render_entry_toml, toml_remove_server_text};
use super::entry::codex_entry_from_map;
use super::inventory::{parse_container, TargetScan};
use super::model::LEGACY_TOML_COMMENT;
use super::targets::Format;

pub enum Change<'a> {
    /// 让该条目恰好等于给定内容（不存在则新增）。
    Set(&'a Map<String, Value>),
    Remove,
}

fn json_container_key(format: &Format) -> &'static str {
    if *format == Format::OpenCodeJson {
        "mcp"
    } else {
        "mcpServers"
    }
}

fn render_json(scan: &TargetScan, name: &str, change: &Change) -> Result<Vec<u8>> {
    let format = &scan.target.format;
    let mut doc = match &scan.old_bytes {
        Some(bytes) => json_doc_from_bytes(&scan.target.path, bytes)?,
        None => Map::new(),
    };
    let key = json_container_key(format);
    match change {
        Change::Set(new) => {
            let container = doc
                .entry(key.to_string())
                .or_insert_with(|| Value::Object(Map::new()));
            if container.is_null() {
                *container = Value::Object(Map::new());
            }
            let map = container
                .as_object_mut()
                .ok_or_else(|| anyhow!("配置文件中的 '{key}' 不是对象"))?;
            // preserve_order：已有键原位替换，新键追加在末尾
            map.insert(name.to_string(), Value::Object((*new).clone()));
        }
        Change::Remove => {
            if let Some(Value::Object(map)) = doc.get_mut(key) {
                map.shift_remove(name);
            }
        }
    }
    Ok(serde_json::to_string_pretty(&Value::Object(doc))?.into_bytes())
}

fn json_to_toml(value: &Value) -> Result<toml_edit::Value> {
    Ok(match value {
        Value::String(s) => s.as_str().into(),
        Value::Bool(b) => (*b).into(),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                i.into()
            } else if let Some(f) = n.as_f64() {
                f.into()
            } else {
                bail!("数值超出 TOML 可表示范围")
            }
        }
        Value::Array(items) => {
            let mut arr = toml_edit::Array::new();
            for item in items {
                arr.push(json_to_toml(item)?);
            }
            toml_edit::Value::Array(arr)
        }
        Value::Object(map) => {
            let mut table = toml_edit::InlineTable::new();
            for (k, v) in map {
                table.insert(k, json_to_toml(v)?);
            }
            toml_edit::Value::InlineTable(table)
        }
        Value::Null => bail!("TOML 不能表示 null"),
    })
}

/// 让 toml_edit 条目与 `new` 一致：只动与 `old` 不同的键；已有值替换时沿用原值的
/// 前后缀装饰（行尾注释不丢），子表递归同步以保留表头形式。行内表插入新键后
/// 重排该行内表的空白（行内表里没有注释可丢）。
fn sync_item(
    item: &mut toml_edit::Item,
    old: &Map<String, Value>,
    new: &Map<String, Value>,
) -> Result<()> {
    let inline = item.is_inline_table();
    let table = item
        .as_table_like_mut()
        .ok_or_else(|| anyhow!("条目不是表"))?;
    let mut inserted = false;
    for key in old.keys() {
        if !new.contains_key(key) {
            table.remove(key);
        }
    }
    for (key, value) in new {
        let previous = old.get(key);
        if previous == Some(value) {
            continue;
        }
        if let (Some(Value::Object(old_child)), Value::Object(new_child)) = (previous, value) {
            if let Some(child) = table.get_mut(key) {
                if child.is_table_like() {
                    sync_item(child, old_child, new_child)?;
                    continue;
                }
            }
        }
        let mut converted = json_to_toml(value)?;
        match table.get_mut(key) {
            Some(toml_edit::Item::Value(existing)) => {
                *converted.decor_mut() = existing.decor().clone();
                *existing = converted;
            }
            Some(item) => *item = toml_edit::Item::Value(converted),
            None => {
                table.insert(key, toml_edit::Item::Value(converted));
                inserted = true;
            }
        }
    }
    if inline && inserted {
        table.fmt();
    }
    Ok(())
}

fn strip_legacy_comment(item: &mut toml_edit::Item) {
    let Some(table) = item.as_table_mut() else {
        return;
    };
    let Some(prefix) = table
        .decor()
        .prefix()
        .and_then(|p| p.as_str())
        .map(str::to_string)
    else {
        return;
    };
    if !prefix
        .lines()
        .any(|line| line.trim() == LEGACY_TOML_COMMENT)
    {
        return;
    }
    let kept: String = prefix
        .split_inclusive('\n')
        .filter(|line| line.trim() != LEGACY_TOML_COMMENT)
        .collect();
    table.decor_mut().set_prefix(kept);
}

fn toml_text(scan: &TargetScan) -> Result<String> {
    match &scan.old_bytes {
        Some(bytes) => String::from_utf8(bytes.clone())
            .map_err(|_| anyhow!("配置文件不是有效 UTF-8: {}", scan.target.path.display())),
        None => Ok(String::new()),
    }
}

fn render_toml(scan: &TargetScan, name: &str, change: &Change) -> Result<Vec<u8>> {
    let text = toml_text(scan)?;
    let existing = scan.entry(name);
    let out = match (change, existing) {
        (Change::Set(new), Some(old_entry)) => {
            let old = old_entry
                .raw
                .as_object()
                .ok_or_else(|| anyhow!("条目 {name} 不是表"))?;
            let mut doc: toml_edit::Document = text
                .parse()
                .map_err(|_| anyhow!("TOML 配置解析失败: {}", scan.target.path.display()))?;
            let servers = doc
                .get_mut("mcp_servers")
                .and_then(|item| item.as_table_like_mut())
                .ok_or_else(|| anyhow!("找不到 mcp_servers 表"))?;
            let item = servers
                .get_mut(name)
                .ok_or_else(|| anyhow!("找不到条目 {name}"))?;
            sync_item(item, old, new)?;
            strip_legacy_comment(item);
            doc.to_string()
        }
        (Change::Set(new), None) => {
            let entry = codex_entry_from_map(new)?;
            // 追加的新表与前文恰好隔一个空行：只收掉文件末尾的空白行。
            let mut lines: Vec<&str> = text.split_inclusive('\n').collect();
            while lines.last().map_or(false, |line| line.trim().is_empty()) {
                lines.pop();
            }
            let mut content = lines.concat();
            let rendered = render_entry_toml(name, &entry);
            if content.is_empty() {
                content.push_str(rendered.trim_start_matches('\n'));
            } else {
                if !content.ends_with('\n') {
                    content.push('\n');
                }
                content.push_str(&rendered);
            }
            content
        }
        (Change::Remove, _) => {
            let doc: toml_edit::Document = text
                .parse()
                .map_err(|_| anyhow!("TOML 配置解析失败: {}", scan.target.path.display()))?;
            let header_form = doc
                .get("mcp_servers")
                .and_then(|item| item.as_table_like())
                .and_then(|servers| servers.get(name))
                .and_then(|item| item.as_table())
                .map_or(false, |table| !table.is_dotted());
            let parent_is_table = doc.get("mcp_servers").map_or(false, |item| {
                item.as_table().map_or(false, |t| !t.is_dotted())
            });
            if header_form && parent_is_table {
                toml_remove_server_text(&text, name)
            } else {
                let mut doc = doc;
                if let Some(servers) = doc
                    .get_mut("mcp_servers")
                    .and_then(|item| item.as_table_like_mut())
                {
                    servers.remove(name);
                }
                doc.to_string()
            }
        }
    };
    Ok(out.into_bytes())
}

/// 算出新字节并自检。
pub fn render(scan: &TargetScan, name: &str, change: Change) -> Result<Vec<u8>> {
    let format = &scan.target.format;
    let bytes = match format {
        Format::ClaudeJson | Format::OpenCodeJson => render_json(scan, name, &change)?,
        Format::CodexToml => render_toml(scan, name, &change)?,
        Format::Unsupported(fmt) => bail!("不支持的 MCP 配置格式: {fmt}"),
    };
    let path = &scan.target.path;
    let (old_container, old_root) = parse_container(format, path, scan.old_bytes.as_deref())?;
    let (new_container, new_root) = parse_container(format, path, Some(&bytes))
        .map_err(|e| anyhow!("写入后的配置无法解析，已放弃写入: {e:#}"))?;
    if old_root != new_root {
        bail!("写入会改动 MCP 条目以外的配置，已放弃写入");
    }
    match &change {
        Change::Set(new) => {
            if new_container.get(name) != Some(&Value::Object((*new).clone())) {
                bail!("写入后的条目 {name} 与预期不一致，已放弃写入");
            }
        }
        Change::Remove => {
            if new_container.contains_key(name) {
                bail!("条目 {name} 删除后仍存在，已放弃写入");
            }
        }
    }
    for (other, value) in &old_container {
        if other != name && new_container.get(other) != Some(value) {
            bail!("写入会改动其他条目 {other}，已放弃写入");
        }
    }
    for other in new_container.keys() {
        if other != name && !old_container.contains_key(other) {
            bail!("写入会新增其他条目 {other}，已放弃写入");
        }
    }
    Ok(bytes)
}
