use anyhow::{anyhow, bail, Context as _, Result};
use rusqlite::TransactionBehavior;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

use super::paths::{
    atomic_create_or_replace_link, check_not_external_conflict, is_link_entry, managed_bases,
    parse_only, repo_base_dir, resolve_link_target, resolve_loose, safe_remove_link,
    ExpectedLinkState, RollbackGuard,
};
use super::persistence::{load_bindings_from_conn, load_bindings_readonly, write_binding};
use super::snapshot::{discover_repo_skills, hash_skill_dir, snapshot_skill, verify_skill_version};
use super::status::{load_metadata, record_apply_operation, DomainMetadata};
use super::CollectionDef;
use crate::native::{Context, Request};
use crate::store;

pub fn apply(ctx: &Context, req: &Request) -> Result<Value> {
    let only = req
        .value("--only")
        .ok_or_else(|| anyhow!("缺少 --only 参数"))?;
    let action = req
        .value("--action")
        .ok_or_else(|| anyhow!("缺少 --action 参数"))?;
    let dry_run = req.flag("--dry-run");
    let target = parse_only(only)?;

    if dry_run {
        return Ok(json!({
            "domain": "skills",
            "dry_run": true,
            "steps": [],
            "skipped": [only],
        }));
    }

    let result = (|| -> Result<Value> {
        let meta = load_metadata(ctx)?;
        let fallback_coll = CollectionDef {
            name: target.collection.clone(),
            url: None,
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

        match &target.skill {
            Some(skill) => apply_single_skill(ctx, req, coll, &meta, skill, action)?,
            None => apply_collection(ctx, req, coll, &meta, action)?,
        }

        Ok(json!({
            "domain": "skills",
            "dry_run": false,
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
    let repo_dir = repo_base_dir(ctx).join(&coll.name);
    let (discovered, _) = discover_repo_skills(&repo_dir, &coll.raw)?;
    let source_dir = discovered
        .get(skill)
        .ok_or_else(|| anyhow!("集合 {} 中未发现 skill: {}", coll.name, skill))?;
    let link_path = agent_info.skills_dir.join(skill);
    let bases = managed_bases(ctx);

    let mut conn = store::open_write(ctx)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let bindings = load_bindings_from_conn(&tx)?;
    let existing_binding = bindings.get(&(coll.name.clone(), skill.to_string(), agent.to_string()));

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
    check_not_external_conflict(link_path, bases)?;
    let content_path = snapshot_skill(source_dir, &ctx.data_dir)?;
    let version = hash_skill_dir(source_dir)?;

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
        write_binding(&tx, &coll.name, skill, agent, &version, true, true)?;
        store::record_operation(
            &tx,
            Some(&req.run_id),
            "skills:add",
            Some(&format!("{}/{}@{}", coll.name, skill, agent)),
            Some(&json!({"action": "add", "version": version})),
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
    let content_path = snapshot_skill(source_dir, &ctx.data_dir)?;
    let version = hash_skill_dir(source_dir)?;

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
    let content_path = verify_skill_version(&ctx.data_dir, &b.version)?;
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
) -> Result<()> {
    let old_version = existing_binding
        .map(|b| b.version.clone())
        .unwrap_or_default();
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
        write_binding(&tx, &coll.name, skill, agent, &old_version, false, false)?;
        store::record_operation(
            &tx,
            Some(&req.run_id),
            "skills:remove",
            Some(&format!("{}/{}@{}", coll.name, skill, agent)),
            Some(&json!({"action": "remove"})),
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
    let bases = managed_bases(ctx);
    let repo_dir = repo_base_dir(ctx).join(&coll.name);

    match action {
        "update" => apply_coll_update(ctx, req, coll, &repo_dir)?,
        "enable" | "disable" => {
            apply_coll_toggle(ctx, req, coll, meta, &bases, action == "enable")?
        }
        "remove" => apply_coll_remove(ctx, req, coll, meta, &bases)?,
        _ => bail!("不支持的集合动作: {}", action),
    }
    Ok(())
}

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
    let offline = req.flag("--offline");
    if repo_dir.join(".git").is_dir() {
        if !offline {
            let st = std::process::Command::new("git")
                .args(["fetch", "--tags", "origin"])
                .current_dir(repo_dir)
                .status()
                .with_context(|| format!("git fetch 失败: {}", repo_dir.display()))?;
            if !st.success() {
                bail!("[{}] git fetch 失败", coll.name);
            }
            let st = std::process::Command::new("git")
                .args(["pull", "--ff-only"])
                .current_dir(repo_dir)
                .status()
                .with_context(|| format!("git pull 失败: {}", repo_dir.display()))?;
            if !st.success() {
                bail!("[{}] git pull 失败", coll.name);
            }
        }
    } else {
        if offline {
            bail!("[{}] 离线模式且本地仓库缺失，无法下载", coll.name);
        }
        std::fs::create_dir_all(repo_base_dir(ctx))?;
        let st = std::process::Command::new("git")
            .args(["clone", url, &repo_dir.to_string_lossy()])
            .status()
            .with_context(|| "git clone 启动失败")?;
        if !st.success() {
            bail!("[{}] git clone 失败", coll.name);
        }
        let _ = std::fs::write(
            repo_dir.join(".agent-manager-owned"),
            serde_json::to_string_pretty(&json!({
                "managed_by": "agent-manager",
                "repository": coll.name
            }))? + "\n",
        );
    }
    let mut conn = store::open_write(ctx)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    store::record_operation(
        &tx,
        Some(&req.run_id),
        "skills:update",
        Some(&coll.name),
        Some(&json!({"action": "update"})),
    )?;
    tx.commit()?;
    Ok(())
}

fn apply_coll_toggle_link(
    ctx: &Context,
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
            let content = verify_skill_version(&ctx.data_dir, &binding.version)?;
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
    } else if link_path.exists() || is_link_entry(&link_path) {
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
        apply_coll_toggle_link(ctx, bases, agent_info, s, b, enabling, &mut rollback_guard)?;
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

    let mut rollback_guard = RollbackGuard::new();
    for ((c, s, a), b) in &bindings {
        if c != &coll.name {
            continue;
        }
        if let Some(agent_info) = meta.agents.get(a) {
            let link_path = agent_info.skills_dir.join(s);
            if link_path.exists() || is_link_entry(&link_path) {
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
        store::record_operation(
            &tx,
            Some(&req.run_id),
            "skills:remove",
            Some(&coll.name),
            Some(&json!({"action": "remove"})),
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
