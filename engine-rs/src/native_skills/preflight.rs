use anyhow::{anyhow, bail, Result};
use std::collections::BTreeMap;
use std::path::PathBuf;

use super::paths::{
    check_not_external_conflict, check_not_external_conflict_with_takeover,
    check_shared_agent_target, collection_source_root, expected_link_target, is_link_entry,
    is_local_source, managed_bases, parse_only, validate_local_source,
};
use super::persistence::load_bindings_readonly;
use super::status::{load_metadata, DomainMetadata};
use super::{CollectionDef, COLLECTION_ACTIONS, SKILL_ACTIONS};
use crate::native::{Context, Request};

/// 锁前预检：参数严格校验（含已退役的 --dry-run 拒绝）+ 纯读状态校验。
/// Ok(()) 表示合法 apply（execute 稍后必须取得全局 apply lock）；参数无效或
/// 状态阻塞即 Err。无效/阻塞预检不创建 data/home/DB/表/文件。
pub fn preflight_apply(ctx: &Context, req: &Request) -> Result<()> {
    if req.command.replace('.', ":") == "skills:adopt" {
        return super::adopt::preflight(ctx, req);
    }
    if req.flag("--dry-run") {
        bail!("skills:apply 已退役 --dry-run，拒绝该参数（执行前请以 skills:status 确认状态）");
    }
    let action = req
        .value("--action")
        .ok_or_else(|| anyhow!("缺少 --action 参数"))?;
    if action == "remove-link" {
        return preflight_remove_link(ctx, req);
    }
    let only = req
        .value("--only")
        .ok_or_else(|| anyhow!("缺少 --only 参数"))?;
    let target = parse_only(only)?;
    // 显式接管通道：仅逐 skill 的 add/remove 可携带，其余动作与集合级一律拒绝。
    if req.flag("--allow-takeover")
        && !(target.skill.is_some() && matches!(action, "add" | "remove"))
    {
        bail!("--allow-takeover 仅适用于逐 skill 的 add/remove 动作");
    }
    let meta = load_metadata(ctx)?;

    let fallback_coll = CollectionDef {
        name: target.collection.clone(),
        url: None,
        source_path: None,
        skill_paths: None,
        agents: None,
        disabled: None,
        enabled: false,
        deleted: true,
        raw: serde_json::Map::new(),
    };
    let coll = match meta.collections.get(&target.collection) {
        Some(c) => c,
        None => {
            if target.skill.is_none() && action == "remove" {
                let bindings = load_bindings_readonly(ctx)?;
                if bindings
                    .iter()
                    .any(|((c, _, _), _)| c == &target.collection)
                {
                    &fallback_coll
                } else {
                    bail!("未知 skill 集合: {}", target.collection);
                }
            } else {
                bail!("未知 skill 集合: {}", target.collection);
            }
        }
    };

    if coll.deleted && action != "remove" {
        bail!(
            "[{}] 集合已被删除（tombstone），拒绝 {}；如需重建请先保存定义",
            target.collection,
            action
        );
    }

    let bases = managed_bases(ctx, &meta.collections);
    let agent_targets: BTreeMap<String, PathBuf> = meta
        .agents
        .iter()
        .map(|(k, v)| (k.clone(), v.skills_dir.clone()))
        .collect();

    match &target.skill {
        Some(skill) => {
            preflight_skill_action(ctx, req, coll, &meta, &bases, &agent_targets, skill, action)?
        }
        None => preflight_coll_action(ctx, req, coll, &meta, &bases, &agent_targets, action)?,
    }

    Ok(())
}

