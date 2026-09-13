//! MCP 参数解析与前置纯读校验

use std::collections::HashSet;
use std::path::Path;

use anyhow::{anyhow, bail, Result};
use serde_json::Value;

use super::model::{
    canonicalize_agent_id, is_valid_server_name, Bindings, AGENT_ACTIONS, LIBRARY_ACTIONS,
};
use super::persistence::load_bindings_readonly;
use super::policy_view::{
    allowed_agents, expected_extra_fields, is_allowed, library_enabled, library_view, resolve_env,
    resolve_header_vars, validate_definition, validate_definition_env,
};
use super::targets::{
    agent_targets, check_shared_targets, check_symlink_metadata, extract_entry,
    validate_target_path, Format,
};
use crate::native::{Context, Request};
use crate::store;

pub struct ParsedAction {
    pub server: String,
    pub agent: Option<String>,
    pub action: String,
    pub dry_run: bool,
    pub offline: bool,
    /// 显式接管通道：仅逐 agent add/update 可用，允许用受管渲染结果替换
    /// 目标文件中的未受管同名条目；缺省保持 fail-closed 不变。
    pub allow_takeover: bool,
}

pub fn parse_action_args(req: &Request) -> Result<ParsedAction> {
    let mut seen_flags: HashSet<&str> = HashSet::new();
    let mut only: Option<String> = None;
    let mut action: Option<String> = None;
    let mut agent: Option<String> = None;
    let mut dry_run = false;
    let mut offline = false;
    let mut allow_takeover = false;

    let mut i = 0;
    while i < req.args.len() {
        let token = &req.args[i];
        if !token.starts_with("--") {
            bail!("意外的位置参数 '{}'", token);
        }
        let flag = token.as_str();
        if !flag.starts_with("--") {
            bail!("意外的位置参数 '{}'", flag);
        }
        if !seen_flags.insert(flag) {
            bail!("重复提供参数 '{}'", flag);
        }
        match flag {
            "--dry-run" => {
                dry_run = true;
                i += 1;
            }
            "--offline" => {
                offline = true;
                i += 1;
            }
            "--allow-takeover" => {
                allow_takeover = true;
                i += 1;
            }
            "--only" | "--action" | "--agent" => {
                i += 1;
                let val = req
                    .args
                    .get(i)
                    .ok_or_else(|| anyhow!("参数 '{}' 缺少参数值", flag))?;
                if val.starts_with("--") {
                    bail!("参数 '{}' 缺少参数值", flag);
                }
                match flag {
                    "--only" => only = Some(val.clone()),
                    "--action" => action = Some(val.clone()),
                    "--agent" => agent = Some(canonicalize_agent_id(val)),
                    _ => unreachable!(),
                }
                i += 1;
            }
            _ => bail!("未知选项 '{}'", flag),
        }
    }

    let only_val = only.ok_or_else(|| anyhow!("缺少 --only 参数"))?;
    let server = only_val
        .strip_prefix("mcp:")
        .ok_or_else(|| anyhow!("--only 只接受 mcp:<name>，得到 '{}'", only_val))?
        .to_string();
    if !is_valid_server_name(&server) {
        bail!(
            "非法 server 名称 '{}': 仅支持字母、数字、短横线与下划线",
            server
        );
    }
    let act = action.ok_or_else(|| anyhow!("缺少 --action 参数"))?;

    if let Some(ref _a) = agent {
        if !AGENT_ACTIONS.contains(&act.as_str()) {
            bail!(
                "逐 agent 动作不支持 '{}'，仅支持 add/update/enable/disable/remove",
                act
            );
        }
    } else if !LIBRARY_ACTIONS.contains(&act.as_str()) {
        bail!(
            "库级动作不支持 '{}'，仅支持 enable/disable/remove（逐 agent 动作需指定 --agent）",
            act
        );
    }

    if allow_takeover && !(agent.is_some() && matches!(act.as_str(), "add" | "update")) {
        bail!("--allow-takeover 仅适用于逐 agent 的 add/update 动作");
    }

    Ok(ParsedAction {
        server,
        agent,
        action: act,
        dry_run,
        offline,
        allow_takeover,
    })
}

pub fn preflight_apply(ctx: &Context, req: &Request) -> Result<bool> {
    let parsed = parse_action_args(req)?;
    let policy = store::effective_policy(ctx)?;
    let bindings = load_bindings_readonly(ctx)?;
    preflight_check_state(&parsed, &policy, &ctx.home_dir, &bindings)?;
    Ok(!parsed.dry_run)
}

