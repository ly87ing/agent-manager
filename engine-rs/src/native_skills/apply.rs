use anyhow::{anyhow, bail, Context as _, Result};
use rusqlite::TransactionBehavior;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

use super::paths::{
    atomic_create_or_replace_link, check_not_external_conflict,
    check_not_external_conflict_with_takeover, collection_source_root, expected_link_target,
    is_link_entry, is_local_source, is_unmanaged_link, managed_bases, parse_only, repo_base_dir,
    resolve_link_target, resolve_loose, safe_remove_link, safe_remove_link_with_takeover,
    validate_local_source, ExpectedLinkState, RollbackGuard,
};
use super::persistence::{
    delete_binding, load_bindings_from_conn, load_bindings_readonly, write_binding,
};
use super::snapshot::{discover_repo_skills, hash_skill_dir};
use super::status::{load_metadata, record_apply_operation, DomainMetadata};
use super::CollectionDef;
use crate::native::{Context, Request};
use crate::store;

pub fn apply(ctx: &Context, req: &Request) -> Result<Value> {
    // 参数严格校验是所有写路径的第一步：--dry-run 已退役，一律拒绝。
    if req.flag("--dry-run") {
        bail!("skills:apply 已退役 --dry-run，拒绝该参数（执行前请以 skills:status 确认状态）");
    }
    let action = req
        .value("--action")
        .ok_or_else(|| anyhow!("缺少 --action 参数"))?;
    // 清理孤立链接：不属于任何集合，因此不要 --only，用 --link 指名
    if action == "remove-link" {
        super::preflight::validate_replace_options(req, action, false)?;
        apply_remove_link(ctx, req)?;
        let link = req.value("--link").unwrap_or("");
        return Ok(json!({
            "domain": "skills",
            "steps": [{"name": format!("skills:remove-link {}", link), "ok": true, "exit_code": 0}],
            "skipped": [],
        }));
    }
    let only = req
        .value("--only")
        .ok_or_else(|| anyhow!("缺少 --only 参数"))?;
    let target = parse_only(only)?;
    super::preflight::validate_replace_options(req, action, target.skill.is_some())?;
    // 显式接管通道：仅逐 skill 的 add/remove 可携带，其余动作与集合级一律拒绝。
    if req.flag("--allow-takeover")
        && !(target.skill.is_some() && matches!(action, "add" | "remove"))
    {
        bail!("--allow-takeover 仅适用于逐 skill 的 add/remove 动作");
    }

    let result = (|| -> Result<Value> {
        let meta = load_metadata(ctx)?;
        let fallback_coll = super::orphaned_collection(&target.collection);
        let coll = match meta.collections.get(&target.collection) {
            Some(c) => c,
            None => {
                if action == "remove" {
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

        match &target.skill {
            Some(skill) => apply_single_skill(ctx, req, coll, &meta, skill, action)?,
            None => apply_collection(ctx, req, coll, &meta, action)?,
        }

        Ok(json!({
            "domain": "skills",
            "steps": [{"name": format!("skills:{} {}", action, only), "ok": true, "exit_code": 0}],
            "skipped": [],
        }))
    })();

    match result {
        Ok(val) => Ok(val),
        Err(err) => {
            let _ = record_apply_operation(ctx, req, only, action, false);
            Err(err)
        }
    }
}

/// 清理助手技能目录里的孤立链接：断链或旧体系残留，不属于任何集合、也没有分配记录。
///
/// 只删除符号链接本身，绝不跟随链接删源；真实文件/目录一律 fail-closed；
/// 已有分配记录的条目要走它所属集合的 remove，不从这里绕过。
fn apply_remove_link(ctx: &Context, req: &Request) -> Result<()> {
    let agent = req
        .value("--agent")
        .ok_or_else(|| anyhow!("清理链接必须指定 --agent"))?;
    let link = req
        .value("--link")
        .ok_or_else(|| anyhow!("清理链接必须用 --link 指名"))?;
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

    let old_target = resolve_link_target(&link_path).ok();
    crate::platform::remove_link(&link_path)
        .with_context(|| format!("[{}@{}] 删除链接失败", link, agent))?;

    let mut conn = store::open_write(ctx)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let res = (|| -> Result<()> {
        store::record_operation(
            &tx,
            Some(&req.run_id),
            "skills:remove-link",
            Some(&format!("{}@{}", link, agent)),
            Some(
                &json!({"action": "remove-link", "dangling": old_target.as_ref().map(|t| !t.exists()).unwrap_or(true)}),
            ),
        )?;
        tx.commit()?;
        Ok(())
    })();
    if let Err(err) = res {
        // 审计写不进去就把链接放回原样，不留下「删了但没记录」的状态
        if let Some(target) = old_target {
            let _ = crate::platform::link_dir(&target, &link_path);
        }
        return Err(err);
    }
    Ok(())
}

fn apply_single_skill(
    ctx: &Context,
    req: &Request,
    coll: &CollectionDef,
    meta: &DomainMetadata,
    skill: &str,
    action: &str,
) -> Result<()> {
    let agent = req
        .value("--agent")
        .ok_or_else(|| anyhow!("逐 skill 动作必须指定 --agent"))?;
    let agent_info = meta
        .agents
        .get(agent)
        .ok_or_else(|| anyhow!("未知 agent: {}", agent))?;
    // 移除分配不需要来源：上游删掉了这个技能、或来源本身坏了，也要能撤掉分配。
    let removing = action == "remove";
    // 本地来源集合直接在登记的源目录里发现 skill（直链模式）；git 集合仍走自管仓库缓存。
    if is_local_source(coll) && !removing {
        validate_local_source(ctx, coll, &agent_target_map(meta))?;
    }
    let repo_dir = collection_source_root(ctx, coll);
    let discovered = match discover_repo_skills(&repo_dir, &coll.raw) {
        Ok((found, _)) => found,
        Err(_) if removing => std::collections::BTreeMap::new(),
        Err(err) => return Err(err),
    };
    let source_dir = discovered.get(skill);
    if source_dir.is_none() && !removing {
        bail!("集合 {} 中未发现 skill: {}", coll.name, skill);
    }
    let link_path = agent_info.skills_dir.join(skill);
    let bases = managed_bases(ctx, &meta.collections);

    let mut conn = store::open_write(ctx)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let bindings = load_bindings_from_conn(&tx)?;
    if coll.deleted
        && bindings.iter().any(|((other, name, target), b)| {
            other != &coll.name && name == skill && target == agent && b.assigned
        })
    {
        bail!("该助手有其他来源的同名技能分配，拒绝清理孤立分配的入口");
    }
    let existing_binding = bindings.get(&(coll.name.clone(), skill.to_string(), agent.to_string()));
    let Some(source_dir) = source_dir else {
        // 来源里已经没有这个技能：连分配记录一起删掉，不留下永远找不到来源的一行
        if existing_binding.is_none() {
            bail!("集合 {} 中未发现 skill: {}", coll.name, skill);
        }
        return apply_skill_remove(
            req,
            coll,
            existing_binding,
            &link_path,
            &bases,
            tx,
            skill,
            agent,
            true,
        );
    };

    dispatch_single_skill_action(
        ctx,
        req,
        action,
        coll,
        agent_info,
        existing_binding,
        source_dir,
        &link_path,
        &bases,
        tx,
        skill,
        agent,
    )
}

fn dispatch_single_skill_action(
    ctx: &Context,
    req: &Request,
    action: &str,
    coll: &CollectionDef,
    agent_info: &super::AgentTargetInfo,
    existing_binding: Option<&super::BindingEntry>,
    source_dir: &Path,
    link_path: &Path,
    bases: &[PathBuf],
    tx: rusqlite::Transaction,
    skill: &str,
    agent: &str,
) -> Result<()> {
    match action {
        "add" => apply_skill_add(
            ctx,
            req,
            coll,
            agent_info,
            existing_binding,
            source_dir,
            link_path,
            bases,
            tx,
            skill,
            agent,
        ),
        "update" => apply_skill_update(
            ctx,
            req,
            coll,
            agent_info,
            existing_binding,
            source_dir,
            link_path,
            bases,
            tx,
            skill,
            agent,
        ),
        "enable" => apply_skill_enable(
            ctx,
            req,
            coll,
            agent_info,
            existing_binding,
            link_path,
            bases,
            tx,
            skill,
            agent,
        ),
        "disable" => apply_skill_disable(
            req,
            coll,
            existing_binding,
            link_path,
            bases,
            tx,
            skill,
            agent,
        ),
        "remove" => apply_skill_remove(
            req,
            coll,
            existing_binding,
            link_path,
            bases,
            tx,
            skill,
            agent,
            false,
        ),
        _ => bail!("不支持的技能动作: {}", action),
    }
}

fn apply_skill_add(
    ctx: &Context,
    req: &Request,
    coll: &CollectionDef,
    agent_info: &super::AgentTargetInfo,
    existing_binding: Option<&super::BindingEntry>,
    source_dir: &Path,
    link_path: &Path,
    bases: &[PathBuf],
    tx: rusqlite::Transaction,
    skill: &str,
    agent: &str,
) -> Result<()> {
    if let Some(b) = existing_binding {
        if b.assigned {
            bail!(
                "[{}/{}@{}] 分配已存在，如需更新请使用 update",
                coll.name,
                skill,
                agent
            );
        }
    }
    if !coll.enabled {
        bail!(
            "[{}] 集合已全局停用，拒绝新增技能分配（请先启用集合）",
            coll.name
        );
    }
    if !agent_info.enabled {
        bail!("Agent '{}' 已停用，拒绝新增技能分配", agent);
    }
    if let Some(ref disabled) = coll.disabled {
        if disabled.iter().any(|d| d == skill) {
            bail!("[{}/{}] skill 已被集合全局禁用，拒绝操作", coll.name, skill);
        }
    }
    // 显式接管：--allow-takeover 仅放行非受管符号链接的替换；真实文件/目录仍 fail-closed。
    let allow_takeover = req.flag("--allow-takeover");
    check_not_external_conflict_with_takeover(link_path, bases, allow_takeover)?;
    let replacement = super::preflight::replacement(
        ctx,
        req,
        coll,
        skill,
        agent,
        &load_bindings_from_conn(&tx)?,
        &load_metadata(ctx)?,
    )?;
    let takeover = is_unmanaged_link(link_path, bases);
    // 本地来源集合直链源目录（内容归外部所有、跟随其更新）；git 集合落内容快照。
    let (version, content_path) = if is_local_source(coll) {
        let version = hash_skill_dir(source_dir)?;
        let path = expected_link_target(ctx, coll, skill, &version)?;
        (version, path)
    } else {
        super::git_source::Source::new(&collection_source_root(ctx, coll))
            .snapshot(source_dir, &ctx.data_dir)?
    };

    let mut rollback_guard = RollbackGuard::new();
    let old_target = if is_link_entry(link_path) {
        resolve_link_target(link_path).ok()
    } else {
        None
    };
    atomic_create_or_replace_link(&content_path, link_path)?;
    rollback_guard.record_mutation(
        link_path,
        old_target,
        ExpectedLinkState::Present(resolve_loose(&content_path)),
    );

    let res = (move || -> Result<()> {
        if let Some((origin, version)) = &replacement {
            write_binding(&tx, origin, skill, agent, version, false, false)?;
        }
        write_binding(&tx, &coll.name, skill, agent, &version, true, true)?;
        store::record_operation(
            &tx,
            Some(&req.run_id),
            "skills:add",
            Some(&format!("{}/{}@{}", coll.name, skill, agent)),
            Some(
                &json!({"action": "add", "version": version, "takeover": takeover, "replaced_from": replacement}),
            ),
        )?;
        tx.commit()?;
        Ok(())
    })();
    if let Err(err) = res {
        if let Err(rb_err) = rollback_guard.rollback() {
            return Err(err.context(format!("软链接回滚未完全成功: {rb_err}")));
        }
        return Err(err);
    }
    rollback_guard.disarm();
    Ok(())
}

fn apply_skill_update(
    ctx: &Context,
    req: &Request,
    coll: &CollectionDef,
    agent_info: &super::AgentTargetInfo,
    existing_binding: Option<&super::BindingEntry>,
    source_dir: &Path,
    link_path: &Path,
    bases: &[PathBuf],
    tx: rusqlite::Transaction,
    skill: &str,
    agent: &str,
) -> Result<()> {
    let b = existing_binding.ok_or_else(|| {
        anyhow!(
            "[{}/{}@{}] 无既有分配，无法更新（请先 add）",
            coll.name,
            skill,
            agent
        )
    })?;
    if !b.assigned {
        bail!(
            "[{}/{}@{}] 分配已被移除，如需恢复请使用 add",
            coll.name,
            skill,
            agent
        );
    }
    let (version, content_path) = if is_local_source(coll) {
        let version = hash_skill_dir(source_dir)?;
        let path = expected_link_target(ctx, coll, skill, &version)?;
        (version, path)
    } else {
        super::git_source::Source::new(&collection_source_root(ctx, coll))
            .snapshot(source_dir, &ctx.data_dir)?
    };

    let mut rollback_guard = RollbackGuard::new();
    if b.enabled && coll.enabled && agent_info.enabled {
        check_not_external_conflict(link_path, bases)?;
        let old_target = if is_link_entry(link_path) {
            resolve_link_target(link_path).ok()
        } else {
            None
        };
        atomic_create_or_replace_link(&content_path, link_path)?;
        rollback_guard.record_mutation(
            link_path,
            old_target,
            ExpectedLinkState::Present(resolve_loose(&content_path)),
        );
    }
    let res = (move || -> Result<()> {
        write_binding(&tx, &coll.name, skill, agent, &version, b.enabled, true)?;
        store::record_operation(
            &tx,
            Some(&req.run_id),
            "skills:update",
            Some(&format!("{}/{}@{}", coll.name, skill, agent)),
            Some(&json!({"action": "update", "version": version})),
        )?;
        tx.commit()?;
        Ok(())
    })();
    if let Err(err) = res {
        if let Err(rb_err) = rollback_guard.rollback() {
            return Err(err.context(format!("软链接回滚未完全成功: {rb_err}")));
        }
        return Err(err);
    }
    rollback_guard.disarm();
    Ok(())
}

fn validate_enable_eligibility<'a>(
    coll: &CollectionDef,
    agent_info: &super::AgentTargetInfo,
    existing_binding: Option<&'a super::BindingEntry>,
    skill: &str,
    agent: &str,
) -> Result<&'a super::BindingEntry> {
    let b = existing_binding.ok_or_else(|| {
        anyhow!(
            "[{}/{}@{}] 无既有分配，无法启用（请先 add）",
            coll.name,
            skill,
            agent
        )
    })?;
    if !b.assigned {
        bail!("[{}/{}@{}] 分配已被移除，无法启用", coll.name, skill, agent);
    }
    if !coll.enabled {
        bail!(
            "[{}] 集合已全局停用，拒绝启用下属技能 [{}/{}]（请先启用集合）",
            coll.name,
            coll.name,
            skill
        );
    }
    if !agent_info.enabled {
        bail!(
            "Agent '{}' 已停用，拒绝启用技能 [{}/{}]",
            agent,
            coll.name,
            skill
        );
    }
    if let Some(ref disabled) = coll.disabled {
        if disabled.iter().any(|d| d == skill) {
            bail!("[{}/{}] skill 已被集合全局禁用，拒绝启用", coll.name, skill);
        }
    }
    Ok(b)
}

fn apply_skill_enable(
    ctx: &Context,
    req: &Request,
    coll: &CollectionDef,
    agent_info: &super::AgentTargetInfo,
    existing_binding: Option<&super::BindingEntry>,
    link_path: &Path,
    bases: &[PathBuf],
    tx: rusqlite::Transaction,
    skill: &str,
    agent: &str,
) -> Result<()> {
    let b = validate_enable_eligibility(coll, agent_info, existing_binding, skill, agent)?;
    let content_path = expected_link_target(ctx, coll, skill, &b.version)?;
    check_not_external_conflict(link_path, bases)?;

    let mut rollback_guard = RollbackGuard::new();
    let old_target = if is_link_entry(link_path) {
        resolve_link_target(link_path).ok()
    } else {
        None
    };
    atomic_create_or_replace_link(&content_path, link_path)?;
    rollback_guard.record_mutation(
        link_path,
        old_target,
        ExpectedLinkState::Present(resolve_loose(&content_path)),
    );

    let res = (move || -> Result<()> {
        write_binding(&tx, &coll.name, skill, agent, &b.version, true, true)?;
        store::record_operation(
            &tx,
            Some(&req.run_id),
            "skills:enable",
            Some(&format!("{}/{}@{}", coll.name, skill, agent)),
            Some(&json!({"action": "enable", "version": b.version})),
        )?;
        tx.commit()?;
        Ok(())
    })();
    if let Err(err) = res {
        if let Err(rb_err) = rollback_guard.rollback() {
            return Err(err.context(format!("软链接回滚未完全成功: {rb_err}")));
        }
        return Err(err);
    }
    rollback_guard.disarm();
    Ok(())
}

fn apply_skill_disable(
    req: &Request,
    coll: &CollectionDef,
    existing_binding: Option<&super::BindingEntry>,
    link_path: &Path,
    bases: &[PathBuf],
    tx: rusqlite::Transaction,
    skill: &str,
    agent: &str,
) -> Result<()> {
    let b = existing_binding
        .ok_or_else(|| anyhow!("[{}/{}@{}] 无既有分配，无法停用", coll.name, skill, agent))?;
    let mut rollback_guard = RollbackGuard::new();
    if link_path.exists() || is_link_entry(link_path) {
        check_not_external_conflict(link_path, bases)?;
        let old_target = if is_link_entry(link_path) {
            resolve_link_target(link_path).ok()
        } else {
            None
        };
        safe_remove_link(link_path, bases)?;
        rollback_guard.record_mutation(link_path, old_target, ExpectedLinkState::Absent);
    }
    let res = (move || -> Result<()> {
        write_binding(&tx, &coll.name, skill, agent, &b.version, false, true)?;
        store::record_operation(
            &tx,
            Some(&req.run_id),
            "skills:disable",
            Some(&format!("{}/{}@{}", coll.name, skill, agent)),
            Some(&json!({"action": "disable", "version": b.version})),
        )?;
        tx.commit()?;
        Ok(())
    })();
    if let Err(err) = res {
        if let Err(rb_err) = rollback_guard.rollback() {
            return Err(err.context(format!("软链接回滚未完全成功: {rb_err}")));
        }
        return Err(err);
    }
    rollback_guard.disarm();
    Ok(())
}

fn apply_skill_remove(
    req: &Request,
    coll: &CollectionDef,
    existing_binding: Option<&super::BindingEntry>,
    link_path: &Path,
    bases: &[PathBuf],
    tx: rusqlite::Transaction,
    skill: &str,
    agent: &str,
    source_missing: bool,
) -> Result<()> {
    let old_version = existing_binding
        .map(|b| b.version.clone())
        .unwrap_or_default();
    // 显式接管：--allow-takeover 仅放行非受管符号链接的删除；真实文件/目录仍 fail-closed。
    let allow_takeover = req.flag("--allow-takeover");
    let takeover = is_unmanaged_link(link_path, bases);
    let mut rollback_guard = RollbackGuard::new();
    if link_path.exists() || is_link_entry(link_path) {
        check_not_external_conflict_with_takeover(link_path, bases, allow_takeover)?;
        let old_target = if is_link_entry(link_path) {
            resolve_link_target(link_path).ok()
        } else {
            None
        };
        safe_remove_link_with_takeover(link_path, bases, allow_takeover)?;
        rollback_guard.record_mutation(link_path, old_target, ExpectedLinkState::Absent);
    }
    let res = (move || -> Result<()> {
        if source_missing {
            delete_binding(&tx, &coll.name, skill, agent)?;
        } else {
            write_binding(&tx, &coll.name, skill, agent, &old_version, false, false)?;
        }
        store::record_operation(
            &tx,
            Some(&req.run_id),
            "skills:remove",
            Some(&format!("{}/{}@{}", coll.name, skill, agent)),
            Some(
                &json!({"action": "remove", "takeover": takeover, "source_missing": source_missing}),
            ),
        )?;
        tx.commit()?;
        Ok(())
    })();
    if let Err(err) = res {
        if let Err(rb_err) = rollback_guard.rollback() {
            return Err(err.context(format!("软链接回滚未完全成功: {rb_err}")));
        }
        return Err(err);
    }
    rollback_guard.disarm();
    Ok(())
}

fn apply_collection(
    ctx: &Context,
    req: &Request,
    coll: &CollectionDef,
    meta: &DomainMetadata,
    action: &str,
) -> Result<()> {
    let bases = managed_bases(ctx, &meta.collections);
    let repo_dir = collection_source_root(ctx, coll);

    match action {
        // 本地来源集合没有远端可拉：update = 重扫源目录、刷新版本指纹并修复已启用链接。
        "update" if is_local_source(coll) => apply_local_coll_update(ctx, req, coll, meta, &bases)?,
        "update" => apply_coll_update(ctx, req, coll, &repo_dir)?,
        "enable" | "disable" => {
            apply_coll_toggle(ctx, req, coll, meta, &bases, action == "enable")?
        }
        "remove" => {
            apply_coll_remove(ctx, req, coll, meta, &bases)?;
            if req.flag("--purge") {
                purge_collection_leftovers(ctx, req, coll, meta)?;
            }
        }
        _ => bail!("不支持的集合动作: {}", action),
    }
    Ok(())
}

/// meta 中的 agent → 技能目录映射（本地来源自指校验用）。
fn agent_target_map(meta: &DomainMetadata) -> std::collections::BTreeMap<String, PathBuf> {
    meta.agents
        .iter()
        .map(|(k, v)| (k.clone(), v.skills_dir.clone()))
        .collect()
}

/// 本地来源集合的集合级 update：无网络、无 clone。重扫登记源目录，刷新各已分配 binding 的
/// 内容指纹，并为「已启用且集合/助手都启用」的分配修复直链；源中已消失的 skill 保持原状不动。
fn apply_local_coll_update(
    ctx: &Context,
    req: &Request,
    coll: &CollectionDef,
    meta: &DomainMetadata,
    bases: &[PathBuf],
) -> Result<()> {
    let root = validate_local_source(ctx, coll, &agent_target_map(meta))?;
    let (discovered, _) = discover_repo_skills(&root, &coll.raw)?;

    let mut conn = store::open_write(ctx)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let bindings = load_bindings_from_conn(&tx)?;
    let mut rollback_guard = RollbackGuard::new();
    let mut refreshed = 0usize;

    let res = (|| -> Result<()> {
        for ((c, s, a), b) in &bindings {
            if c != &coll.name || !b.assigned {
                continue;
            }
            let Some(agent_info) = meta.agents.get(a) else {
                continue;
            };
            let Some(source_dir) = discovered.get(s) else {
                continue;
            };
            let version = hash_skill_dir(source_dir)?;
            if b.enabled && coll.enabled && agent_info.enabled {
                let link_path = agent_info.skills_dir.join(s);
                check_not_external_conflict(&link_path, bases)?;
                let old_target = if is_link_entry(&link_path) {
                    resolve_link_target(&link_path).ok()
                } else {
                    None
                };
                atomic_create_or_replace_link(source_dir, &link_path)?;
                rollback_guard.record_mutation(
                    &link_path,
                    old_target,
                    ExpectedLinkState::Present(resolve_loose(source_dir)),
                );
            }
            write_binding(&tx, &coll.name, s, a, &version, b.enabled, true)?;
            refreshed += 1;
        }
        store::record_operation(
            &tx,
            Some(&req.run_id),
            "skills:update",
            Some(&coll.name),
            Some(&json!({"action": "update", "source": "local", "refreshed": refreshed})),
        )?;
        tx.commit()?;
        Ok(())
    })();
    if let Err(err) = res {
        if let Err(rb_err) = rollback_guard.rollback() {
            return Err(err.context(format!("软链接回滚未完全成功: {rb_err}")));
        }
        return Err(err);
    }
    rollback_guard.disarm();
    Ok(())
}

/// 跑一条 git 子命令，失败时把 git 自己的 stderr 带进错误——只说"git pull 失败"
/// 等于没说，用户在界面上看不出到底卡在哪。
fn apply_coll_update(
    ctx: &Context,
    req: &Request,
    coll: &CollectionDef,
    repo_dir: &Path,
) -> Result<()> {
    let url = coll
        .url
        .as_deref()
        .ok_or_else(|| anyhow!("[{}] 集合定义缺少 url", coll.name))?;
    let detail = if req.flag("--offline") {
        if !repo_dir.join(".git").is_dir() {
            bail!("[{}] 离线模式且本地仓库缺失，无法下载", coll.name);
        }
        json!({"action": "update", "offline": true})
    } else {
        let moved = super::versioning::update_repo(ctx, coll, url, repo_dir, req.value("--ref"))?;
        json!({
            "action": "update",
            "from": moved["from"]["commit"],
            "to": moved["to"]["commit"],
            "ref": moved["to"]["label"],
        })
    };
    let mut conn = store::open_write(ctx)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    store::record_operation(
        &tx,
        Some(&req.run_id),
        "skills:update",
        Some(&coll.name),
        Some(&detail),
    )?;
    tx.commit()?;
    Ok(())
}

/// 账号里这个名字的链接是否指向本集合为它准备的内容：git 集合是这条分配记录的版本快照，
/// 本机目录集合是它的源目录。同名链接可能已归别的集合（例如「改成我的版本」之后），
/// 那样的链接在停用、移除本集合时一律不动。
fn link_owned_by(ctx: &Context, coll: &CollectionDef, version: &str, link_path: &Path) -> bool {
    if !is_link_entry(link_path) {
        return false;
    }
    let Ok(actual) = resolve_link_target(link_path) else {
        return false;
    };
    if is_local_source(coll) {
        return super::paths::is_path_within(&actual, &collection_source_root(ctx, coll));
    }
    actual
        == resolve_loose(
            &ctx.data_dir
                .join("skill-versions")
                .join(version)
                .join("content"),
        )
}

fn apply_coll_toggle_link(
    ctx: &Context,
    coll: &CollectionDef,
    bases: &[PathBuf],
    agent_info: &super::AgentTargetInfo,
    skill: &str,
    binding: &super::BindingEntry,
    enabling: bool,
    rollback_guard: &mut RollbackGuard,
) -> Result<()> {
    let link_path = agent_info.skills_dir.join(skill);
    if enabling {
        if binding.enabled && agent_info.enabled {
            let content = expected_link_target(ctx, coll, skill, &binding.version)?;
            check_not_external_conflict(&link_path, bases)?;
            let old_target = if is_link_entry(&link_path) {
                resolve_link_target(&link_path).ok()
            } else {
                None
            };
            atomic_create_or_replace_link(&content, &link_path)?;
            rollback_guard.record_mutation(
                &link_path,
                old_target,
                ExpectedLinkState::Present(resolve_loose(&content)),
            );
        }
    } else if link_owned_by(ctx, coll, &binding.version, &link_path) {
        check_not_external_conflict(&link_path, bases)?;
        let old_target = if is_link_entry(&link_path) {
            resolve_link_target(&link_path).ok()
        } else {
            None
        };
        safe_remove_link(&link_path, bases)?;
        rollback_guard.record_mutation(&link_path, old_target, ExpectedLinkState::Absent);
    }
    Ok(())
}

fn apply_coll_toggle(
    ctx: &Context,
    req: &Request,
    coll: &CollectionDef,
    meta: &DomainMetadata,
    bases: &[PathBuf],
    enabling: bool,
) -> Result<()> {
    let action = if enabling { "enable" } else { "disable" };
    let mut conn = store::open_write(ctx)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let bindings = load_bindings_from_conn(&tx)?;

    let mut rollback_guard = RollbackGuard::new();
    for ((c, s, a), b) in &bindings {
        if c != &coll.name || !b.assigned {
            continue;
        }
        let Some(agent_info) = meta.agents.get(a) else {
            continue;
        };
        apply_coll_toggle_link(
            ctx,
            coll,
            bases,
            agent_info,
            s,
            b,
            enabling,
            &mut rollback_guard,
        )?;
    }

    let res = (|| -> Result<()> {
        let mut policy = store::policy_from_connection(&tx)?;
        if let Some(skills_obj) = policy.get_mut("skills").and_then(Value::as_object_mut) {
            let coll_obj = skills_obj
                .entry(coll.name.clone())
                .or_insert_with(|| json!({}))
                .as_object_mut()
                .ok_or_else(|| anyhow!("skills.{} 不是对象", coll.name))?;
            coll_obj.insert("enabled".to_string(), json!(enabling));
        }
        store::write_policy(&tx, &policy)?;
        store::record_operation(
            &tx,
            Some(&req.run_id),
            &format!("skills:{}", action),
            Some(&coll.name),
            Some(&json!({"action": action, "enabled": enabling})),
        )?;
        tx.commit()?;
        Ok(())
    })();
    if let Err(err) = res {
        if let Err(rb_err) = rollback_guard.rollback() {
            return Err(err.context(format!("软链接回滚未完全成功: {rb_err}")));
        }
        return Err(err);
    }
    rollback_guard.disarm();
    Ok(())
}

/// 移除集合后的收尾（`--purge`）：删掉本工具下载的仓库镜像（只删带所有权标记、位于
/// repositories/ 下的；本机目录来源一律不碰），并清理再没有任何分配记录或账号链接用到的版本快照。
fn purge_collection_leftovers(
    ctx: &Context,
    req: &Request,
    coll: &CollectionDef,
    meta: &DomainMetadata,
) -> Result<()> {
    let mut removed_repo = false;
    if !is_local_source(coll) {
        let base = repo_base_dir(ctx);
        let repo = base.join(&coll.name);
        if repo.join(".agent-manager-owned").is_file()
            && repo.parent() == Some(base.as_path())
            && !is_link_entry(&repo)
        {
            std::fs::remove_dir_all(&repo)
                .with_context(|| format!("删除仓库镜像 {} 失败", repo.display()))?;
            removed_repo = true;
        }
    }
    // 仍在用的快照：任何分配记录里的版本，加上任何账号技能目录里链接指向的快照
    let versions_dir = ctx.data_dir.join("skill-versions");
    let mut in_use: std::collections::BTreeSet<String> = load_bindings_readonly(ctx)?
        .values()
        .map(|b| b.version.clone())
        .collect();
    for agent in meta.agents.values() {
        let Ok(entries) = std::fs::read_dir(&agent.skills_dir) else {
            continue;
        };
        for entry in entries.flatten() {
            if let Ok(target) = crate::platform::read_link(entry.path()) {
                if let Ok(rest) = target.strip_prefix(&versions_dir) {
                    if let Some(first) = rest.components().next() {
                        in_use.insert(first.as_os_str().to_string_lossy().to_string());
                    }
                }
            }
        }
    }
    let mut removed_snapshots = 0usize;
    if let Ok(entries) = std::fs::read_dir(&versions_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.')
                || !super::paths::is_sha256_hex(&name)
                || in_use.contains(&name)
            {
                continue;
            }
            if std::fs::remove_dir_all(entry.path()).is_ok() {
                removed_snapshots += 1;
            }
        }
    }
    let mut conn = store::open_write(ctx)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    store::record_operation(
        &tx,
        Some(&req.run_id),
        "skills:purge",
        Some(&coll.name),
        Some(&json!({"removed_repo": removed_repo, "removed_snapshots": removed_snapshots})),
    )?;
    tx.commit()?;
    Ok(())
}

