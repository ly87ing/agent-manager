//! MCP 纯读状态聚合（mcp:status，零写库保证）

use anyhow::Result;
use serde_json::{json, Map, Value};

use super::model::{
    compute_definition_version, is_valid_server_name, mask_definition_with_agents_for_status,
    Binding,
};
use super::persistence::load_bindings_readonly;
use super::policy_view::{
    allowed_agents, expected_extra_fields, inspect_env, inspect_headers, is_allowed,
    library_enabled, library_view, validate_definition, validate_definition_env,
};
use super::targets::{
    agent_targets, extract_entry, validate_target_path, AgentTarget, Entry, Format,
};
use crate::native::Context;
use crate::store;

fn target_exists(target: &std::path::Path) -> bool {
    match std::fs::symlink_metadata(target) {
        Ok(_) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(_) => true,
    }
}

fn make_cell(
    state: &str,
    target: &std::path::Path,
    binding: Option<&Binding>,
    avail_version: Value,
    actions: Vec<&str>,
    blocked_reason: Option<String>,
) -> Value {
    json!({
        "state": state,
        "target": target.to_string_lossy(),
        "target_exists": target_exists(target),
        "assigned": binding.map_or(false, |b| b.assigned),
        "enabled": binding.map_or(false, |b| b.enabled),
        "version": binding.and_then(|b| if b.assigned { Some(Value::String(b.version.clone())) } else { None }).unwrap_or(Value::Null),
        "available_version": avail_version,
        "actions": actions,
        "blocked_reason": blocked_reason.map(Value::String).unwrap_or(Value::Null)
    })
}

/// stdio 命令能否在本机解析（只读：查 PATH 与官方安装器常用的 ~/.local/bin）。
///
/// 这是配完之后最容易踩的坑——命令没装，助手启动时才报错。只做存在性判断，
/// 不执行它，也不对远程服务做网络探测（那要发请求、可能带上鉴权，不适合只读状态）。
fn command_resolvable(command: &str, home_dir: &std::path::Path) -> bool {
    if command.contains('/') {
        return std::path::Path::new(command).is_file();
    }
    let mut dirs: Vec<std::path::PathBuf> = std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).collect())
        .unwrap_or_default();
    dirs.push(home_dir.join(".local").join("bin"));
    dirs.into_iter().any(|dir| dir.join(command).is_file())
}

/// 定义的命令可用性：stdio 给 true/false，远程定义没有本地命令，恒 null。
fn command_availability(definition: &Value, home_dir: &std::path::Path) -> Value {
    match definition.get("command").and_then(Value::as_str) {
        Some(command) if !command.trim().is_empty() => {
            Value::Bool(command_resolvable(command.trim(), home_dir))
        }
        _ => Value::Null,
    }
}

fn make_element(
    server: &str,
    spec: Value,
    enabled: bool,
    agents: Value,
    env: Map<String, Value>,
    headers: Map<String, Value>,
    agent_cells: Map<String, Value>,
    obs_actions: Vec<&str>,
    blocked_reason: Option<String>,
    state: &str,
    actions: Vec<Value>,
    freshness: Value,
    command_available: Value,
) -> Value {
    json!({
        "id": format!("mcp:{}", server),
        "domain": "mcp",
        "spec": spec,
        "policy": { "enabled": enabled, "agents": agents },
        "observed": {
            "env": env,
            "headers": headers,
            "agents": agent_cells,
            "actions": obs_actions,
            "command_available": command_available,
            "blocked_reason": blocked_reason.map(Value::String).unwrap_or(Value::Null)
        },
        "state": state,
        "freshness": freshness,
        "actions": actions
    })
}

/// 无上游概念（不在探测宇宙）的 mcp 元素固定 freshness 块。
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