/// 清理孤立链接的锁前预检：全程零写，只做名称校验与目标形态判定。
fn preflight_remove_link(ctx: &Context, req: &Request) -> Result<()> {
    let agent = req
        .value("--agent")
        .ok_or_else(|| anyhow!("清理链接必须指定 --agent"))?;
    let link = req
        .value("--link")
        .ok_or_else(|| anyhow!("清理链接必须用 --link 指名"))?;
    if req.value("--only").is_some() {
        bail!("清理链接不接受 --only（它不属于任何集合）");
    }
    super::paths::validate_path_identifier(agent, "Agent 名称")?;
    super::paths::validate_path_identifier(link, "链接名称")?;
    let meta = load_metadata(ctx)?;
    let agent_info = meta
        .agents
        .get(agent)
        .ok_or_else(|| anyhow!("未知 agent: {}", agent))?;
    let link_path = agent_info.skills_dir.join(link);
    if !is_link_entry(&link_path) {
        if link_path.exists() {
            bail!("[{}@{}] 目标是真实文件或目录，拒绝删除", link, agent);
        }
        bail!("[{}@{}] 目标不存在，无需清理", link, agent);
    }
    let bindings = load_bindings_readonly(ctx)?;
    if bindings.iter().any(|((_, skill, bound_agent), binding)| {
        skill == link && bound_agent == agent && binding.assigned
    }) {
        bail!(
            "[{}@{}] 这条链接有分配记录，请在它所属集合里移除，不从清理入口绕过",
            link,
            agent
        );
    }
    Ok(())
}

fn preflight_skill_action(
    ctx: &Context,
    req: &Request,
    coll: &CollectionDef,
    meta: &DomainMetadata,
    bases: &[PathBuf],
    agent_targets: &BTreeMap<String, PathBuf>,
    skill: &str,
    action: &str,
) -> Result<()> {
    if !SKILL_ACTIONS.contains(&action) {
        bail!("不支持的技能动作: {}", action);
    }
    let agent = req
        .value("--agent")
        .ok_or_else(|| anyhow!("逐 skill 动作必须指定 --agent"))?;
    super::paths::validate_path_identifier(agent, "Agent 名称")?;
    let agent_info = meta
        .agents
        .get(agent)
        .ok_or_else(|| anyhow!("未知 agent: {}", agent))?;

    check_shared_agent_target(agent, agent_targets)?;

    // 本地来源集合先做源目录安全校验（存在、非链接、不越界、不与助手技能目录重叠）。
    if is_local_source(coll) && matches!(action, "add" | "update" | "enable") {
        validate_local_source(ctx, coll, agent_targets)?;
    }
    let link_path = agent_info.skills_dir.join(skill);
    // 逐 skill add/remove 可携带 --allow-takeover 放行非受管符号链接（其余动作该旗标
    // 已在入口被拒）；真实文件/目录与缺旗标的非受管链接仍 fail-closed。
    check_not_external_conflict_with_takeover(&link_path, bases, req.flag("--allow-takeover"))?;

    if (action == "add" || action == "enable") && !coll.enabled {
        bail!(
            "[{}] 集合已全局停用，拒绝启用/新增下属技能分配（请先启用集合）",
            coll.name
        );
    }
    if (action == "add" || action == "enable") && !agent_info.enabled {
        bail!("Agent '{}' 已停用，拒绝启用/新增技能分配", agent);
    }
    if let Some(ref allowed) = coll.agents {
        if !allowed.iter().any(|a| a == agent) {
            bail!(
                "[{}/{}@{}] Agent '{}' 不在集合允许的助手范围内",
                coll.name,
                skill,
                agent,
                agent
            );
        }
    }
    if let Some(ref disabled) = coll.disabled {
        if disabled.iter().any(|d| d == skill) && (action == "add" || action == "enable") {
            bail!("[{}/{}] skill 已被集合全局禁用，拒绝操作", coll.name, skill);
        }
    }

    let bindings = load_bindings_readonly(ctx)?;
    let existing_binding = bindings.get(&(coll.name.clone(), skill.to_string(), agent.to_string()));
    preflight_verify_skill_binding(ctx, coll, action, skill, agent, existing_binding)?;
    Ok(())
}