/// 与 preflight 等价的状态校验，直接作用于调用方提供的最新
/// policy/targets/bindings。`apply` 在 Immediate 事务内读出最新状态后、
/// 任何文件/DB 写入前必须调用本函数，保证直接调用 apply 也无法绕过约束。
pub fn preflight_check_state(
    parsed: &ParsedAction,
    policy: &Value,
    home_dir: &Path,
    bindings: &Bindings,
) -> Result<()> {
    let library = library_view(policy);
    let (targets, disabled) = agent_targets(policy, home_dir)?;
    check_shared_targets(&targets)?;

    let def_opt = library.definitions.get(&parsed.server);

    if let Some(ref a) = parsed.agent {
        if disabled.contains(a) {
            bail!("Agent '{}' 已被禁用（在策略中停用），拒绝操作", a);
        }
        let target = targets
            .iter()
            .find(|t| &t.agent == a)
            .ok_or_else(|| anyhow!("未找到 Agent '{}' 的 MCP 配置路径", a))?;
        if let Format::Unsupported(ref fmt) = target.format {
            bail!("Agent '{}' 的 MCP 配置格式 '{}' 不支持", a, fmt);
        }
        validate_target_path(&target.path)?;

        let (entry_opt, is_managed) =
            extract_entry(&target.path, target.format.clone(), &parsed.server)?;
        // 显式接管：仅 --allow-takeover 的 add/update 允许替换未受管同名条目；
        // 该条目将被受管渲染整体替换，其外部字段随条目一并被替换，不再逐字段拦截。
        let takeover = parsed.allow_takeover
            && matches!(parsed.action.as_str(), "add" | "update")
            && entry_opt.is_some()
            && !is_managed;
        if let Some(ref entry) = entry_opt {
            if !is_managed && !takeover {
                bail!(
                    "目标文件 {} 中已存在非受管同名条目 '{}'，拒绝覆盖以防止冲突",
                    target.path.display(),
                    parsed.server
                );
            }
            if !takeover {
                if let Some(def) = def_opt {
                    let expected_extra =
                        expected_extra_fields(def, target.format.clone(), &parsed.server)?;
                    for (k, _) in &entry.extra {
                        if !expected_extra.contains_key(k) {
                            bail!(
                                "目标文件 {} 中的条目 '{}' 包含未受管外部属性（外部未识别字段 '{}'），拒绝覆盖以防止外部配置丢失",
                                target.path.display(),
                                parsed.server,
                                k
                            );
                        }
                    }
                }
            }
        }

        let binding = bindings.get(&parsed.server).and_then(|m| m.get(a));
        match parsed.action.as_str() {
            "add" => {
                let def = def_opt.ok_or_else(|| anyhow!("服务定义 '{}' 不存在", parsed.server))?;
                validate_definition(def, &parsed.server)?;
                validate_definition_env(def)?;
                let allowed = allowed_agents(def)?;
                if !is_allowed(&allowed, a) {
                    bail!("Agent '{}' 不在服务 '{}' 的允许列表中", a, parsed.server);
                }
                if binding.map_or(false, |b| b.assigned) {
                    bail!(
                        "服务 '{}' 已在 Agent '{}' 中分配，add 不适用",
                        parsed.server,
                        a
                    );
                }
                if entry_opt.is_some() && is_managed {
                    bail!(
                        "目标文件 {} 中已存在残留受管条目 '{}'，add 不适用，请先执行 remove 清理",
                        target.path.display(),
                        parsed.server
                    );
                }
                resolve_env(def, &parsed.server)?;
                resolve_header_vars(def, &parsed.server)?;
            }
            "update" => {
                let def = def_opt.ok_or_else(|| anyhow!("服务定义 '{}' 不存在", parsed.server))?;
                validate_definition(def, &parsed.server)?;
                validate_definition_env(def)?;
                let allowed = allowed_agents(def)?;
                if !is_allowed(&allowed, a) {
                    bail!("Agent '{}' 不在服务 '{}' 的允许列表中", a, parsed.server);
                }
                if !binding.map_or(false, |b| b.assigned) {
                    bail!(
                        "服务 '{}' 尚未在 Agent '{}' 中分配，update 不适用",
                        parsed.server,
                        a
                    );
                }
                resolve_env(def, &parsed.server)?;
                resolve_header_vars(def, &parsed.server)?;
            }
            "enable" => {
                let b = binding.filter(|b| b.assigned).ok_or_else(|| {
                    anyhow!(
                        "服务 '{}' 尚未在 Agent '{}' 中分配，enable 不适用",
                        parsed.server,
                        a
                    )
                })?;
                if b.enabled {
                    bail!(
                        "服务 '{}' 在 Agent '{}' 中已处于启用状态，enable 不适用",
                        parsed.server,
                        a
                    );
                }
                if entry_opt.is_some() && is_managed {
                    bail!(
                        "目标文件 {} 中已残留受管配置，enable 不适用，请先执行 disable 同步或 remove",
                        target.path.display()
                    );
                }
                let def = def_opt.unwrap_or(&b.definition);
                validate_definition(def, &parsed.server)?;
                validate_definition_env(def)?;
                let allowed = allowed_agents(def)?;
                if !is_allowed(&allowed, a) {
                    bail!(
                        "Agent '{}' 不在服务 '{}' 的允许列表中，无法启用",
                        a,
                        parsed.server
                    );
                }
                resolve_env(&b.definition, &parsed.server)?;
                resolve_header_vars(&b.definition, &parsed.server)?;
            }
            "disable" => {
                let b = binding.filter(|b| b.assigned).ok_or_else(|| {
                    anyhow!(
                        "服务 '{}' 尚未在 Agent '{}' 中分配，disable 不适用",
                        parsed.server,
                        a
                    )
                })?;
                if !b.enabled && (entry_opt.is_none() || !is_managed) {
                    bail!(
                        "服务 '{}' 在 Agent '{}' 中已处于停用状态，disable 不适用",
                        parsed.server,
                        a
                    );
                }
            }
            "remove" => {
                let is_assigned = binding.map_or(false, |b| b.assigned);
                if !is_assigned && entry_opt.is_none() {
                    bail!(
                        "服务 '{}' 在 Agent '{}' 中不存在分配或受管条目，remove 不适用",
                        parsed.server,
                        a
                    );
                }
            }
            _ => unreachable!(),
        }
    } else {
        match parsed.action.as_str() {
            "enable" => {
                def_opt.ok_or_else(|| anyhow!("服务定义 '{}' 不存在", parsed.server))?;
                if library_enabled(&library, &parsed.server) {
                    bail!("服务 '{}' 已处于启用状态，enable 不适用", parsed.server);
                }
            }
            "disable" => {
                def_opt.ok_or_else(|| anyhow!("服务定义 '{}' 不存在", parsed.server))?;
                if !library_enabled(&library, &parsed.server) {
                    bail!("服务 '{}' 已处于停用状态，disable 不适用", parsed.server);
                }
            }
            "remove" => {
                let def_exists = def_opt.is_some();
                let binding_exists = bindings
                    .get(&parsed.server)
                    .map_or(false, |m| !m.is_empty());
                let mut disk_exists = false;
                for target in &targets {
                    let has_binding = bindings
                        .get(&parsed.server)
                        .and_then(|m| m.get(&target.agent))
                        .is_some();
                    let target_exists = check_symlink_metadata(&target.path)?.is_some();
                    if target_exists {
                        validate_target_path(&target.path)?;
                    }

                    if let Format::Unsupported(ref fmt) = target.format {
                        if has_binding || target_exists {
                            bail!(
                                "目标 Agent '{}' 的配置格式 '{}' 不受支持，无法执行安全清理",
                                target.agent,
                                fmt
                            );
                        }
                        continue;
                    }
                    if !target_exists {
                        continue;
                    }
                    let (entry_opt, is_managed) =
                        extract_entry(&target.path, target.format.clone(), &parsed.server)?;
                    if let Some(ref entry) = entry_opt {
                        if !is_managed {
                            bail!(
                                "目标文件 {} 中已存在非受管同名条目 '{}'，拒绝自动删除以防止冲突",
                                target.path.display(),
                                parsed.server
                            );
                        }
                        if let Some(def) = def_opt {
                            let expected_extra =
                                expected_extra_fields(def, target.format.clone(), &parsed.server)?;
                            for (k, _) in &entry.extra {
                                if !expected_extra.contains_key(k) {
                                    bail!(
                                        "目标文件 {} 中的条目 '{}' 包含未受管外部属性（外部未识别字段 '{}'），拒绝删除以防止外部配置丢失",
                                        target.path.display(),
                                        parsed.server,
                                        k
                                    );
                                }
                            }
                        }
                        disk_exists = true;
                    }
                }
                if !def_exists && !binding_exists && !disk_exists {
                    bail!("服务 '{}' 不存在，remove 不适用", parsed.server);
                }
            }
            _ => unreachable!(),
        }
    }

    Ok(())
}
