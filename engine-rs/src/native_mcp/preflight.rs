//! mcp:apply 参数解析与写入计划（纯读）。
//!
//! 预检（锁前）与执行（锁内、Immediate 事务里重读文件后）共用 [`plan`]：同一份清单
//! 得出同一份计划，直接调用 apply 也绕不过这些校验。

use std::collections::HashSet;
use std::path::PathBuf;

use anyhow::{anyhow, bail, Result};
use serde_json::{Map, Value};

use super::entry::{
    self, apply_edit, apply_enabled, neutral_from_spec, neutral_from_view, parse_spec, render_new,
};
use super::inventory::{self, FileEntry, Inventory, TargetScan};
use super::model::{canonicalize_agent_id, is_sha256_hex, is_valid_server_name};
use super::saved::SavedRow;
use super::snapshot::{self, Adoptions};
use super::targets::Format;
use super::write::{self, Change};
use crate::native::{Context, Request};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Add,
    Edit,
    Enable,
    Disable,
    Remove,
    Copy,
    Forget,
}

impl Action {
    fn parse(raw: &str) -> Result<Self> {
        Ok(match raw {
            "add" => Action::Add,
            "edit" => Action::Edit,
            "enable" => Action::Enable,
            "disable" => Action::Disable,
            "remove" => Action::Remove,
            "copy" => Action::Copy,
            "forget" => Action::Forget,
            "update" | "upgrade" => bail!(
                "mcp:apply 已不再支持 '{raw}'：条目以各助手配置文件为准，请用 edit 直接修改该助手里的条目"
            ),
            other => bail!(
                "mcp:apply 不支持动作 '{other}'，仅支持 add/edit/enable/disable/remove/copy/forget"
            ),
        })
    }

    pub fn id(self) -> &'static str {
        match self {
            Action::Add => "add",
            Action::Edit => "edit",
            Action::Enable => "enable",
            Action::Disable => "disable",
            Action::Remove => "remove",
            Action::Copy => "copy",
            Action::Forget => "forget",
        }
    }
}

pub struct ParsedAction {
    pub server: String,
    pub action: Action,
    pub agent: Option<String>,
    pub to: Option<String>,
    pub spec: Option<String>,
    pub base: Option<String>,
    pub to_base: Option<String>,
    pub replace: bool,
}

