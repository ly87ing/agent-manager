//! 停用快照：Claude 停用时把条目移出文件、存进 `mcp_saved`；重新启用或复制时写回。
//!
//! 快照里不留疑似凭据：env 字面量命中疑似凭据检测的、以及全部 header 字面量，
//! 转存到用户选择的外部密钥文件并改成 `${VAR}`，记入 `resolve`（写回时解析成原值）；
//! 未命中检测的普通 env 字面量（如 `NODE_ENV=production`）原样保留。

use anyhow::{anyhow, bail, Result};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

use super::entry::{self, env_ref, neutral_from_view, HeaderVal, NVal, Neutral, View};
use super::saved::{Resolve, SavedRow};
use super::targets::Format;

/// 转存计划：变量名 → 取值（只在进程内使用，不进日志与库）。
pub type Adoptions = BTreeMap<String, String>;

fn upper_token(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect()
}

fn env_var_name(key: &str) -> String {
    if crate::secret_values::valid_var_name(key) {
        return key.to_string();
    }
    let mut out: String = key
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if out.is_empty() || out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, '_');
    }
    out
}

/// 选一个可用的变量名：外部文件（或进程环境）里还没有，或已有且取值相同；
/// 同一次停用里也不能让两个不同取值落到同一个名字。
fn pick_var(candidates: &[String], value: &str, planned: &Adoptions) -> Option<String> {
    for name in candidates {
        if !crate::secret_values::valid_var_name(name) {
            continue;
        }
        if let Some(existing) = planned.get(name) {
            if existing == value {
                return Some(name.clone());
            }
            continue;
        }
        match crate::secret_values::lookup(name) {
            None => return Some(name.clone()),
            Some(existing) if existing == value => return Some(name.clone()),
            Some(_) => continue,
        }
    }
    None
}

/// Claude 停用：由文件条目生成快照，返回（快照行, 需要转存的变量）。
pub fn from_claude_entry(
    server: &str,
    agent: &str,
    raw: &Map<String, Value>,
    view: &View,
) -> Result<(SavedRow, Adoptions)> {
    let format = Format::ClaudeJson;
    let mut entry = entry::without_marker(&format, raw);
    let mut resolve = Resolve::default();
    let mut adoptions = Adoptions::new();
    let server_token = upper_token(server);

    let mut env_updates = Vec::new();
    for (key, value) in &view.env {
        // 名字像凭据（GITHUB_TOKEN 等）或取值像凭据都转存；只有两者都不像的普通字面量
        // （如 NODE_ENV=production）才原样进快照
        if env_ref(&format, value).is_some()
            || (!crate::native::credential_like_name(key)
                && crate::native::reject_secret_text(value).is_none())
        {
            continue;
        }
        let candidates = vec![
            env_var_name(key),
            format!("MCP_{server_token}_{}", upper_token(key)),
        ];
        let var = pick_var(&candidates, value, &adoptions).ok_or_else(|| {
            anyhow!(
                "外部密钥文件（或进程环境）里已有变量 {} 且取值不同，无法自动转存环境变量 {key}",
                candidates.join(" / ")
            )
        })?;
        adoptions.insert(var.clone(), value.clone());
        resolve.env.push(key.clone());
        env_updates.push((key.clone(), var));
    }
    let mut header_updates = Vec::new();
    for (name, value) in &view.headers {
        let HeaderVal::Lit(literal) = value else {
            continue;
        };
        let candidates = vec![
            format!("{server_token}_{}", upper_token(name)),
            format!("MCP_{server_token}_{}", upper_token(name)),
        ];
        let var = pick_var(&candidates, literal, &adoptions).ok_or_else(|| {
            anyhow!(
                "外部密钥文件（或进程环境）里已有变量 {} 且取值不同，无法自动转存请求头 {name}",
                candidates.join(" / ")
            )
        })?;
        adoptions.insert(var.clone(), literal.clone());
        resolve.headers.push(name.clone());
        header_updates.push((name.clone(), var));
    }
    for (field, updates) in [("env", env_updates), ("headers", header_updates)] {
        if let Some(Value::Object(map)) = entry.get_mut(field) {
            for (key, var) in updates {
                map.insert(key, Value::String(format!("${{{var}}}")));
            }
        }
    }
    if !adoptions.is_empty() {
        let (path, error) = crate::secret_values::source_state();
        if path.is_none() || error.is_some() {
            let mut keys: Vec<String> = resolve.env.clone();
            keys.extend(resolve.headers.iter().cloned());
            bail!(
                "条目 {server} 含疑似凭据的明文取值（{}），停用需要把它们转存到外部密钥文件：请先在「设置 → 密钥」选择可用的外部文件",
                keys.join(", ")
            );
        }
    }
    let row = SavedRow {
        server: server.to_string(),
        agent: agent.to_string(),
        entry: Value::Object(entry),
        resolve,
        origin: super::saved::ORIGIN_DISABLED.to_string(),
    };
    super::saved::validate_row(&row)
        .map_err(|err| anyhow!("{err:#}；请先把这类取值改成 ${{VAR}} 引用再停用"))?;
    Ok((row, adoptions))
}

fn resolve_value(var_ref: &str, missing: &mut Vec<String>) -> String {
    let Some(var) = super::model::placeholder_var(var_ref) else {
        return var_ref.to_string();
    };
    match crate::secret_values::lookup(var) {
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
