use anyhow::Result;
use rusqlite::TransactionBehavior;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use super::paths::{
    expand_home, is_link_entry, is_managed_link, managed_bases, repo_base_dir, resolve_link_target,
    resolve_loose,
};
use super::persistence::load_bindings_readonly;
use super::snapshot::{discover_repo_skills, hash_skill_dir};
use super::{AgentTargetInfo, CollectionDef};
use crate::native::{Context, Request};
use crate::store;

pub struct DomainMetadata {
    pub collections: BTreeMap<String, CollectionDef>,
    pub agents: BTreeMap<String, AgentTargetInfo>,
}

pub fn load_metadata(ctx: &Context) -> Result<DomainMetadata> {
    let policy = store::effective_policy(ctx)?;
    let agents = load_agent_targets(&policy, &ctx.home_dir);
    let collections = load_collection_defs(&policy);
    Ok(DomainMetadata {
        collections,
        agents,
    })
}

fn load_agent_targets(policy: &Value, home_dir: &Path) -> BTreeMap<String, AgentTargetInfo> {
    let mut agents = BTreeMap::new();
    let agent_defs = policy
        .get("agents")
        .and_then(|v| v.get("definitions"))
        .and_then(Value::as_object);
    let Some(agent_defs) = agent_defs else {
        return agents;
    };
    for (agent_id, raw) in agent_defs {
        if raw.get("_delete") == Some(&Value::Bool(true)) {
            continue;
        }
        let enabled = policy
            .get("agents")
            .and_then(|v| v.get(agent_id))
            .and_then(|v| v.get("enabled"))
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let skills_dir_raw = raw.get("skills_dir").and_then(Value::as_str).or_else(|| {
            policy
                .get("skills")
                .and_then(|v| v.get("agents"))
                .and_then(|v| v.get(agent_id))
                .and_then(Value::as_str)
        });
        let skills_dir = match skills_dir_raw {
            Some(p) => expand_home(p, home_dir),
            None => home_dir.join(format!(".{}/skills", agent_id)),
        };
        agents.insert(
            agent_id.clone(),
            AgentTargetInfo {
                id: agent_id.clone(),
                enabled,
                skills_dir,
            },
        );
    }
    agents
}

fn load_collection_defs(policy: &Value) -> BTreeMap<String, CollectionDef> {
    let mut collections = BTreeMap::new();
    let repos = policy
        .get("skills")
        .and_then(|v| v.get("repositories"))
        .and_then(Value::as_object);
    let Some(repos) = repos else {
        return collections;
    };
    for (coll_name, raw) in repos {
        let is_deleted = raw.get("_delete") == Some(&Value::Bool(true));
        let raw_obj = raw.as_object().cloned().unwrap_or_default();
        let coll_enabled = policy
            .get("skills")
            .and_then(|v| v.get(coll_name))
            .and_then(|v| v.get("enabled"))
            .and_then(Value::as_bool)
            .unwrap_or(true);

        let url = raw
            .get("url")
            .and_then(Value::as_str)
            .map(|s| s.to_string());
        let skill_paths = raw.get("skill_paths").and_then(Value::as_array).map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(|s| s.to_string())
                .collect()
        });
        let allowed_agents = raw.get("agents").and_then(Value::as_array).map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(|s| s.to_string())
                .collect()
        });
        let disabled_skills = raw.get("disabled").and_then(Value::as_array).map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(|s| s.to_string())
                .collect()
        });

        collections.insert(
            coll_name.clone(),
            CollectionDef {
                name: coll_name.clone(),
                url,
                skill_paths,
                agents: allowed_agents,
                disabled: disabled_skills,
                enabled: coll_enabled,
                deleted: is_deleted,
                raw: raw_obj,
            },
        );
    }
    collections
}

pub fn record_apply_operation(
    ctx: &Context,
    req: &Request,
    target: &str,
    action: &str,
    ok: bool,
) -> Result<()> {
    let mut conn = store::open_write(ctx)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    store::record_operation(
        &tx,
        Some(&req.run_id),
        &format!("skills:{action}"),
        Some(target),
        Some(&json!({"action": action, "ok": ok})),
    )?;
    tx.commit()?;
    Ok(())
}

