//! 停用快照：Claude 停用时把条目移出文件、存进 `mcp_saved`；重新启用或复制时写回。
//!
//! 快照只保留已有变量引用与普通字面量；需要迁移明文凭据的条目失败关闭，
//! 用户必须在同一账号显式配置引用。停用操作不会写外部密钥文件。

use super::entry::{self, neutral_from_view, NVal, Neutral, View};
use super::saved::{Resolve, SavedRow};
use super::targets::Format;
use anyhow::{anyhow, bail, Result};
use serde_json::{Map, Value};

/// A disable action must not migrate user credentials as a side effect.
/// Existing references and harmless literals can be snapshotted unchanged.
pub fn from_claude_entry(
    server: &str,
    agent: &str,
    raw: &Map<String, Value>,
    _view: &View,
) -> Result<SavedRow> {
    let row = SavedRow {
        server: server.to_string(),
        agent: agent.to_string(),
        entry: Value::Object(entry::without_marker(&Format::ClaudeJson, raw)),
        resolve: Resolve::default(),
        origin: super::saved::ORIGIN_DISABLED.to_string(),
    };
    super::saved::validate_row(&row).map_err(|_| anyhow!(
        "条目 {server} 含需要独立配置的明文或不安全字段，没有停用；请在此账号显式配置变量引用，工具不会自动迁移或覆盖凭据"
    ))?;
    Ok(row)
}

fn resolve_value(var_ref: &str, missing: &mut Vec<String>) -> String {
    let Some(var) = super::model::placeholder_var(var_ref) else {
        return var_ref.to_string();
    };
    match crate::secret_values::lookup_account(var).ok() {
        Some(value) => value,
        None => {
            missing.push(var.to_string());
            String::new()
        }
    }
}

/// 写回快照：`resolve` 列出的键解析成取值，其余原样。
pub fn restore(row: &SavedRow) -> Result<Map<String, Value>> {
    let mut entry = row
        .entry
        .as_object()
        .cloned()
        .ok_or_else(|| anyhow!("快照条目不是对象"))?;
    let mut missing = Vec::new();
    for key in &row.resolve.env {
        for field in ["env", "environment"] {
            if let Some(Value::Object(map)) = entry.get_mut(field) {
                if let Some(Value::String(text)) = map.get_mut(key) {
                    *text = resolve_value(text, &mut missing);
                }
            }
        }
    }
    for key in &row.resolve.headers {
        if let Some(Value::Object(map)) = entry.get_mut("headers") {
            if let Some(Value::String(text)) = map.get_mut(key) {
                *text = resolve_value(text, &mut missing);
            }
        }
    }
    if !missing.is_empty() {
        missing.sort();
        missing.dedup();
        bail!("凭证变量未配置: {}", missing.join(", "));
    }
    Ok(entry)
}

/// 快照作为复制来源：视图 + resolve 标记（转存/迁移来的引用写入时解析）。
pub fn neutral(format: &Format, row: &SavedRow) -> Result<(Neutral, View)> {
    let view = entry::parse(format, &row.entry).map_err(|e| anyhow!("快照结构无效: {e}"))?;
    let mut n = neutral_from_view(format, &view);
    for (key, value) in n.env.iter_mut() {
        if row.resolve.env.contains(key) {
            if let NVal::Ref { resolve, .. } = value {
                *resolve = true;
            }
        }
    }
    for (key, value) in n.headers.iter_mut() {
        if row.resolve.headers.contains(key) {
            if let NVal::Ref { resolve, .. } = value {
                *resolve = true;
            }
        }
    }
    Ok((n, view))
}
