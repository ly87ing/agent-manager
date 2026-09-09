use anyhow::{anyhow, bail, Result};
use serde_json::{json, Map, Value};

use super::paths::{is_link_entry, parse_only, repo_base_dir};
use super::persistence::load_bindings_readonly;
use super::preflight::preflight_apply;
use super::status::{load_metadata, DomainMetadata};
use crate::native::{Context, Request};

pub fn build_plan(ctx: &Context, req: &Request) -> Result<Value> {
    preflight_apply(ctx, req)?;

    let only = req
        .value("--only")
        .ok_or_else(|| anyhow!("缺少 --only 参数"))?;
    let action = req
        .value("--action")
        .ok_or_else(|| anyhow!("缺少 --action 参数"))?;
    let target = parse_only(only)?;
    let meta = load_metadata(ctx)?;

    let mut actions = Vec::new();
    match &target.skill {
        Some(skill) => {
            let agent = req
                .value("--agent")
                .ok_or_else(|| anyhow!("逐 skill 动作必须指定 --agent"))?;
            plan_single_skill(
                action,
                &target.collection,
                skill,
                agent,
                &meta,
                &mut actions,
            )?;
        }
        None => {
            plan_collection(action, &target.collection, ctx, &meta, &mut actions)?;
        }
    }

    let mut summary = Map::new();
    for risk in [
        "read",
        "write-local",
        "exec-local",
        "network",
        "install-remove",
    ] {
        let count = actions
            .iter()
            .filter(|a| a.get("risk").and_then(Value::as_str) == Some(risk))
            .count();
        summary.insert(risk.to_string(), json!(count));
    }

    Ok(json!({
        "domain": "skills",
        "actions": actions,
        "summary": summary,
    }))
}

fn plan_single_skill(
    action: &str,
    coll_name: &str,
    skill: &str,
    agent: &str,
    meta: &DomainMetadata,
    actions: &mut Vec<Value>,
) -> Result<()> {
    let agent_info = meta
        .agents
        .get(agent)
        .ok_or_else(|| anyhow!("未知 agent: {}", agent))?;
    let link_path = agent_info.skills_dir.join(skill);
    let element = format!("skill:{}/{}", coll_name, skill);

    match action {
        "add" | "enable" => {
            actions.push(json!({
                "op": "symlink",
                "element": element,
                "target": link_path.display().to_string(),
                "risk": "exec-local",
                "note": format!("显式{}受管入口 {}/{}@{}", action, coll_name, skill, agent),
            }));
        }
        "update" => {
            actions.push(json!({
                "op": "symlink",
                "element": element,
                "target": link_path.display().to_string(),
                "risk": "exec-local",
                "note": format!("显式更新受管入口 {}/{}@{}", coll_name, skill, agent),
            }));
        }
        "disable" | "remove" => {
            actions.push(json!({
                "op": "unlink",
                "element": element,
                "target": link_path.display().to_string(),
                "risk": "exec-local",
                "note": format!("显式{}受管入口 {}/{}@{}", action, coll_name, skill, agent),
            }));
        }
        _ => bail!("不支持的技能动作: {}", action),
    }
    Ok(())
}

fn plan_collection(
    action: &str,
    coll_name: &str,
    ctx: &Context,
    meta: &DomainMetadata,
    actions: &mut Vec<Value>,
) -> Result<()> {
    match action {
        "update" => plan_coll_update(coll_name, ctx, actions),
        "enable" => plan_coll_enable(coll_name, ctx, meta, actions),
        "disable" => plan_coll_disable(coll_name, ctx, meta, actions),
        "remove" => plan_coll_remove(coll_name, ctx, meta, actions),
        _ => bail!("不支持的集合动作: {}", action),
    }
}

fn plan_coll_update(coll_name: &str, ctx: &Context, actions: &mut Vec<Value>) -> Result<()> {
    let element = format!("skill:{}", coll_name);
    let repo_dir = repo_base_dir(ctx).join(coll_name);
    actions.push(json!({
        "op": "git-pull",
        "element": element,
        "target": repo_dir.display().to_string(),
        "risk": "network",
        "note": format!("刷新集合仓库缓存 {}", coll_name),
    }));
    actions.push(json!({
        "op": "write-file",
        "element": element,
        "target": Value::Null,
        "risk": "write-local",
        "note": format!("集合 {} 既有分配保持 pinned 版本，不追新", coll_name),
    }));
    Ok(())
}

fn plan_coll_enable(
    coll_name: &str,
    ctx: &Context,
    meta: &DomainMetadata,
    actions: &mut Vec<Value>,
) -> Result<()> {
    let element = format!("skill:{}", coll_name);
    let bindings = load_bindings_readonly(ctx)?;
    for ((c, s, a), b) in &bindings {
        if c == coll_name && b.assigned && b.enabled {
            if let Some(agent_info) = meta.agents.get(a) {
                let link = agent_info.skills_dir.join(s);
                actions.push(json!({
                    "op": "symlink",
                    "element": format!("skill:{}/{}", c, s),
                    "target": link.display().to_string(),
                    "risk": "exec-local",
                    "note": format!("集合 {} enable: skill {}@{}", c, s, a),
                }));
            }
        }
    }
    actions.push(json!({
        "op": "write-file",
        "element": element,
        "target": Value::Null,
        "risk": "write-local",
        "note": format!("恢复集合 {} 原已启用分配并保存全局开关 enabled=true", coll_name),
    }));
    Ok(())
}

fn plan_coll_disable(
    coll_name: &str,
    ctx: &Context,
    meta: &DomainMetadata,
    actions: &mut Vec<Value>,
) -> Result<()> {
    let element = format!("skill:{}", coll_name);
    let bindings = load_bindings_readonly(ctx)?;
    for ((c, s, a), b) in &bindings {
        if c == coll_name && b.assigned {
            if let Some(agent_info) = meta.agents.get(a) {
                let link = agent_info.skills_dir.join(s);
                if link.exists() || is_link_entry(&link) {
                    actions.push(json!({
                        "op": "unlink",
                        "element": format!("skill:{}/{}", c, s),
                        "target": link.display().to_string(),
                        "risk": "exec-local",
                        "note": format!("集合 {} disable: skill {}@{}", c, s, a),
                    }));
                }
            }
        }
    }
    actions.push(json!({
        "op": "write-file",
        "element": element,
        "target": Value::Null,
        "risk": "write-local",
        "note": format!("撤下集合 {} 当前启用入口并保存全局开关 enabled=false", coll_name),
    }));
    Ok(())
}

fn plan_coll_remove(
    coll_name: &str,
    ctx: &Context,
    meta: &DomainMetadata,
    actions: &mut Vec<Value>,
) -> Result<()> {
    let element = format!("skill:{}", coll_name);
    let bindings = load_bindings_readonly(ctx)?;
    for ((c, s, a), b) in &bindings {
        if c == coll_name && b.assigned {
            if let Some(agent_info) = meta.agents.get(a) {
                let link = agent_info.skills_dir.join(s);
                actions.push(json!({
                    "op": "unlink",
                    "element": format!("skill:{}/{}", c, s),
                    "target": link.display().to_string(),
                    "risk": "write-local",
                    "note": format!("清理集合 {} 链接: {}@{}", c, s, a),
                }));
            }
        }
    }
    actions.push(json!({
        "op": "write-file",
        "element": element,
        "target": Value::Null,
        "risk": "write-local",
        "note": format!("清理集合 {} 全部分配并写入 definition tombstone；源目录保留", coll_name),
    }));
    Ok(())
}