struct AgentCellEval {
    state: &'static str,
    actions: Vec<&'static str>,
    blocked_reason: Option<String>,
    should_be_linked: bool,
    link_exists: bool,
    is_managed: bool,
}

fn eval_agent_cell(
    coll: &CollectionDef,
    agent_info: &AgentTargetInfo,
    agent_id: &str,
    binding: Option<&super::BindingEntry>,
    link_path: &Path,
    bases: &[PathBuf],
    available: bool,
) -> AgentCellEval {
    let link_exists = link_path.exists() || is_link_entry(link_path);
    let is_managed = is_managed_link(link_path, bases);
    let is_supported = match &coll.agents {
        Some(allowed) => allowed.contains(&agent_id.to_string()),
        None => true,
    };
    let assigned = binding.map(|b| b.assigned).unwrap_or(false);
    let binding_enabled = binding.map(|b| b.enabled).unwrap_or(false);

    if !is_supported {
        return AgentCellEval {
            state: "unsupported",
            actions: vec![],
            blocked_reason: Some("该助手不在集合允许的助手范围内".to_string()),
            should_be_linked: false,
            link_exists,
            is_managed,
        };
    }
    if !assigned {
        let actions = if coll.enabled && agent_info.enabled && available {
            vec!["add"]
        } else {
            vec![]
        };
        let st = if link_exists {
            "extra-link"
        } else {
            "not-assigned"
        };
        return AgentCellEval {
            state: st,
            actions,
            blocked_reason: None,
            should_be_linked: false,
            link_exists,
            is_managed,
        };
    }

    let (state, act, blocked_reason, should_be_linked) = eval_assigned_agent_cell(
        binding_enabled,
        coll.enabled,
        agent_info.enabled,
        available,
        link_exists,
        is_managed,
    );

    AgentCellEval {
        state,
        actions: act,
        blocked_reason,
        should_be_linked,
        link_exists,
        is_managed,
    }
}

fn eval_assigned_agent_cell(
    binding_enabled: bool,
    coll_enabled: bool,
    agent_enabled: bool,
    available: bool,
    link_exists: bool,
    is_managed: bool,
) -> (&'static str, Vec<&'static str>, Option<String>, bool) {
    let mut act = Vec::new();
    if available {
        act.push("update");
    }
    if binding_enabled && coll_enabled && agent_enabled {
        act.push("disable");
    } else if coll_enabled && agent_enabled {
        act.push("enable");
    }
    act.push("remove");

    let should_be_linked = binding_enabled && coll_enabled && agent_enabled;
    let (state, blocked_reason) = if should_be_linked {
        if !link_exists {
            ("missing-link", None)
        } else if !is_managed {
            ("drifted", Some("目标入口存在外部非受管内容".to_string()))
        } else {
            ("linked", None)
        }
    } else {
        (
            if link_exists {
                "extra-link"
            } else {
                "disabled"
            },
            None,
        )
    };

    (state, act, blocked_reason, should_be_linked)
}

fn check_managed_target_state(
    ctx: &Context,
    link_path: &Path,
    version: Option<&str>,
) -> &'static str {
    match resolve_link_target(link_path) {
        Ok(target) => {
            let expected_content = ctx
                .data_dir
                .join("skill-versions")
                .join(version.unwrap_or(""))
                .join("content");
            if !target.exists() {
                "dead-link"
            } else if target == resolve_loose(&expected_content) {
                "linked"
            } else {
                "drifted"
            }
        }
        Err(_) => "dead-link",
    }
}

struct SkillAgentAggregation {
    agents_cell_map: Map<String, Value>,
    any_assigned: bool,
    any_drifted: bool,
    any_missing: bool,
    all_linked: bool,
    all_disabled: bool,
}

