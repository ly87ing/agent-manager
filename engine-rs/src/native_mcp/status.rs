//! mcp:status：各助手配置文件里的实际条目 + 已保存未生效项（只读、零写）。

use anyhow::Result;
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;

use super::entry::{self, masked_entry, masked_extra, referenced_vars, Transport, View};
use super::inventory::{self, FileEntry, Inventory, TargetScan};
use super::model::{is_valid_server_name, placeholder_var};
use super::saved::{self, SavedRow};
use super::targets::Format;
use crate::native::Context;

/// stdio 命令能否在本机解析（只读：查 PATH 与常见安装目录）。
///
/// 这是配完之后最容易踩的坑——命令没装，助手启动时才报错。只做存在性判断，
/// 不执行它，也不对远程服务做网络探测。
fn command_resolvable(command: &str, home_dir: &std::path::Path) -> bool {
    if command.contains('/') || (cfg!(windows) && command.contains('\\')) {
        return std::path::Path::new(command).is_file();
    }
    let mut dirs: Vec<std::path::PathBuf> = std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).collect())
        .unwrap_or_default();
    dirs.extend(crate::native_agents::fallback_bin_dirs(home_dir));
    // Windows 上按 PATHEXT 补扩展名（npx → npx.cmd）
    dirs.into_iter()
        .flat_map(|dir| crate::platform::command_candidates(&dir, command))
        .any(|path| path.is_file())
}

fn command_available(view: &View, home_dir: &std::path::Path) -> Value {
    match view.transport {
        Transport::Stdio if !view.command.trim().is_empty() => {
            Value::Bool(command_resolvable(view.command.trim(), home_dir))
        }
        _ => Value::Null,
    }
}

/// 无上游概念的元素固定 freshness 块。
fn freshness_not_applicable() -> Value {
    json!({
        "state": "not-applicable",
        "current": Value::Null,
        "upstream": Value::Null,
        "checked_at": Value::Null,
        "probe": "native",
        "cache_age_sec": Value::Null
    })
}

fn vars_block(format: &Format, view: &View) -> Value {
    let mut out = Map::new();
    for (key, kind, var) in referenced_vars(format, view) {
        out.insert(
            key,
            json!({
                "placeholder": format!("${{{var}}}"),
                "resolved": crate::secret_values::is_set(&var),
                "stored": crate::secret_values::is_stored(&var),
                "kind": kind,
            }),
        );
    }
    Value::Object(out)
}

/// 助手级阻断原因：文件出错、助手在本工具中停用、名称不合法。
fn agent_block(scan: Option<&TargetScan>, agent: &str, server: &str) -> Option<String> {
    let Some(scan) = scan else {
        return Some(format!("助手 '{agent}' 已没有 MCP 配置目标"));
    };
    if let Some(error) = &scan.error {
        return Some(error.clone());
    }
    if !scan.target.enabled {
        return Some(format!("助手 '{agent}' 已在本工具中停用"));
    }
    if !is_valid_server_name(server) {
        return Some(
            "服务名含本工具不支持的字符（只允许字母、数字、- 与 _），请直接在助手配置里修改"
                .to_string(),
        );
    }
    None
}

fn cell_base(state: &str, source: &str, scan: Option<&TargetScan>) -> Map<String, Value> {
    let mut cell = Map::new();
    cell.insert("state".into(), json!(state));
    cell.insert("source".into(), json!(source));
    let (target, exists) = scan.map_or((String::new(), false), |s| {
        (s.target.path.to_string_lossy().to_string(), s.exists)
    });
    cell.insert("target".into(), json!(target));
    cell.insert("target_exists".into(), json!(exists));
    cell
}

