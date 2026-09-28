//! 已建环境的共用设置：按逐项写入顺序在内存里计算目标文件，预览本身零写。

use anyhow::{anyhow, bail, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use super::model::{ProfileDef, ProfileKind};
use super::{preview_seed, seed, shared};
use crate::native::{Context, Request};
use crate::native_hooks::model::{self as hooks_model, HookKind};
use crate::native_mcp::{
    self as mcp,
    preflight::{Action, ParsedAction},
};

const MAX_FILE_BYTES: usize = 4 * 1024 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Selection {
    item: String,
    source_base: String,
    target_base: String,
}

struct FileState {
    path: String,
    exists: bool,
    content: Option<Vec<u8>>,
    changed: bool,
}

fn sub(req: &Request, command: &str, args: Vec<String>) -> Request {
    Request {
        command: command.into(),
        args,
        run_id: req.run_id.clone(),
    }
}

fn file<'a>(files: &'a mut BTreeMap<PathBuf, FileState>, path: &Path) -> Result<&'a mut FileState> {
    if !files.contains_key(path) {
        let (_, content) = seed::read_target(path)?;
        if content
            .as_ref()
            .is_some_and(|bytes| bytes.len() > MAX_FILE_BYTES)
        {
            bail!("配置文件超过安全读取上限");
        }
        files.insert(
            path.to_path_buf(),
            FileState {
                path: path.display().to_string(),
                exists: content.is_some(),
                content,
                changed: false,
            },
        );
    }
    Ok(files.get_mut(path).expect("刚放入文件快照"))
}

fn domain_source(
    ctx: &Context,
    req: &Request,
    def: &ProfileDef,
    item: &str,
    base: &str,
) -> Result<Value> {
    let (command, source_id, pointer) = match (item, def.kind) {
        ("hooks", ProfileKind::Claude) => ("hooks:status", "hooks:claude", "/observed/hooks"),
        ("hooks", ProfileKind::Codex) => ("hooks:status", "hooks:codex", "/observed/hooks"),
        ("rules", ProfileKind::Claude) => {
            ("rules:status", "rules:claude_global", "/observed/content")
        }
        _ => ("rules:status", "rules:codex_global", "/observed/content"),
    };
    let status = if item == "hooks" {
        crate::native_hooks::handle(ctx, &sub(req, command, vec![]))?
    } else {
        crate::native_rules::handle(ctx, &sub(req, command, vec![]))?
    };
    let source = seed::element(&status, source_id).ok_or_else(|| anyhow!("找不到来源设置"))?;
    if source.pointer("/observed/base").and_then(Value::as_str) != Some(base) {
        bail!("来源已变化，请刷新差异");
    }
    source
        .pointer(pointer)
        .filter(|value| !value.is_null())
        .cloned()
        .ok_or_else(|| anyhow!("来源内容不可读"))
}

fn render_one(
    ctx: &Context,
    req: &Request,
    def: &ProfileDef,
    dir: &Path,
    selection: &Selection,
    files: &mut BTreeMap<PathBuf, FileState>,
) -> Result<()> {
    let main = dir.join(def.kind.target_file());
    match selection.item.as_str() {
        "permissions" | "statusline" => {
            let source = shared::read_source(&shared::settings_file(ctx, def))?;
            let target = file(files, &main)?;
            let from = shared::setting_part(def.kind, source.as_deref(), &selection.item)?;
            let before =
                shared::setting_part(def.kind, target.content.as_deref(), &selection.item)?;
            if from.is_none()
                || shared::value_base(from.as_ref()) != selection.source_base
                || shared::value_base(before.as_ref()) != selection.target_base
            {
                bail!("来源或环境设置已变化，请刷新差异后重试");
            }
            target.content = Some(shared::render_setting(
                def.kind,
                source.as_deref(),
                target.content.as_deref(),
                &selection.item,
            )?);
            target.changed = true;
        }
        "hooks" => {
            let source = domain_source(ctx, req, def, "hooks", &selection.source_base)?;
            let target_id = format!("hooks:profile:{}", def.id);
            crate::native_hooks::preflight_apply(
                ctx,
                &sub(
                    req,
                    "hooks:apply",
                    vec![
                        "--only".into(),
                        target_id,
                        "--base".into(),
                        selection.target_base.clone(),
                        "--value".into(),
                        serde_json::to_string(&source)?,
                    ],
                ),
            )?;
            let path = if def.kind == ProfileKind::Claude {
                main
            } else {
                dir.join("hooks.json")
            };
            let target = file(files, &path)?;
            let root = target
                .content
                .as_deref()
                .map(serde_json::from_slice::<Value>)
                .transpose()?
                .unwrap_or_else(|| json!({}));
            let hooks = hooks_model::validate_hooks_map(Some(&source))?;
            let kind = if def.kind == ProfileKind::Claude {
                HookKind::Claude
            } else {
                HookKind::Codex
            };
            target.content = hooks_model::render_target(kind, Some(&root), &hooks);
            target.changed = true;
        }
        "rules" => {
            let source = domain_source(ctx, req, def, "rules", &selection.source_base)?;
            let body = source.as_str().ok_or_else(|| anyhow!("规则不是文本"))?;
            crate::native_rules::preflight_apply(
                ctx,
                &sub(
                    req,
                    "rules:apply",
                    vec![
                        "--only".into(),
                        format!("rules:profile:{}", def.id),
                        "--base".into(),
                        selection.target_base.clone(),
                        "--value".into(),
                        body.into(),
                    ],
                ),
            )?;
            let name = if def.kind == ProfileKind::Claude {
                "CLAUDE.md"
            } else {
                "AGENTS.md"
            };
            let target = file(files, &dir.join(name))?;
            target.content = Some(body.as_bytes().to_vec());
            target.changed = true;
        }
        item => {
            let name = item
                .strip_prefix("mcp:")
                .ok_or_else(|| anyhow!("不认识的共用设置项"))?;
            let path = if def.kind == ProfileKind::Claude {
                dir.join(".claude.json")
            } else {
                main
            };
            let current = file(files, &path)?.content.clone();
            let mut inv = mcp::inventory::load(ctx)?;
            inv.scans.retain(|scan| scan.target.agent != def.id);
            inv.scans.push(preview_seed::mcp_scan(def, &path, current)?);
            let parsed = ParsedAction {
                server: name.into(),
                action: Action::Copy,
                agent: Some(mcp::model::canonicalize_agent_id(&def.base_agent)),
                to: Some(def.id.clone()),
                spec: None,
                base: Some(selection.source_base.clone()),
                to_base: (selection.target_base != "missing")
                    .then(|| selection.target_base.clone()),
                replace: selection.target_base != "missing",
            };
            let plan = mcp::preflight::plan(&parsed, &inv)?;
            if plan.file_writes.len() != 1
                || plan.file_writes[0].path != path
                || !plan.saved_upserts.is_empty()
                || !plan.adoptions.is_empty()
            {
                bail!("MCP 复制未得到单一目标文件，请在 MCP 页处理");
            }
            let target = file(files, &path)?;
            target.content = Some(plan.file_writes.into_iter().next().unwrap().new_bytes);
            target.changed = true;
        }
    }
    Ok(())
}

