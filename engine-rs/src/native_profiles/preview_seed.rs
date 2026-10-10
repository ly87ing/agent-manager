//! 新建实例时只读计算「先写实例主文件，再带入勾选设置」的可见结果。

use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};
use std::path::Path;

use super::model::{ProfileDef, ProfileKind};
use super::seed;
use crate::native::{Context, Request};
use crate::native_mcp::{
    self as mcp,
    inventory::{FileEntry, TargetScan},
};

pub(super) struct Preview {
    pub main: Vec<u8>,
    pub files: Vec<Value>,
    pub results: Vec<Value>,
}

fn result(item: &str, state: &str, detail: impl Into<String>) -> Value {
    json!({"item": item, "state": state, "detail": detail.into()})
}

fn sub_request(req: &Request, command: &str) -> Request {
    Request {
        command: command.to_string(),
        args: Vec::new(),
        run_id: req.run_id.clone(),
    }
}

fn read_optional(ctx: &Context, def: &ProfileDef, path: &Path) -> Result<Option<Vec<u8>>> {
    seed::read_target(ctx, def, path).map(|(_, bytes)| bytes)
}

fn file(path: String, original: Option<&[u8]>, bytes: Vec<u8>) -> Result<Value> {
    if bytes.len() > 4 * 1024 * 1024 {
        bail!("配置文件超过安全读取上限");
    }
    Ok(json!({
        "path": path,
        "exists": original.is_some(),
        "original": crate::native_mcp::original_text(original),
        "content": String::from_utf8(bytes).map_err(|_| anyhow!("预览结果不是 UTF-8 文本"))?,
    }))
}

fn rules(
    ctx: &Context,
    req: &Request,
    def: &ProfileDef,
    dir: &Path,
) -> Result<(Value, Option<Value>)> {
    let status = crate::native_rules::handle(ctx, &sub_request(req, "rules:status"))?;
    let (source_id, name) = match def.kind {
        ProfileKind::Claude => ("rules:claude_global", "CLAUDE.md"),
        ProfileKind::Codex => ("rules:codex_global", "AGENTS.md"),
    };
    let source =
        seed::element(&status, source_id).ok_or_else(|| anyhow!("默认环境没有规则目标"))?;
    let content = source.pointer("/observed/content").and_then(Value::as_str);
    if content.is_none_or(|text| text.trim().is_empty()) {
        let detail = match source
            .pointer("/observed/blocked_reason")
            .and_then(Value::as_str)
        {
            Some(why) => format!("读不到默认环境的 {name}：{why}"),
            None => format!("默认环境没有 {name}"),
        };
        return Ok((result("rules", "absent", detail), None));
    }
    let path = dir.join(name);
    let previous = read_optional(ctx, def, &path)?;
    let existing = previous
        .as_deref()
        .map(std::str::from_utf8)
        .transpose()
        .map_err(|_| anyhow!("独立环境的 {name} 不是 UTF-8 文本"))?;
    let copied = existing.is_none_or(|text| text.trim().is_empty());
    let bytes = if copied {
        content.unwrap().as_bytes().to_vec()
    } else {
        previous.clone().unwrap()
    };
    let state = if copied { "copied" } else { "exists" };
    let detail = if copied {
        format!("已复制 {name}")
    } else {
        format!("独立环境已有 {name}，没有改")
    };
    let display = format!("{}/{name}", def.config_dir_raw.trim_end_matches('/'));
    Ok((
        result("rules", state, detail),
        Some(file(display, previous.as_deref(), bytes)?),
    ))
}

pub(super) fn mcp_scan(
    def: &ProfileDef,
    path: &Path,
    bytes: Option<Vec<u8>>,
) -> Result<TargetScan> {
    let format = match def.kind {
        ProfileKind::Claude => mcp::Format::ClaudeJson,
        ProfileKind::Codex => mcp::Format::CodexToml,
    };
    let (container, _) = mcp::inventory::parse_container(&format, path, bytes.as_deref())?;
    let entries = container
        .into_iter()
        .map(|(name, raw)| FileEntry {
            base: mcp::model::digest_of(&raw),
            view: mcp::entry::parse(&format, &raw),
            name,
            raw,
        })
        .collect();
    Ok(TargetScan {
        target: mcp::AgentTarget {
            agent: def.id.clone(),
            path: path.to_path_buf(),
            format,
            enabled: def.enabled,
        },
        exists: bytes.is_some(),
        error: None,
        entries,
        old_bytes: bytes,
    })
}

