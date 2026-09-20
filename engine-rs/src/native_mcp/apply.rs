//! MCP 预检与动作执行（mcp:apply）

use anyhow::{anyhow, bail, Result};
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

/// 升级要钉到哪个版本：取上次「检查更新」记下的官方最新版。
///
/// 没查过、或查出来并不落后，都不该悄悄改定义——宁可让用户先去点一下检查更新，
/// 也不拿一份不知道多旧的记录去改配置。
fn upgrade_target(ctx: &Context, server: &str) -> Result<String> {
    let cache = crate::native_upstream::load_probe_cache(ctx)?;
    let record = cache
        .get(&format!("mcp:{server}"))
        .ok_or_else(|| anyhow!("还没查过 '{server}' 的最新版本，先点一下「检查更新」再升级"))?;
    let upstream = record
        .upstream
        .as_deref()
        .filter(|v| !v.is_empty())
        .ok_or_else(|| anyhow!("没查到 '{server}' 的最新版本，先点一下「检查更新」再升级"))?;
    if record.state != "behind" {
        bail!("'{server}' 已经是最新版 {upstream}，不用升级");
    }
    Ok(upstream.to_string())
}

/// 把定义里的 npx 包参数钉到指定版本：`foo` 或 `foo@1.2.0` 都写成 `foo@<version>`。
fn pin_npx_version(def: &mut Value, version: &str) -> Result<()> {
    let args = def
        .get_mut("args")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| anyhow!("定义里没有 args，无法钉版本"))?;
    for arg in args.iter_mut() {
        let Some(text) = arg.as_str() else { continue };
        if text.starts_with('-') {
            continue;
        }
        let (name, _) = crate::native_upstream::split_package_spec(text);
        *arg = json!(format!("{name}@{version}"));
        return Ok(());
    }
    bail!("定义的 args 里找不到包名")
}

pub fn apply(ctx: &Context, req: &Request) -> Result<Value> {
    let parsed = parse_action_args(req)?;
    let only = req.value("--only").unwrap_or("");
    preflight_apply(ctx, req)?;

    // 升级的目标版本来自上次「检查更新」记下的结果。趁还没开写事务先读出来——
    // Immediate 事务握着写锁时再开只读连接会互相等。
    let upgrade_to = if parsed.action == "upgrade" {
        Some(upgrade_target(ctx, &parsed.server)?)
    } else {
        None
    };

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
    let mut takeover = false;

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

                    // 事务内重检（preflight_check_state 已跑）后记录接管事实：
                    // 显式接管且写入前已有同名条目 → 本次为整体替换，审计需标记。
                    takeover = parsed.allow_takeover
                        && extract_entry(&target.path, target.format.clone(), &parsed.server)
                            .map(|(entry, _)| entry.is_some())
                            .unwrap_or(false);

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
                "upgrade" => {
                    let version = upgrade_to.as_deref().expect("upgrade 分支必有目标版本");
                    let defs = policy
                        .get_mut("mcp")
                        .and_then(Value::as_object_mut)
                        .and_then(|m| m.get_mut("definitions"))
                        .and_then(Value::as_object_mut)
                        .ok_or_else(|| anyhow!("策略中没有 MCP 定义"))?;
                    let def = defs
                        .get_mut(&parsed.server)
                        .ok_or_else(|| anyhow!("服务定义 '{}' 不存在", parsed.server))?;
                    pin_npx_version(def, version)?;
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
                "ok": true,
                "takeover": takeover
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
        "steps": [{
            "name": format!("mcp:{} {}", parsed.action, only),
            "ok": true,
            "exit_code": 0
        }],
        "skipped": []
    }))
}
