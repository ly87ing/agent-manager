//! MCP 预检与动作执行（mcp:apply）

use anyhow::{anyhow, Result};
use rusqlite::TransactionBehavior;
use serde_json::{json, Value};

use super::model::{compute_definition_version, Binding};
use super::persistence::{ensure_table, load_bindings_from, save_binding_in_tx};
use super::policy_view::{build_entry, library_view};
use super::preflight::{parse_action_args, preflight_apply, preflight_check_state};
use super::render::render_target_content;
use super::rollback::{atomic_write_checked, RollbackGuard};
use super::targets::{agent_targets, check_shared_targets, extract_entry, Format};
use crate::native::{Context, Request};
use crate::store;

pub fn apply(ctx: &Context, req: &Request) -> Result<Value> {
    let parsed = parse_action_args(req)?;
    let only = req.value("--only").unwrap_or("");
    preflight_apply(ctx, req)?;

    if parsed.dry_run {
        return Ok(json!({
            "domain": "mcp",
            "dry_run": true,
            "steps": [],
            "snapshot": Value::Null,
            "skipped": [only]
        }));
    }

    let mut conn = store::open_write(ctx)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

    let mut policy = store::policy_from_connection(&tx)?;
    let bindings = load_bindings_from(&tx)?;

    // 在任何文件/DB 写入前，基于 Immediate 事务内的最新状态执行与
    // preflight 等价的校验：直接调用 apply（绕过 execute 的预检）同样被拒绝。
    preflight_check_state(&parsed, &policy, &ctx.home_dir, &bindings)?;

    ensure_table(&tx)?;
    let library = library_view(&policy);
    let (targets, _) = agent_targets(&policy, &ctx.home_dir)?;
    check_shared_targets(&targets)?;

    let mut guard = RollbackGuard::new();

    let execute_result: Result<()> = (|| {
        if let Some(ref a) = parsed.agent {
            let target = targets
                .iter()
                .find(|t| &t.agent == a)
                .ok_or_else(|| anyhow!("未找到 Agent '{}' 的 MCP 配置路径", a))?;
            let binding = bindings.get(&parsed.server).and_then(|m| m.get(a));

            match parsed.action.as_str() {
                "add" | "update" => {
                    let def = library
                        .definitions
                        .get(&parsed.server)
                        .ok_or_else(|| anyhow!("服务定义 '{}' 不存在", parsed.server))?;
                    let entry = build_entry(def, &parsed.server, target.format.clone(), a)?;
                    let version = compute_definition_version(def);

                    let rendered = render_target_content(
                        &target.path,
                        &target.format,
                        &parsed.server,
                        Some(&entry),
                    )?;
                    let old_bytes = rendered.old_bytes;
                    let content = rendered.content;
                    atomic_write_checked(
                        &target.path,
                        &content,
                        Some((&parsed.server, &entry, target.format.clone())),
                        None,
                        old_bytes.as_deref(),
                    )?;
                    guard.record_written(&target.path, old_bytes, content.into_bytes());

                    let new_binding = Binding {
                        version,
                        definition: def.clone(),
                        enabled: true,
                        assigned: true,
                        applied_digest: entry.digest(),
                    };
                    save_binding_in_tx(&tx, &parsed.server, a, Some(&new_binding))?;
                }
                "enable" => {
                    let b = binding.filter(|b| b.assigned).ok_or_else(|| {
                        anyhow!("服务 '{}' 尚未分配给 Agent '{}'", parsed.server, a)
                    })?;
                    let entry =
                        build_entry(&b.definition, &parsed.server, target.format.clone(), a)?;

                    let rendered = render_target_content(
                        &target.path,
                        &target.format,
                        &parsed.server,
                        Some(&entry),
                    )?;
                    let old_bytes = rendered.old_bytes;
                    let content = rendered.content;
                    atomic_write_checked(
                        &target.path,
                        &content,
                        Some((&parsed.server, &entry, target.format.clone())),
                        None,
                        old_bytes.as_deref(),
                    )?;
                    guard.record_written(&target.path, old_bytes, content.into_bytes());

                    let updated = Binding {
                        version: b.version.clone(),
                        definition: b.definition.clone(),
                        enabled: true,
                        assigned: true,
                        applied_digest: entry.digest(),
                    };
                    save_binding_in_tx(&tx, &parsed.server, a, Some(&updated))?;
                }
                "disable" => {
                    let b = binding.filter(|b| b.assigned).ok_or_else(|| {
                        anyhow!("服务 '{}' 尚未分配给 Agent '{}'", parsed.server, a)
                    })?;

                    if target.path.exists() {
                        let rendered = render_target_content(
                            &target.path,
                            &target.format,
                            &parsed.server,
                            None,
                        )?;
                        let old_bytes = rendered.old_bytes;
                        let content = rendered.content;
                        atomic_write_checked(
                            &target.path,
                            &content,
                            None,
                            Some((&parsed.server, target.format.clone())),
                            old_bytes.as_deref(),
                        )?;
                        guard.record_written(&target.path, old_bytes, content.into_bytes());
                    }

                    let updated = Binding {
                        version: b.version.clone(),
                        definition: b.definition.clone(),
                        enabled: false,
                        assigned: true,
                        applied_digest: String::new(),
                    };
                    save_binding_in_tx(&tx, &parsed.server, a, Some(&updated))?;
                }
                "remove" => {
                    if target.path.exists() {
                        let rendered = render_target_content(
                            &target.path,
                            &target.format,
                            &parsed.server,
                            None,
                        )?;
                        let old_bytes = rendered.old_bytes;
                        let content = rendered.content;
                        atomic_write_checked(
                            &target.path,
                            &content,
                            None,
                            Some((&parsed.server, target.format.clone())),
                            old_bytes.as_deref(),
                        )?;
                        guard.record_written(&target.path, old_bytes, content.into_bytes());
                    }

                    save_binding_in_tx(&tx, &parsed.server, a, None)?;
                }
                _ => unreachable!(),
            }
        } else {
            match parsed.action.as_str() {
                "enable" => {
                    if let Some(policy_map) = policy.as_object_mut() {
                        let mcp_obj = policy_map.entry("mcp").or_insert_with(|| json!({}));
                        if let Some(obj) = mcp_obj.as_object_mut() {
                            let sw = obj.entry(&parsed.server).or_insert_with(|| json!({}));
                            if let Some(sw_map) = sw.as_object_mut() {
                                sw_map.insert("enabled".into(), json!(true));
                            }
                        }
                    }
                    store::write_policy(&tx, &policy)?;
                }
                "disable" => {
                    if let Some(policy_map) = policy.as_object_mut() {
                        let mcp_obj = policy_map.entry("mcp").or_insert_with(|| json!({}));
                        if let Some(obj) = mcp_obj.as_object_mut() {
                            let sw = obj.entry(&parsed.server).or_insert_with(|| json!({}));
                            if let Some(sw_map) = sw.as_object_mut() {
                                sw_map.insert("enabled".into(), json!(false));
                            }
                        }
                    }
                    store::write_policy(&tx, &policy)?;
                }
                "remove" => {
                    for target in &targets {
                        if let Format::Unsupported(_) = target.format {
                            continue;
                        }
                        let (entry_opt, is_managed) =
                            extract_entry(&target.path, target.format.clone(), &parsed.server)?;
                        let has_binding = bindings
                            .get(&parsed.server)
                            .and_then(|m| m.get(&target.agent))
                            .is_some();
                        if has_binding || (entry_opt.is_some() && is_managed) {
                            if target.path.exists() {
                                let rendered = render_target_content(
                                    &target.path,
                                    &target.format,
                                    &parsed.server,
                                    None,
                                )?;
                                let old_bytes = rendered.old_bytes;
                                let content = rendered.content;
                                atomic_write_checked(
                                    &target.path,
                                    &content,
                                    None,
                                    Some((&parsed.server, target.format.clone())),
                                    old_bytes.as_deref(),
                                )?;
                                guard.record_written(&target.path, old_bytes, content.into_bytes());
                            }
                        }
                    }

                    tx.execute(
                        "DELETE FROM mcp_bindings WHERE server = ?1",
                        rusqlite::params![parsed.server],
                    )?;

                    if let Some(mcp) = policy.get_mut("mcp").and_then(Value::as_object_mut) {
                        if let Some(defs) =
                            mcp.get_mut("definitions").and_then(Value::as_object_mut)
                        {
                            if let Some(def_val) =
                                defs.get_mut(&parsed.server).and_then(Value::as_object_mut)
                            {
                                def_val.insert("_delete".into(), json!(true));
                            }
                        }
                        mcp.remove(&parsed.server);
                    }
                    store::write_policy(&tx, &policy)?;
                }
                _ => unreachable!(),
            }
        }

        store::record_operation(
            &tx,
            Some(&req.run_id),
            &format!("mcp:{}", parsed.action),
            Some(only),
            Some(&json!({
                "action": parsed.action,
                "agent": parsed.agent,
                "ok": true
            })),
        )?;

        tx.commit()?;
        Ok(())
    })();

    if let Err(e) = execute_result {
        if let Err(rb_err) = guard.rollback() {
            return Err(anyhow!("执行失败: {e:#}；配置回滚同时失败: {rb_err:#}"));
        }
        return Err(e);
    }

    guard.commit();

    Ok(json!({
        "domain": "mcp",
        "dry_run": false,
        "steps": [{
            "name": format!("mcp:{} {}", parsed.action, only),
            "ok": true,
            "exit_code": 0
        }],
        "snapshot": Value::Null,
        "skipped": []
    }))
}