fn build_orphan_element(
    server: &str,
    bindings: &std::collections::BTreeMap<String, std::collections::BTreeMap<String, Binding>>,
    targets: &[AgentTarget],
) -> Value {
    let def = bindings
        .get(server)
        .and_then(|m| m.values().next())
        .map(|b| b.definition.clone())
        .unwrap_or_else(|| json!({"command": "orphan"}));
    let mut agent_cells = Map::new();
    for t in targets {
        let b = bindings.get(server).and_then(|m| m.get(&t.agent));
        let (s, acts, blk) = if !t.enabled {
            match extract_entry(&t.path, t.format.clone(), server) {
                Err(e) => (
                    "probe-error",
                    vec![],
                    Some(format!("配置文件解析失败: {}", e)),
                ),
                Ok((Some(_), false)) => (
                    "drifted",
                    vec![],
                    Some("存在外部非受管同名条目，拒绝操作以防止冲突".into()),
                ),
                Ok((Some(_), true)) => (
                    "leftover",
                    vec![],
                    Some(format!("Agent '{}' 已在策略中停用", t.agent)),
                ),
                Ok((None, _)) if b.is_some() => (
                    "disabled",
                    vec![],
                    Some(format!("Agent '{}' 已在策略中停用", t.agent)),
                ),
                Ok((None, _)) => ("absent", vec![], None),
            }
        } else {
            match extract_entry(&t.path, t.format.clone(), server) {
                Err(e) => (
                    "probe-error",
                    vec![],
                    Some(format!("配置文件解析失败: {}", e)),
                ),
                Ok((Some(_), false)) => (
                    "drifted",
                    vec![],
                    Some("存在外部非受管同名条目，拒绝操作以防止冲突".into()),
                ),
                Ok((entry, managed)) => {
                    let res = b.is_some() || (entry.is_some() && managed);
                    if res {
                        if b.map_or(false, |b| b.assigned && b.enabled) {
                            ("drifted", vec!["disable", "remove"], None)
                        } else {
                            ("leftover", vec!["remove"], None)
                        }
                    } else {
                        ("absent", vec![], None)
                    }
                }
            }
        };
        agent_cells.insert(
            t.agent.clone(),
            make_cell(s, &t.path, b, Value::Null, acts, blk),
        );
    }
    let has_blocker = agent_cells.values().any(|c| {
        c.get("state").and_then(Value::as_str) == Some("probe-error")
            || c.get("blocked_reason")
                .and_then(Value::as_str)
                .map_or(false, |r| r.contains("非受管同名条目"))
    });
    let has_rem = targets.iter().any(|target| {
        let binding_exists = bindings
            .get(server)
            .and_then(|per_agent| per_agent.get(&target.agent))
            .is_some();
        let cell_has_residue = agent_cells
            .get(&target.agent)
            .and_then(|cell| cell.get("state"))
            .and_then(Value::as_str)
            .map_or(false, |state| matches!(state, "leftover" | "drifted"));
        binding_exists || cell_has_residue
    });
    let (orphan_actions, obs_actions, blk_reason) = if has_blocker || !has_rem {
        let r = if has_blocker {
            "部分目标配置文件解析失败或存在非受管冲突，无法执行清理"
        } else {
            "无受管残留配置可清理"
        };
        (vec![], vec![], Some(r.to_string()))
    } else {
        (
            vec![
                json!({ "id": "remove", "risk": "write-local", "needs_lock": true, "offline_ok": true }),
            ],
            vec!["remove"],
            None,
        )
    };
    make_element(
        server,
        mask_definition_with_agents_for_status(&def),
        false,
        Value::Null,
        Map::new(),
        Map::new(),
        agent_cells,
        obs_actions,
        blk_reason,
        "drifted",
        orphan_actions,
        freshness_not_applicable(),
        // 孤儿元素只剩残留配置，没有库内定义可判定命令
        Value::Null,
    )
}