pub(super) fn preview(ctx: &Context, req: &Request) -> Result<Value> {
    if req.args.len() != 4 || req.args[0] != "--only" || req.args[2] != "--selection" {
        bail!("profiles:shared-preview 只接受 --only profiles:<实例名> --selection <JSON>");
    }
    let id = req.args[1]
        .strip_prefix("profiles:")
        .filter(|id| super::model::valid_profile_id(id))
        .ok_or_else(|| anyhow!("需要 --only profiles:<实例名>"))?;
    let mut selections: Vec<Selection> = serde_json::from_str(&req.args[3])
        .map_err(|_| anyhow!("--selection 不是合法的共用设置列表"))?;
    if selections.is_empty() || selections.len() > 100 {
        bail!("请选 1 到 100 个共用设置项");
    }
    let mut seen = HashSet::new();
    if selections
        .iter()
        .any(|selection| !seen.insert(selection.item.as_str()))
    {
        bail!("共用设置项不能重复");
    }
    let def = seed::find_def(ctx, id)?;
    let dir = seed::config_dir(ctx, &def)?;
    let rows = shared::status(
        ctx,
        &sub(
            req,
            "profiles:shared-status",
            vec!["--only".into(), req.args[1].clone()],
        ),
    )?;
    let rows = rows["items"]
        .as_array()
        .ok_or_else(|| anyhow!("共用设置状态不可读"))?;
    for selection in &selections {
        let row = rows
            .iter()
            .find(|row| row["item"] == selection.item)
            .ok_or_else(|| anyhow!("来源没有这个共用设置项"))?;
        if row["source_base"] != selection.source_base
            || row["target_base"] != selection.target_base
        {
            bail!("来源或环境配置已变化，请刷新差异后重试");
        }
        if !matches!(row["state"].as_str(), Some("new" | "different")) {
            bail!("所选共用设置当前不可更新，请刷新差异");
        }
    }
    selections.sort_by_key(|selection| {
        rows.iter()
            .position(|row| row["item"] == selection.item)
            .unwrap()
    });
    let mut files = BTreeMap::new();
    let mut results = Vec::new();
    for selection in &selections {
        match render_one(ctx, req, &def, &dir, selection, &mut files) {
            Ok(()) => {
                results.push(json!({"item": selection.item, "state": "ready", "detail": "可更新"}))
            }
            Err(err) => results.push(json!({
                "item": selection.item,
                "state": "failed",
                "detail": crate::sanitize::sanitize_diagnostic_message(&format!("{err:#}")),
            })),
        }
    }
    let files = files.into_values().filter(|file| file.changed)
        .map(|file| {
            let bytes = file.content.ok_or_else(|| anyhow!("预览文件没有内容"))?;
            if bytes.len() > MAX_FILE_BYTES { bail!("配置文件超过安全读取上限"); }
            Ok(json!({
                "path": file.path,
                "exists": file.exists,
                "content": String::from_utf8(bytes).map_err(|_| anyhow!("预览结果不是 UTF-8 文本"))?,
            }))
        }).collect::<Result<Vec<_>>>()?;
    Ok(json!({"element": format!("profiles:{id}"), "results": results, "files": files}))
}