pub fn parse_action_args(req: &Request) -> Result<ParsedAction> {
    let mut seen: HashSet<&str> = HashSet::new();
    let mut values: std::collections::HashMap<&str, String> = std::collections::HashMap::new();
    let mut replace = false;
    let mut i = 0;
    while i < req.args.len() {
        let token = req.args[i].as_str();
        if !token.starts_with("--") {
            bail!("意外的位置参数 '{}'", token);
        }
        if !seen.insert(token) {
            bail!("重复提供参数 '{}'", token);
        }
        match token {
            "--dry-run" => {
                bail!("mcp:apply 已退役 --dry-run，拒绝该参数（执行前请以 mcp:status 确认状态）")
            }
            "--allow-takeover" => {
                bail!("mcp:apply 已退役 --allow-takeover：条目以各助手配置文件为准，直接编辑即可")
            }
            "--offline" => i += 1,
            "--replace" => {
                replace = true;
                i += 1;
            }
            "--only" | "--action" | "--agent" | "--to" | "--spec" | "--base" | "--to-base" => {
                let value = req
                    .args
                    .get(i + 1)
                    .ok_or_else(|| anyhow!("参数 '{}' 缺少参数值", token))?;
                if value.starts_with("--") {
                    bail!("参数 '{}' 缺少参数值", token);
                }
                values.insert(token, value.clone());
                i += 2;
            }
            _ => bail!("未知选项 '{}'", token),
        }
    }
    let only = values
        .remove("--only")
        .ok_or_else(|| anyhow!("缺少 --only 参数"))?;
    let server = only
        .strip_prefix("mcp:")
        .ok_or_else(|| anyhow!("--only 只接受 mcp:<name>，得到 '{}'", only))?
        .to_string();
    if !is_valid_server_name(&server) {
        bail!(
            "非法 server 名称 '{}': 仅支持字母、数字、短横线与下划线",
            server
        );
    }
    let action = Action::parse(
        &values
            .remove("--action")
            .ok_or_else(|| anyhow!("缺少 --action 参数"))?,
    )?;
    let agent = values.remove("--agent").map(|a| canonicalize_agent_id(&a));
    let to = values.remove("--to").map(|a| canonicalize_agent_id(&a));
    let spec = values.remove("--spec");
    let base = values.remove("--base");
    let to_base = values.remove("--to-base");
    for (flag, digest) in [("--base", &base), ("--to-base", &to_base)] {
        if let Some(digest) = digest {
            if !is_sha256_hex(digest) {
                bail!("{flag} 必须是 64 位小写十六进制摘要");
            }
        }
    }

    let need = |cond: bool, what: &str| -> Result<()> {
        if cond {
            Ok(())
        } else {
            bail!("{} 动作{}", action.id(), what)
        }
    };
    let forbid = |present: bool, flag: &str| -> Result<()> {
        if present {
            bail!("{} 动作不接受 {}", action.id(), flag)
        }
        Ok(())
    };
    match action {
        Action::Forget => {
            forbid(agent.is_some(), "--agent")?;
            forbid(spec.is_some(), "--spec")?;
            forbid(base.is_some(), "--base")?;
        }
        _ => need(agent.is_some(), "需要 --agent")?,
    }
    match action {
        Action::Add => {
            need(spec.is_some(), "需要 --spec")?;
            forbid(base.is_some(), "--base")?;
        }
        Action::Edit => {
            need(spec.is_some(), "需要 --spec")?;
            need(base.is_some(), "需要 --base（打开编辑时条目的摘要）")?;
        }
        Action::Enable | Action::Disable | Action::Remove | Action::Copy => {
            forbid(spec.is_some(), "--spec")?;
            need(base.is_some(), "需要 --base（操作时条目的摘要）")?;
        }
        Action::Forget => {}
    }
    if action == Action::Copy {
        need(to.is_some(), "需要 --to")?;
        if to == agent {
            bail!("copy 的来源与目标是同一个助手");
        }
        if replace != to_base.is_some() {
            bail!("copy 覆盖同名条目需同时提供 --replace 与 --to-base（目标条目的摘要）");
        }
    } else {
        forbid(to.is_some(), "--to")?;
        forbid(replace, "--replace")?;
        forbid(to_base.is_some(), "--to-base")?;
    }
    Ok(ParsedAction {
        server,
        action,
        agent,
        to,
        spec,
        base,
        to_base,
        replace,
    })
}

pub struct FileWrite {
    pub path: PathBuf,
    pub old_bytes: Option<Vec<u8>>,
    pub new_bytes: Vec<u8>,
}

#[derive(Default)]
pub struct Plan {
    pub file_writes: Vec<FileWrite>,
    pub saved_upserts: Vec<SavedRow>,
    pub saved_deletes: Vec<(String, String)>,
    pub adoptions: Adoptions,
    pub warnings: Vec<String>,
}

fn stale(server: &str, agent: &str) -> anyhow::Error {
    anyhow!("条目 {server} 在助手 {agent} 的配置中已在外部修改，请刷新后重试")
}

/// 可写的目标：存在、文件可解析、助手未在本工具中停用。
fn operable<'a>(inv: &'a Inventory, agent: &str) -> Result<&'a TargetScan> {
    let scan = inv
        .scan(agent)
        .ok_or_else(|| anyhow!("未找到助手 '{}' 的 MCP 配置路径", agent))?;
    if let Some(error) = &scan.error {
        bail!("助手 '{}' 的 MCP 配置不可用：{}", agent, error);
    }
    if !scan.target.enabled {
        bail!("助手 '{}' 已在本工具中停用，拒绝修改其配置", agent);
    }
    Ok(scan)
}

fn raw_map(entry: &FileEntry) -> Result<&Map<String, Value>> {
    entry
        .raw
        .as_object()
        .ok_or_else(|| anyhow!("条目 {} 不是对象", entry.name))
}

fn valid_view<'a>(entry: &'a FileEntry) -> Result<&'a entry::View> {
    entry
        .view
        .as_ref()
        .map_err(|reason| anyhow!("条目 {} 结构无法识别（{}），只能删除", entry.name, reason))
}

fn push_write(plan: &mut Plan, scan: &TargetScan, server: &str, change: Change) -> Result<()> {
    let new_bytes = write::render(scan, server, change)?;
    if scan.old_bytes.as_deref() == Some(new_bytes.as_slice()) {
        return Ok(());
    }
    plan.file_writes.push(FileWrite {
        path: scan.target.path.clone(),
        old_bytes: scan.old_bytes.clone(),
        new_bytes,
    });
    Ok(())
}