fn mcp(
    ctx: &Context,
    def: &ProfileDef,
    dir: &Path,
    main: &mut Vec<u8>,
) -> Result<(Value, Option<Value>)> {
    let mut inv = mcp::inventory::load(ctx)?;
    let main_agent = mcp::model::canonicalize_agent_id(&def.base_agent);
    let Some(source) = inv.scan(&main_agent) else {
        return Ok((result("mcp", "absent", "默认环境没有 MCP 配置"), None));
    };
    if let Some(err) = &source.error {
        return Ok((
            result("mcp", "absent", format!("读不到默认环境的 MCP 配置：{err}")),
            None,
        ));
    }
    if source.entries.is_empty() {
        return Ok((result("mcp", "absent", "默认环境没有 MCP 服务"), None));
    }
    let source_entries = source.entries.clone();
    let source_format = source.target.format.clone();
    let target_path = match def.kind {
        ProfileKind::Claude => dir.join(".claude.json"),
        ProfileKind::Codex => dir.join("config.toml"),
    };
    if let Some(other) = inv.scans.iter().find(|scan| {
        scan.target.agent != def.id
            && mcp::targets::resolved_path(&scan.target.path)
                == mcp::targets::resolved_path(&target_path)
    }) {
        bail!(
            "与助手 '{}' 共用同一个 MCP 配置文件，为保证隔离不予操作",
            other.target.agent
        );
    }
    let previous = match def.kind {
        ProfileKind::Claude => read_optional(ctx, def, &target_path)?,
        ProfileKind::Codex => Some(main.clone()),
    };
    // 写入前的 .claude.json 原文（Claude 的 MCP 另写这个文件）：界面据此只显示变化的部分
    let claude_original = match def.kind {
        ProfileKind::Claude => previous.clone(),
        ProfileKind::Codex => None,
    };
    let target_scan = mcp_scan(def, &target_path, previous)?;
    inv.scans.retain(|scan| scan.target.agent != def.id);
    inv.scans.push(target_scan);

    let (mut copied, mut existing, mut skipped, mut failures, mut notes) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for entry in source_entries {
        if inv
            .scan(&def.id)
            .and_then(|scan| scan.entry(&entry.name))
            .is_some()
            || inv.saved_row(&entry.name, &def.id).is_some()
        {
            existing.push(entry.name);
            continue;
        }
        let Ok(view) = &entry.view else {
            skipped.push(entry.name);
            continue;
        };
        if seed::cannot_share_account_entry(&source_format, view) {
            skipped.push(entry.name);
            continue;
        }
        let parsed = mcp::preflight::ParsedAction {
            server: entry.name.clone(),
            action: mcp::preflight::Action::Copy,
            agent: Some(main_agent.clone()),
            to: Some(def.id.clone()),
            spec: None,
            base: Some(entry.base),
            to_base: None,
            replace: false,
        };
        match mcp::preflight::plan(ctx, &parsed, &inv) {
            Ok(plan) => {
                notes.extend(plan.warnings);
                if let Some(write) = plan.file_writes.into_iter().next() {
                    let next = mcp_scan(def, &target_path, Some(write.new_bytes))?;
                    *inv.scans
                        .iter_mut()
                        .find(|scan| scan.target.agent == def.id)
                        .unwrap() = next;
                }
                copied.push(entry.name);
            }
            Err(err) => failures.push(format!(
                "{}（{}）",
                entry.name,
                crate::sanitize::sanitize_diagnostic_message(&format!("{err:#}"))
            )),
        }
    }
    let mut parts = Vec::new();
    if !copied.is_empty() {
        parts.push(format!("已复制 {}", copied.join("、")));
    }
    if !existing.is_empty() {
        parts.push(format!("已有同名、没有改：{}", existing.join("、")));
    }
    if !skipped.is_empty() {
        parts.push(format!(
            "凭据归属或格式无法确认、没有复制：{}（请在目标账号单独配置）",
            skipped.join("、")
        ));
    }
    if !failures.is_empty() {
        parts.push(format!("没有复制成功：{}", failures.join("；")));
    }
    parts.extend(notes);
    let state = if !copied.is_empty() {
        "copied"
    } else if !failures.is_empty() {
        "failed"
    } else if !skipped.is_empty() {
        "skipped"
    } else {
        "exists"
    };
    let bytes = inv.scan(&def.id).and_then(|scan| scan.old_bytes.clone());
    let extra = if def.kind == ProfileKind::Claude {
        bytes
            .map(|bytes| {
                file(
                    format!("{}/.claude.json", def.config_dir_raw.trim_end_matches('/')),
                    claude_original.as_deref(),
                    bytes,
                )
            })
            .transpose()?
    } else {
        if let Some(bytes) = bytes {
            *main = bytes;
        }
        None
    };
    Ok((result("mcp", state, parts.join("；")), extra))
}

pub(super) fn preview(
    ctx: &Context,
    req: &Request,
    def: &ProfileDef,
    dir: &Path,
    main: Vec<u8>,
    items: &[&str],
) -> Preview {
    let (mut main, mut results) = match seed::preview_settings(ctx, def.kind, main.clone(), items) {
        Ok(outcome) => outcome,
        Err(err) => {
            let detail = crate::sanitize::sanitize_diagnostic_message(&format!("{err:#}"));
            let failed = items
                .iter()
                .filter(|item| matches!(**item, "permissions" | "statusline"))
                .map(|item| result(item, "failed", &detail))
                .collect();
            (main, failed)
        }
    };
    let mut files = Vec::new();
    for item in items {
        let outcome = match *item {
            "rules" => rules(ctx, req, def, dir),
            "mcp" => mcp(ctx, def, dir, &mut main),
            _ => continue,
        };
        match outcome {
            Ok((result, Some(file))) => {
                results.push(result);
                files.push(file);
            }
            Ok((result, None)) => results.push(result),
            Err(err) => results.push(result(
                item,
                "failed",
                crate::sanitize::sanitize_diagnostic_message(&format!("{err:#}")),
            )),
        }
    }
    Preview {
        main,
        files,
        results,
    }
}