fn preflight_verify_skill_binding(
    ctx: &Context,
    coll: &CollectionDef,
    action: &str,
    skill: &str,
    agent: &str,
    existing_binding: Option<&super::BindingEntry>,
) -> Result<()> {
    let coll_name = &coll.name;
    match action {
        "enable" => {
            let b = existing_binding.ok_or_else(|| {
                anyhow!("[{coll_name}/{skill}@{agent}] 无既有分配，无法启用（请先 add）")
            })?;
            if !b.assigned {
                bail!("[{coll_name}/{skill}@{agent}] 分配已被移除，无法启用");
            }
            expected_link_target(ctx, coll, skill, &b.version)?;
        }
        "add" => {
            if let Some(b) = existing_binding {
                if b.assigned {
                    bail!("[{coll_name}/{skill}@{agent}] 分配已存在，如需更新请使用 update");
                }
            }
        }
        "update" => {
            let b = existing_binding.ok_or_else(|| {
                anyhow!("[{coll_name}/{skill}@{agent}] 无既有分配，无法更新（请先 add）")
            })?;
            if !b.assigned {
                bail!("[{coll_name}/{skill}@{agent}] 分配已被移除，如需恢复请使用 add");
            }
        }
        "disable" => {
            let b = existing_binding
                .ok_or_else(|| anyhow!("[{coll_name}/{skill}@{agent}] 无既有分配，无法停用"))?;
            if !b.assigned {
                bail!("[{coll_name}/{skill}@{agent}] 分配已被移除，无法停用");
            }
        }
        _ => {}
    }
    Ok(())
}

fn preflight_coll_action(
    ctx: &Context,
    req: &Request,
    coll: &CollectionDef,
    meta: &DomainMetadata,
    bases: &[PathBuf],
    agent_targets: &BTreeMap<String, PathBuf>,
    action: &str,
) -> Result<()> {
    if !COLLECTION_ACTIONS.contains(&action) {
        bail!("不支持的集合动作: {}", action);
    }
    if req.value("--agent").is_some() {
        bail!("集合动作不接受 --agent");
    }
    if let Some(reference) = req.value("--ref") {
        if action != "update" || is_local_source(coll) {
            bail!("--ref 只用于 git 来源的更新（切换到指定版本）");
        }
        if req.flag("--offline") {
            bail!("--ref 需要联网取远端版本，不能与 --offline 同用");
        }
        if !super::track::valid_ref_name(reference) || reference.contains('*') {
            bail!("--ref 不合法：{reference}");
        }
    } else if req.flag("--ref") {
        bail!("--ref 缺少取值");
    }
    if action == "update" {
        // 跟踪方式写错要在动手前报出来
        if !is_local_source(coll) {
            super::versioning::track_of(coll)?;
            super::versioning::sparse_patterns(coll)?;
        }
        if is_local_source(coll) {
            // 本地来源集合无远端可拉，只校验登记源目录仍然安全可用。
            validate_local_source(ctx, coll, agent_targets)?;
        } else {
            if coll.url.is_none() || coll.url.as_deref() == Some("") {
                bail!("[{}] 集合定义缺少 url，无法刷新资源库", coll.name);
            }
            let repo_dir = collection_source_root(ctx, coll);
            if req.flag("--offline") && !repo_dir.exists() {
                bail!("[{}] 离线模式且本地仓库缺失，无法下载", coll.name);
            }
        }
    }
    let bindings = load_bindings_readonly(ctx)?;
    match action {
        "enable" => {
            for ((c, s, a), b) in &bindings {
                if c == &coll.name && b.assigned && b.enabled {
                    if let Some(agent_info) = meta.agents.get(a) {
                        if agent_info.enabled {
                            let link_path = agent_info.skills_dir.join(s);
                            check_not_external_conflict(&link_path, bases)?;
                            expected_link_target(ctx, coll, s, &b.version)?;
                        }
                    }
                }
            }
        }
        "disable" | "remove" => {
            for ((c, s, a), b) in &bindings {
                if c == &coll.name && b.assigned {
                    if let Some(agent_info) = meta.agents.get(a) {
                        let link_path = agent_info.skills_dir.join(s);
                        if link_path.exists() || is_link_entry(&link_path) {
                            check_not_external_conflict(&link_path, bases)?;
                        }
                    }
                }
            }
        }
        _ => {}
    }
    for agent in meta.agents.keys() {
        check_shared_agent_target(agent, agent_targets)?;
    }
    Ok(())
}