fn compute_cell_actions(
    target: &AgentTarget,
    binding: Option<&Binding>,
    actual_entry: Option<&Entry>,
    is_managed: bool,
    def: &Value,
    server: &str,
    def_error: Option<&str>,
    lib_version: &str,
    agent_allowed: bool,
    missing_reason: Option<&str>,
) -> (&'static str, Vec<&'static str>, Option<String>) {
    if let Some(entry) = actual_entry {
        if !is_managed {
            return (
                "drifted",
                vec![],
                Some("存在外部非受管同名条目，拒绝操作以防止冲突".into()),
            );
        }
        if let Ok(expected) = expected_extra_fields(def, target.format.clone(), server) {
            if let Some((k, _)) = entry.extra.iter().find(|(k, _)| !expected.contains_key(*k)) {
                return (
                    "drifted",
                    vec![],
                    Some(format!(
                        "目标文件 {} 中的条目 '{}' 包含未受管外部属性（外部未识别字段 '{}'），拒绝覆盖以防止外部配置丢失",
                        target.path.display(),
                        server,
                        k
                    )),
                );
            }
        }
    }
    if !target.enabled {
        if actual_entry.is_some() && is_managed {
            return (
                "leftover",
                vec![],
                Some(format!("Agent '{}' 已在策略中停用", target.agent)),
            );
        }
        return (
            "disabled",
            vec![],
            Some(format!("Agent '{}' 已在策略中停用", target.agent)),
        );
    }
    if let Some(err_msg) = def_error {
        let has_res = binding.map_or(false, |b| b.assigned && b.enabled)
            || (actual_entry.is_some() && is_managed);
        let s = "unsupported";
        let acts = if has_res && binding.map_or(false, |b| b.assigned) {
            vec!["disable", "remove"]
        } else if has_res || binding.map_or(false, |b| b.assigned) {
            vec!["remove"]
        } else {
            vec![]
        };
        return (s, acts, Some(err_msg.to_string()));
    }
    if let Some(b) = binding.filter(|b| b.assigned) {
        if b.enabled {
            let in_sync = actual_entry.map_or(false, |entry| {
                entry.digest() == b.applied_digest && b.version == lib_version
            });
            let (s, acts) = if in_sync && agent_allowed {
                ("managed-ok", vec!["disable", "remove"])
            } else {
                let mut acts = Vec::new();
                if agent_allowed && missing_reason.is_none() {
                    acts.push("update");
                }
                acts.extend(["disable", "remove"]);
                let s = if actual_entry.is_some() {
                    "drifted"
                } else if target.path.exists() {
                    "missing"
                } else {
                    "not-materialized"
                };
                (s, acts)
            };
            let blk = if !agent_allowed {
                Some(format!("Agent '{}' 不在服务允许列表中", target.agent))
            } else {
                missing_reason.map(str::to_string)
            };
            return (s, acts, blk);
        } else if actual_entry.is_some() && is_managed {
            let blk = if !agent_allowed {
                Some(format!("Agent '{}' 不在服务允许列表中", target.agent))
            } else {
                None
            };
            return ("drifted", vec!["disable", "remove"], blk);
        } else {
            let mut acts = Vec::new();
            let blk = if !agent_allowed {
                Some(format!("Agent '{}' 不在服务允许列表中", target.agent))
            } else if missing_reason.is_none() {
                acts.push("enable");
                None
            } else {
                missing_reason.map(str::to_string)
            };
            acts.push("remove");
            return ("disabled", acts, blk);
        }
    }
    if actual_entry.is_some() && is_managed {
        ("leftover", vec!["remove"], None)
    } else if agent_allowed {
        let s = if target.path.exists() {
            "missing"
        } else {
            "not-materialized"
        };
        let acts = if missing_reason.is_none() {
            vec!["add"]
        } else {
            vec![]
        };
        (s, acts, missing_reason.map(str::to_string))
    } else {
        ("absent", vec![], None)
    }
}