fn file_cell(
    ctx: &Context,
    scan: &TargetScan,
    server: &str,
    file_entry: &FileEntry,
    snapshot: Option<&SavedRow>,
) -> Value {
    let format = &scan.target.format;
    let block = agent_block(Some(scan), &scan.target.agent, server);
    let (state, mut cell) = match &file_entry.view {
        Ok(view) => {
            let state = if view.enabled { "enabled" } else { "disabled" };
            let mut cell = cell_base(state, "file", Some(scan));
            cell.insert(
                "transport".into(),
                json!(if view.transport == Transport::Remote {
                    "remote"
                } else {
                    "stdio"
                }),
            );
            cell.insert("entry".into(), masked_entry(format, view));
            cell.insert("literal_hints".into(), entry::literal_hints(format, view));
            cell.insert("extra".into(), masked_extra(view));
            cell.insert("vars".into(), vars_block(format, view));
            cell.insert("legacy_marker".into(), json!(view.legacy_marker));
            cell.insert(
                "command_available".into(),
                command_available(view, &ctx.home_dir),
            );
            (state, cell)
        }
        Err(reason) => {
            let mut cell = cell_base("invalid", "file", Some(scan));
            cell.insert("reason".into(), json!(reason));
            ("invalid", cell)
        }
    };
    cell.insert("base".into(), json!(file_entry.base));
    cell.insert("has_saved".into(), json!(snapshot.is_some()));
    let mut notices = Vec::new();
    if let Ok(view) = &file_entry.view {
        if view.legacy_marker {
            notices.push("含旧版 Agent Manager 标记，可能触发助手配置警告；可预览后单独清理");
        }
    }
    if snapshot.is_some() {
        notices.push("库里还保存着一份停用时的旧内容，删除会一并清掉");
    }
    cell.insert(
        "notice".into(),
        if notices.is_empty() {
            Value::Null
        } else {
            json!(notices.join("；"))
        },
    );
    let copy_block = file_entry
        .view
        .as_ref()
        .ok()
        .and_then(|view| entry::account_copy_blocked_reason(format, view));
    cell.insert(
        "copy_safe".into(),
        json!(copy_block.is_none() && file_entry.view.is_ok()),
    );
    cell.insert("copy_blocked_reason".into(), json!(copy_block));
    let mut actions: Vec<&str> = if block.is_some() {
        vec![]
    } else {
        match state {
            "enabled" => vec!["edit", "disable", "copy", "remove"],
            "disabled" => vec!["edit", "enable", "copy", "remove"],
            _ => vec!["remove"],
        }
    };
    if block.is_none()
        && file_entry
            .view
            .as_ref()
            .is_ok_and(|view| view.legacy_marker)
    {
        actions.push("clean-markers");
    }
    if copy_block.is_some() {
        actions.retain(|action| *action != "copy");
    }
    let reason = block.or_else(|| {
        (state == "invalid").then(|| {
            format!(
                "条目结构无法识别（{}），只能删除或直接在配置文件里修正",
                file_entry.view.as_ref().err().cloned().unwrap_or_default()
            )
        })
    });
    cell.insert("blocked_reason".into(), json!(reason));
    cell.insert("actions".into(), json!(actions));
    Value::Object(cell)
}

fn snapshot_cell(scan: Option<&TargetScan>, row: &SavedRow) -> Value {
    let block = agent_block(scan, &row.agent, &row.server);
    let parsed = scan
        .ok_or_else(|| "助手已没有 MCP 配置目标".to_string())
        .and_then(|s| {
            saved::validate_row(row).map_err(|e| format!("{e:#}"))?;
            entry::parse(&s.target.format, &row.entry).map(|v| (s.target.format.clone(), v))
        });
    let mut cell = match &parsed {
        Ok((format, view)) => {
            let mut cell = cell_base("disabled", "saved", scan);
            cell.insert(
                "transport".into(),
                json!(if view.transport == Transport::Remote {
                    "remote"
                } else {
                    "stdio"
                }),
            );
            cell.insert("entry".into(), masked_entry(format, view));
            cell.insert("literal_hints".into(), entry::literal_hints(format, view));
            cell.insert("extra".into(), masked_extra(view));
            cell.insert("vars".into(), vars_block(format, view));
            cell.insert("legacy_marker".into(), json!(false));
            cell.insert("command_available".into(), Value::Null);
            cell
        }
        Err(reason) => {
            let mut cell = cell_base("invalid", "saved", scan);
            cell.insert("reason".into(), json!(reason));
            cell
        }
    };
    cell.insert("base".into(), json!(row.base()));
    cell.insert("has_saved".into(), json!(false));
    cell.insert(
        "notice".into(),
        json!("已停用：条目保存在本工具里，启用时写回配置文件"),
    );
    let copy_block = parsed
        .as_ref()
        .ok()
        .and_then(|(format, view)| entry::account_copy_blocked_reason(format, view));
    cell.insert(
        "copy_safe".into(),
        json!(copy_block.is_none() && parsed.is_ok()),
    );
    cell.insert("copy_blocked_reason".into(), json!(copy_block));
    let mut actions: Vec<&str> = match (&parsed, &block) {
        (Ok(_), None) => vec!["enable", "copy", "remove"],
        // 助手没了或快照无效：只能丢掉这份保存的内容
        _ if scan.map_or(true, |s| s.target.enabled) => vec!["remove"],
        _ => vec![],
    };
    if copy_block.is_some() {
        actions.retain(|action| *action != "copy");
    }
    let reason = match (&parsed, block) {
        (_, Some(block)) => Some(block),
        (Err(reason), None) => Some(format!("已保存的内容无效（{reason}），只能删除")),
        _ => None,
    };
    cell.insert("blocked_reason".into(), json!(reason));
    cell.insert("actions".into(), json!(actions));
    Value::Object(cell)
}