fn apply_coll_remove(
    ctx: &Context,
    req: &Request,
    coll: &CollectionDef,
    meta: &DomainMetadata,
    bases: &[PathBuf],
) -> Result<()> {
    let mut conn = store::open_write(ctx)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let bindings = load_bindings_from_conn(&tx)?;
    if coll.deleted
        && bindings.iter().any(|((c, s, a), b)| {
            c == &coll.name
                && b.assigned
                && bindings.iter().any(|((other, name, target), binding)| {
                    other != c && name == s && target == a && binding.assigned
                })
        })
    {
        bail!("该助手有其他来源的同名技能分配，拒绝清理孤立分配的入口");
    }

    let mut rollback_guard = RollbackGuard::new();
    for ((c, s, a), b) in &bindings {
        if c != &coll.name {
            continue;
        }
        if let Some(agent_info) = meta.agents.get(a) {
            let link_path = agent_info.skills_dir.join(s);
            // 只移除指向本集合内容的链接；同名链接已归别的集合（如用户的版本）时不动
            if link_owned_by(ctx, coll, &b.version, &link_path) {
                check_not_external_conflict(&link_path, bases)?;
                let old_target = if is_link_entry(&link_path) {
                    resolve_link_target(&link_path).ok()
                } else {
                    None
                };
                safe_remove_link(&link_path, bases)?;
                rollback_guard.record_mutation(&link_path, old_target, ExpectedLinkState::Absent);
            }
        }
        write_binding(&tx, c, s, a, &b.version, false, false)?;
    }

    let res = (|| -> Result<()> {
        let mut policy = store::policy_from_connection(&tx)?;
        if let Some(repos) = policy
            .get_mut("skills")
            .and_then(|v| v.get_mut("repositories"))
            .and_then(Value::as_object_mut)
        {
            if let Some(coll_def) = repos.get_mut(&coll.name).and_then(Value::as_object_mut) {
                coll_def.insert("_delete".to_string(), json!(true));
            }
        }
        store::write_policy(&tx, &policy)?;
        // 来源没了：跟随它的规则文件不再跟随（文件内容不动），规则页不再挂「来源已不在」
        let unfollowed = crate::native_rules::sources::forget_rules_of_source(&tx, &coll.name)?;
        // 连同收尾：这个集合的分配记录一并删掉，之后才能判定哪些快照没人再用
        if req.flag("--purge") {
            tx.execute(
                "DELETE FROM skill_bindings WHERE collection = ?1",
                [&coll.name],
            )?;
        }
        store::record_operation(
            &tx,
            Some(&req.run_id),
            "skills:remove",
            Some(&coll.name),
            Some(
                &json!({"action": "remove", "purge": req.flag("--purge"), "unfollowed_rules": unfollowed}),
            ),
        )?;
        tx.commit()?;
        Ok(())
    })();
    if let Err(err) = res {
        if let Err(rb_err) = rollback_guard.rollback() {
            return Err(err.context(format!("软链接回滚未完全成功: {rb_err}")));
        }
        return Err(err);
    }
    rollback_guard.disarm();
    Ok(())
}