fn aggregate_skill_agents(
    ctx: &Context,
    coll_name: &str,
    coll: &CollectionDef,
    skill_name: &str,
    available: bool,
    available_version: Option<&str>,
    meta: &DomainMetadata,
    bindings: &BTreeMap<(String, String, String), super::BindingEntry>,
    bases: &[PathBuf],
) -> SkillAgentAggregation {
    let mut agents_cell_map = Map::new();
    let mut any_assigned = false;
    let mut any_drifted = false;
    let mut any_missing = false;
    let mut all_linked = true;
    let mut all_disabled = true;

    for (agent_id, agent_info) in &meta.agents {
        let binding = bindings.get(&(
            coll_name.to_string(),
            skill_name.to_string(),
            agent_id.clone(),
        ));
        let link_path = agent_info.skills_dir.join(skill_name);
        let assigned = binding.map(|b| b.assigned).unwrap_or(false);
        let binding_enabled = binding.map(|b| b.enabled).unwrap_or(false);
        let version = binding.map(|b| b.version.clone());
        if assigned {
            any_assigned = true;
        }

        let mut ev = eval_agent_cell(
            coll, agent_info, agent_id, binding, &link_path, bases, available,
        );
        verify_agent_link_state(
            ctx,
            &mut ev,
            &link_path,
            version.as_deref(),
            &mut all_disabled,
            &mut any_missing,
            &mut any_drifted,
            &mut all_linked,
        );

        agents_cell_map.insert(
            agent_id.clone(),
            json!({
                "state": ev.state,
                "link_path": link_path.display().to_string(),
                "target_dir_exists": agent_info.skills_dir.is_dir(),
                "assigned": assigned,
                "enabled": binding_enabled,
                "version": version,
                "available_version": available_version,
                "actions": ev.actions,
                "blocked_reason": ev.blocked_reason,
                "target": link_path.display().to_string(),
            }),
        );
    }

    SkillAgentAggregation {
        agents_cell_map,
        any_assigned,
        any_drifted,
        any_missing,
        all_linked,
        all_disabled,
    }
}

fn verify_agent_link_state(
    ctx: &Context,
    ev: &mut AgentCellEval,
    link_path: &Path,
    version: Option<&str>,
    all_disabled: &mut bool,
    any_missing: &mut bool,
    any_drifted: &mut bool,
    all_linked: &mut bool,
) {
    if ev.should_be_linked {
        *all_disabled = false;
        if !ev.link_exists {
            *any_missing = true;
            *all_linked = false;
        } else if !ev.is_managed {
            *any_drifted = true;
            *all_linked = false;
        } else {
            let verified_state = check_managed_target_state(ctx, link_path, version);
            ev.state = verified_state;
            if verified_state != "linked" {
                *all_linked = false;
                if verified_state == "dead-link" {
                    *any_missing = true;
                } else {
                    *any_drifted = true;
                }
            }
        }
    } else {
        *all_linked = false;
    }
}

fn build_skill_element(
    ctx: &Context,
    coll_name: &str,
    coll: &CollectionDef,
    skill_name: &str,
    source_path: Option<&PathBuf>,
    meta: &DomainMetadata,
    bindings: &BTreeMap<(String, String, String), super::BindingEntry>,
    bases: &[PathBuf],
    inbox_skills: &mut Vec<String>,
) -> Value {
    let available = source_path.is_some();
    let available_version = source_path.and_then(|p| hash_skill_dir(p).ok());
    let agg = aggregate_skill_agents(
        ctx,
        coll_name,
        coll,
        skill_name,
        available,
        available_version.as_deref(),
        meta,
        bindings,
        bases,
    );

    let is_inbox = available && !agg.any_assigned;
    if is_inbox {
        inbox_skills.push(skill_name.to_string());
    }

    let skill_state = if agg.any_drifted {
        "drifted"
    } else if agg.any_missing {
        "missing"
    } else if agg.all_linked && agg.any_assigned {
        "managed-ok"
    } else if agg.all_disabled && agg.any_assigned {
        "disabled"
    } else if is_inbox {
        "disabled"
    } else {
        "managed-ok"
    };

    json!({
        "id": format!("skill:{}/{}", coll_name, skill_name),
        "domain": "skills",
        "spec": {
            "collection": coll_name,
            "name": skill_name,
            "source_path": source_path.map(|p| p.display().to_string()),
        },
        "policy": {
            "enabled": coll.enabled,
            "agents": agg.agents_cell_map.iter().map(|(k, v)| (k.clone(), Value::Bool(v["enabled"].as_bool().unwrap_or(false)))).collect::<Map<String, Value>>(),
        },
        "observed": {
            "agents": Value::Object(agg.agents_cell_map),
            "discovery_inbox": is_inbox,
        },
        "state": skill_state,
        "freshness": {
            "state": "not-applicable",
            "current": Value::Null,
            "upstream": Value::Null,
            "checked_at": Value::Null,
            "probe": Value::Null,
            "cache_age_sec": Value::Null,
        },
        "actions": [],
    })
}