/// 未分配的已保存定义的脱敏展示：引用原样，字面量掩码。
fn masked_saved_definition(row: &SavedRow) -> Value {
    let mut out = row.entry.as_object().cloned().unwrap_or_default();
    for field in ["env", "headers"] {
        if let Some(Value::Object(map)) = out.get_mut(field) {
            for value in map.values_mut() {
                let is_ref = value
                    .as_str()
                    .map_or(false, |v| placeholder_var(v).is_some());
                if !is_ref {
                    *value = json!(entry::LITERAL_MASK);
                }
            }
        }
    }
    let valid = saved::validate_row(row).is_ok();
    json!({
        "definition": Value::Object(out),
        "valid": valid,
    })
}

/// 该服务用于上游探测的定义形态（command/args）：文件条目优先（按助手 id 顺序），
/// 其次快照与未分配定义。与 refresh-upstream 的收集规则一致。
pub fn probe_definition(inv: &Inventory, server: &str) -> Option<Value> {
    for scan in &inv.scans {
        if let Some(Ok(view)) = scan.entry(server).map(|e| e.view.as_ref()) {
            if view.transport == Transport::Stdio {
                let def = json!({"command": view.command, "args": view.args});
                if crate::native_upstream::classify_mcp_probe(&def).is_some() {
                    return Some(def);
                }
            }
        }
    }
    for row in inv.saved.iter().filter(|r| r.server == server) {
        let def = if row.agent.is_empty() {
            row.entry.clone()
        } else {
            let format = inv
                .scan(&row.agent)
                .map(|s| s.target.format.clone())
                .unwrap_or(Format::ClaudeJson);
            match entry::parse(&format, &row.entry) {
                Ok(view) if view.transport == Transport::Stdio => {
                    json!({"command": view.command, "args": view.args})
                }
                _ => continue,
            }
        };
        if crate::native_upstream::classify_mcp_probe(&def).is_some() {
            return Some(def);
        }
    }
    None
}

pub fn status(ctx: &Context) -> Result<Value> {
    let inv = inventory::load(ctx)?;
    let probes = crate::native_upstream::load_probe_cache(ctx)?;

    let mut names: BTreeSet<String> = BTreeSet::new();
    for scan in &inv.scans {
        names.extend(scan.entries.iter().map(|e| e.name.clone()));
    }
    names.extend(inv.saved.iter().map(|r| r.server.clone()));

    let mut elements = Vec::new();
    for name in &names {
        let mut cells = Map::new();
        for scan in &inv.scans {
            let agent = &scan.target.agent;
            let snapshot = inv.saved_row(name, agent);
            if let Some(file_entry) = scan.entry(name) {
                cells.insert(
                    agent.clone(),
                    file_cell(ctx, scan, name, file_entry, snapshot),
                );
            } else if let Some(row) = snapshot {
                cells.insert(agent.clone(), snapshot_cell(Some(scan), row));
            }
        }
        for row in inv
            .saved
            .iter()
            .filter(|r| r.server == *name && !r.agent.is_empty())
        {
            if inv.scan(&row.agent).is_none() {
                cells.insert(row.agent.clone(), snapshot_cell(None, row));
            }
        }
        let states: Vec<&str> = cells
            .values()
            .filter_map(|c| c.get("state").and_then(Value::as_str))
            .collect();
        let unassigned = inv.saved_row(name, "");
        let state = if states.contains(&"invalid")
            || unassigned.map_or(false, |row| saved::validate_row(row).is_err())
        {
            "unsupported"
        } else if states.contains(&"enabled") {
            "managed-ok"
        } else {
            "disabled"
        };
        let freshness = if probe_definition(&inv, name).is_some() {
            crate::native_upstream::freshness_from_cache(&probes, &format!("mcp:{name}"), None)
        } else {
            freshness_not_applicable()
        };
        let actions: Vec<Value> = if unassigned.is_some() {
            vec![
                json!({"id": "forget", "risk": "write-local", "needs_lock": true, "offline_ok": true}),
            ]
        } else {
            vec![]
        };
        elements.push(json!({
            "id": format!("mcp:{name}"),
            "domain": "mcp",
            "spec": {
                "name": name,
                "saved": unassigned.map(masked_saved_definition).unwrap_or(Value::Null),
            },
            "policy": {},
            "observed": { "agents": Value::Object(cells) },
            "state": state,
            "freshness": freshness,
            "actions": actions,
        }));
    }

    let targets: Vec<Value> = inv
        .scans
        .iter()
        .map(|scan| {
            json!({
                "agent": scan.target.agent,
                "path": scan.target.path.to_string_lossy(),
                "format": scan.target.format.label(),
                "exists": scan.exists,
                "agent_enabled": scan.target.enabled,
                "disable_mode": scan.target.format.disable_mode(),
                "error": scan.error,
            })
        })
        .collect();

    Ok(json!({
        "domain": "mcp",
        "targets": targets,
        "elements": elements,
    }))
}