pub fn status(ctx: &Context) -> Result<Value> {
    let policy = store::effective_policy(ctx)?;
    let library = library_view(&policy);
    let (targets, _disabled_agents) = agent_targets(&policy, &ctx.home_dir)?;
    let bindings = load_bindings_readonly(ctx)?;
    let probes = crate::native_upstream::load_probe_cache(ctx)?;

    let mut elements = Vec::new();

    let all_servers: std::collections::BTreeSet<_> =
        library.definitions.keys().chain(bindings.keys()).collect();

    for server in all_servers {
        let Some(def) = library.definitions.get(server) else {
            elements.push(build_orphan_element(server, &bindings, &targets));
            continue;
        };

        let name_valid = is_valid_server_name(server);
        let def_shape_res = validate_definition(def, server);
        let env_res = validate_definition_env(def);
        let allowed_res = allowed_agents(def);

        let def_error = if !name_valid {
            Some(format!(
                "服务标识 '{}' 非法: 仅支持字母、数字、短横线与下划线",
                server
            ))
        } else if let Err(e) = def_shape_res {
            Some(e.to_string())
        } else if let Err(e) = env_res {
            Some(e.to_string())
        } else if let Err(e) = allowed_res.as_ref() {
            Some(e.to_string())
        } else {
            None
        };

        let lib_enabled = library_enabled(&library, server);
        let lib_version = compute_definition_version(def);
        let lib_version_val = Value::String(lib_version.clone());
        let allowed = allowed_res.ok().flatten();
        let (observed_env, missing_env_vars) = inspect_env(def);
        let (observed_headers, missing_header_vars) = inspect_headers(def);

        let mut missing_vars = missing_env_vars;
        missing_vars.extend(missing_header_vars);
        missing_vars.sort();
        missing_vars.dedup();

        let missing_reason = if missing_vars.is_empty() {
            None
        } else {
            Some(format!("凭证变量未配置: {}", missing_vars.join(", ")))
        };

        let mut agent_cells = Map::new();
        let mut cell_states = Vec::new();

        for target in &targets {
            let agent = &target.agent;
            let agent_allowed = is_allowed(&allowed, agent);
            let binding = bindings.get(server).and_then(|m| m.get(agent));

            let pre_error = if let Format::Unsupported(ref fmt) = target.format {
                Some(("unsupported", format!("不支持的配置格式: {}", fmt)))
            } else if let Err(e) = validate_target_path(&target.path) {
                Some(("probe-error", e.to_string()))
            } else {
                None
            };
            if let Some((s, reason)) = pre_error {
                agent_cells.insert(
                    agent.clone(),
                    make_cell(
                        s,
                        &target.path,
                        binding,
                        lib_version_val.clone(),
                        vec![],
                        Some(reason),
                    ),
                );
                cell_states.push(s);
                continue;
            }

            let (actual_entry, is_managed) =
                match extract_entry(&target.path, target.format.clone(), server) {
                    Ok(pair) => pair,
                    Err(e) => {
                        agent_cells.insert(
                            agent.clone(),
                            make_cell(
                                "probe-error",
                                &target.path,
                                binding,
                                lib_version_val.clone(),
                                vec![],
                                Some(format!("配置文件解析失败: {}", e)),
                            ),
                        );
                        cell_states.push("probe-error");
                        continue;
                    }
                };

            let (state, actions, blocked_reason) = compute_cell_actions(
                target,
                binding,
                actual_entry.as_ref(),
                is_managed,
                def,
                server,
                def_error.as_deref(),
                &lib_version,
                agent_allowed,
                missing_reason.as_deref(),
            );

            cell_states.push(state);
            agent_cells.insert(
                agent.clone(),
                make_cell(
                    state,
                    &target.path,
                    binding,
                    lib_version_val.clone(),
                    actions,
                    blocked_reason,
                ),
            );
        }

        let overall_state =
            if def_error.is_some() || cell_states.iter().any(|&s| s == "unsupported") {
                "unsupported"
            } else if !lib_enabled {
                "disabled"
            } else if cell_states
                .iter()
                .any(|&s| s == "drifted" || s == "leftover" || s == "probe-error")
            {
                "drifted"
            } else if cell_states.iter().all(|&s| s == "managed-ok") && !cell_states.is_empty() {
                "managed-ok"
            } else if cell_states.iter().any(|&s| s == "managed-ok") {
                "drifted"
            } else if cell_states.iter().any(|&s| s == "disabled") {
                "disabled"
            } else if cell_states.iter().any(|&s| s == "not-materialized") {
                "not-materialized"
            } else {
                "missing"
            };

        // 上游轴：仅 enabled 且为 npx 包 / uvx git 源的定义参与探测宇宙；
        // 其余（含 orphan）恒 not-applicable。status 只读缓存，缺行 → unknown。
        let freshness = if lib_enabled && crate::native_upstream::classify_mcp_probe(def).is_some()
        {
            crate::native_upstream::freshness_from_cache(&probes, &format!("mcp:{}", server), None)
        } else {
            freshness_not_applicable()
        };

        let mut lib_actions = Vec::new();
        if def_error.is_none() {
            lib_actions.push(if lib_enabled { "disable" } else { "enable" });
        }
        // 官方出了新版才谈得上升级，而且只有 npx 包的版本是写在定义里、本工具改得动的；
        // uvx git 源跟的是分支，没有版本号可钉。
        let upgradable = def_error.is_none()
            && freshness["state"] == "behind"
            && matches!(
                crate::native_upstream::classify_mcp_probe(def),
                Some(crate::native_upstream::McpProbeSpec::Npx(_))
            );
        if upgradable {
            lib_actions.push("upgrade");
        }
        let has_remove_blocker = agent_cells.iter().any(|(agent, c)| {
            let state = c.get("state").and_then(Value::as_str).unwrap_or("");
            let binding_exists = bindings
                .get(server)
                .and_then(|per_agent| per_agent.get(agent))
                .is_some();
            let target_exists = c
                .get("target_exists")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let blk = c
                .get("blocked_reason")
                .and_then(Value::as_str)
                .unwrap_or("");
            state == "probe-error"
                || (state == "unsupported"
                    && blk.contains("不支持的配置格式")
                    && (binding_exists || target_exists))
                || blk.contains("非受管同名条目")
                || blk.contains("未受管外部属性")
        });
        if !has_remove_blocker {
            lib_actions.push("remove");
        }

        let elem_actions: Vec<Value> = lib_actions
            .iter()
            .map(|act| {
                json!({ "id": act, "risk": "write-local", "needs_lock": true, "offline_ok": true })
            })
            .collect();

        elements.push(make_element(
            server,
            mask_definition_with_agents_for_status(def),
            lib_enabled,
            def.get("agents").cloned().unwrap_or(Value::Null),
            observed_env,
            observed_headers,
            agent_cells,
            lib_actions,
            def_error,
            overall_state,
            elem_actions,
            freshness,
            command_availability(def, &ctx.home_dir),
        ));
    }

    elements.sort_by(|a, b| {
        a["id"]
            .as_str()
            .unwrap_or("")
            .cmp(b["id"].as_str().unwrap_or(""))
    });

    // 未受管现有配置的导入候选（只读扫描，零写）；掩码语义见 discovery 模块。
    let import_candidates = super::discovery::import_candidates(&library, &bindings, &targets);

    Ok(json!({
        "domain": "mcp",
        "elements": elements,
        "import_candidates": import_candidates
    }))
}