/// 由清单与参数得出写入计划（纯读、不落盘）。
pub fn plan(parsed: &ParsedAction, inv: &Inventory) -> Result<Plan> {
    let server = parsed.server.as_str();
    let mut plan = Plan::default();
    if parsed.action == Action::Forget {
        if inv.saved_row(server, "").is_none() {
            bail!("没有名为 {server} 的已保存定义");
        }
        plan.saved_deletes.push((server.to_string(), String::new()));
        return Ok(plan);
    }
    let agent = parsed.agent.as_deref().expect("解析阶段已保证 --agent");
    let base = parsed.base.as_deref();
    let snapshot_row = inv.saved_row(server, agent);

    match parsed.action {
        Action::Add => {
            let scan = operable(inv, agent)?;
            if scan.entry(server).is_some() {
                bail!("助手 {agent} 的配置里已有名为 {server} 的条目");
            }
            if snapshot_row.is_some() {
                bail!("助手 {agent} 里已有停用的同名条目 {server}，请先启用或删除它");
            }
            let spec = parse_spec(parsed.spec.as_deref().unwrap_or_default())?;
            if spec.startup_timeout_sec.is_some() && scan.target.format != Format::CodexToml {
                bail!("startup_timeout_sec 只适用于 Codex");
            }
            let neutral = neutral_from_spec(&spec)?;
            let new = render_new(&scan.target.format, &neutral)?;
            push_write(&mut plan, scan, server, Change::Set(&new))?;
        }
        Action::Edit => {
            let scan = operable(inv, agent)?;
            let Some(file_entry) = scan.entry(server) else {
                if snapshot_row.is_some() {
                    bail!("{server} 在助手 {agent} 中已停用（保存在本工具里），请先启用再编辑");
                }
                bail!("助手 {agent} 的配置里没有名为 {server} 的条目");
            };
            if Some(file_entry.base.as_str()) != base {
                return Err(stale(server, agent));
            }
            let view = valid_view(file_entry)?;
            let spec = parse_spec(parsed.spec.as_deref().unwrap_or_default())?;
            let new = apply_edit(&scan.target.format, raw_map(file_entry)?, view, &spec)?;
            push_write(&mut plan, scan, server, Change::Set(&new))?;
        }
        Action::Disable => {
            let scan = operable(inv, agent)?;
            let Some(file_entry) = scan.entry(server) else {
                if snapshot_row.is_some() {
                    bail!("{server} 在助手 {agent} 中已处于停用状态");
                }
                bail!("助手 {agent} 的配置里没有名为 {server} 的条目");
            };
            if Some(file_entry.base.as_str()) != base {
                return Err(stale(server, agent));
            }
            let view = valid_view(file_entry)?;
            match scan.target.format {
                Format::ClaudeJson => {
                    let (row, adoptions) =
                        snapshot::from_claude_entry(server, agent, raw_map(file_entry)?, view)?;
                    if !adoptions.is_empty() {
                        let names: Vec<&str> = adoptions.keys().map(String::as_str).collect();
                        plan.warnings.push(format!(
                            "已把 {} 个疑似凭据的明文取值转存到外部密钥文件：{}",
                            names.len(),
                            names.join(", ")
                        ));
                    }
                    plan.adoptions = adoptions;
                    push_write(&mut plan, scan, server, Change::Remove)?;
                    plan.saved_upserts.push(row);
                }
                _ => {
                    if !view.enabled {
                        bail!("{server} 在助手 {agent} 中已处于停用状态");
                    }
                    let new = apply_enabled(&scan.target.format, raw_map(file_entry)?, false);
                    push_write(&mut plan, scan, server, Change::Set(&new))?;
                }
            }
        }
        Action::Enable => {
            let scan = operable(inv, agent)?;
            if let Some(file_entry) = scan.entry(server) {
                if let Some(row) = snapshot_row {
                    if Some(row.base().as_str()) == base {
                        bail!("助手 {agent} 的配置里已存在同名条目 {server}，无法启用保存的旧内容：请先删除或改名现有条目");
                    }
                }
                if Some(file_entry.base.as_str()) != base {
                    return Err(stale(server, agent));
                }
                let view = valid_view(file_entry)?;
                if view.enabled {
                    bail!("{server} 在助手 {agent} 中已处于启用状态");
                }
                let new = apply_enabled(&scan.target.format, raw_map(file_entry)?, true);
                push_write(&mut plan, scan, server, Change::Set(&new))?;
            } else {
                let row = snapshot_row
                    .ok_or_else(|| anyhow!("助手 {agent} 里没有名为 {server} 的条目"))?;
                if Some(row.base().as_str()) != base {
                    return Err(stale(server, agent));
                }
                super::saved::validate_row(row)?;
                let restored = snapshot::restore(row)?;
                entry::parse(&scan.target.format, &Value::Object(restored.clone()))
                    .map_err(|e| anyhow!("保存的内容无法写回 {agent}：{e}"))?;
                push_write(&mut plan, scan, server, Change::Set(&restored))?;
                plan.saved_deletes
                    .push((server.to_string(), agent.to_string()));
            }
        }
        Action::Remove => {
            let file_entry = inv.scan(agent).and_then(|s| s.entry(server));
            if let Some(file_entry) = file_entry {
                let scan = operable(inv, agent)?;
                if Some(file_entry.base.as_str()) != base {
                    return Err(stale(server, agent));
                }
                push_write(&mut plan, scan, server, Change::Remove)?;
                if snapshot_row.is_some() {
                    plan.saved_deletes
                        .push((server.to_string(), agent.to_string()));
                }
            } else if let Some(row) = snapshot_row {
                if let Some(scan) = inv.scan(agent) {
                    if !scan.target.enabled {
                        bail!("助手 '{}' 已在本工具中停用，拒绝修改其配置", agent);
                    }
                }
                if Some(row.base().as_str()) != base {
                    return Err(stale(server, agent));
                }
                plan.saved_deletes
                    .push((server.to_string(), agent.to_string()));
            } else {
                bail!("助手 {agent} 里没有名为 {server} 的条目");
            }
        }
        Action::Copy => {
            let to = parsed.to.as_deref().expect("解析阶段已保证 --to");
            let source_scan = inv.scan(agent);
            let (neutral, view) =
                if let Some(file_entry) = source_scan.and_then(|s| s.entry(server)) {
                    if Some(file_entry.base.as_str()) != base {
                        return Err(stale(server, agent));
                    }
                    let format = &source_scan.expect("有条目必有扫描").target.format;
                    let view = valid_view(file_entry)?.clone();
                    (neutral_from_view(format, &view), view)
                } else if let Some(row) = snapshot_row {
                    if Some(row.base().as_str()) != base {
                        return Err(stale(server, agent));
                    }
                    let format = source_scan
                        .map(|s| s.target.format.clone())
                        .ok_or_else(|| anyhow!("来源助手 {agent} 已没有 MCP 配置目标"))?;
                    super::saved::validate_row(row)?;
                    snapshot::neutral(&format, row)?
                } else {
                    bail!("助手 {agent} 里没有名为 {server} 的条目");
                };
            let target = operable(inv, to)?;
            let existing = target.entry(server);
            let target_row = inv.saved_row(server, to);
            if existing.is_some() || target_row.is_some() {
                if !parsed.replace {
                    bail!("助手 {to} 已有同名条目 {server}；如需覆盖，请确认替换（目标条目的其他字段不会保留）");
                }
                let current = existing
                    .map(|e| e.base.clone())
                    .or_else(|| target_row.map(SavedRow::base));
                if current.as_deref() != parsed.to_base.as_deref() {
                    return Err(stale(server, to));
                }
            }
            let new = render_new(&target.target.format, &neutral)?;
            push_write(&mut plan, target, server, Change::Set(&new))?;
            if target_row.is_some() {
                plan.saved_deletes
                    .push((server.to_string(), to.to_string()));
            }
            let dropped = entry::dropped_on_copy(&view, &target.target.format);
            if !dropped.is_empty() {
                plan.warnings.push(format!(
                    "复制到 {to} 时未带过去的字段：{}",
                    dropped.join(", ")
                ));
            }
        }
        Action::Forget => unreachable!(),
    }
    Ok(plan)
}

/// 锁前预检：严格解析 + 只读清单 + 计划。任何不通过都拒绝，且不建数据目录、库或文件。
pub fn preflight_apply(ctx: &Context, req: &Request) -> Result<()> {
    let parsed = parse_action_args(req)?;
    let inv = inventory::load(ctx)?;
    plan(&parsed, &inv).map(|_| ())
}