fn build_collection_element(
    coll_name: &str,
    coll: &CollectionDef,
    repo_dir: &Path,
    repo_cached: bool,
    owner_marked: bool,
    discovered_names: &[String],
    inbox_skills: &[String],
) -> Value {
    let mut coll_actions = vec!["update".to_string()];
    if coll.enabled {
        coll_actions.push("disable".to_string());
    } else {
        coll_actions.push("enable".to_string());
    }
    coll_actions.push("remove".to_string());

    let coll_state = if !coll.enabled {
        "disabled"
    } else if !repo_cached {
        "not-materialized"
    } else {
        "managed-ok"
    };

    json!({
        "id": format!("skill:{}", coll_name),
        "domain": "skills",
        "spec": {
            "url": coll.url,
            "skill_paths": coll.skill_paths,
            "agents": coll.agents,
        },
        "policy": {
            "enabled": coll.enabled,
        },
        "observed": {
            "repo_cache": repo_dir.display().to_string(),
            "repo_cached": repo_cached,
            "owner_marked": owner_marked,
            "discovered": discovered_names,
            "discovery_inbox": inbox_skills,
            "extra_links": [],
            "actions": coll_actions,
            "blocked_reason": Value::Null,
        },
        "state": coll_state,
        "freshness": {
            "state": "not-applicable",
            "current": Value::Null,
            "upstream": Value::Null,
            "checked_at": Value::Null,
            "probe": Value::Null,
            "cache_age_sec": Value::Null,
        },
        "actions": coll_actions,
    })
}

pub fn build_status(ctx: &Context) -> Result<Value> {
    let meta = load_metadata(ctx)?;
    let bindings = load_bindings_readonly(ctx)?;
    let bases = managed_bases(ctx);
    let mut elements = Vec::new();

    for (coll_name, coll) in &meta.collections {
        if coll.deleted {
            continue;
        }
        let repo_dir = repo_base_dir(ctx).join(coll_name);
        let repo_cached = repo_dir.is_dir();
        let owner_marked = repo_dir.join(".agent-manager-owned").exists();

        let (discovered_map, _dups) = if repo_cached {
            discover_repo_skills(&repo_dir, &coll.raw).unwrap_or_default()
        } else {
            (BTreeMap::new(), BTreeSet::new())
        };
        let discovered_names: Vec<String> = discovered_map.keys().cloned().collect();

        let mut coll_skills: BTreeSet<String> = discovered_map.keys().cloned().collect();
        for ((c, s, _), _) in &bindings {
            if c == coll_name {
                coll_skills.insert(s.clone());
            }
        }

        let mut inbox_skills = Vec::new();
        for skill_name in &coll_skills {
            elements.push(build_skill_element(
                ctx,
                coll_name,
                coll,
                skill_name,
                discovered_map.get(skill_name),
                &meta,
                &bindings,
                &bases,
                &mut inbox_skills,
            ));
        }

        elements.push(build_collection_element(
            coll_name,
            coll,
            &repo_dir,
            repo_cached,
            owner_marked,
            &discovered_names,
            &inbox_skills,
        ));
    }

    Ok(json!({
        "domain": "skills",
        "elements": elements,
    }))
}
