//! MCP 计划生成（mcp:plan，零写）

use anyhow::Result;
use serde_json::{json, Value};

use super::model::sha256_hex_bytes;
use super::persistence::load_bindings_readonly;
use super::policy_view::{build_entry, library_view};
use super::preflight::{parse_action_args, preflight_apply};
use super::render::render_target_content;
use super::targets::{agent_targets, extract_entry, Format};
use crate::native::{Context, Request};
use crate::store;

pub fn plan(ctx: &Context, req: &Request) -> Result<Value> {
    preflight_apply(ctx, req)?;
    let parsed = parse_action_args(req)?;
    let policy = store::effective_policy(ctx)?;
    let library = library_view(&policy);
    let (targets, _) = agent_targets(&policy, &ctx.home_dir)?;
    let bindings = load_bindings_readonly(ctx)?;

    let mut actions = Vec::new();

    if let Some(ref a) = parsed.agent {
        let target = targets.iter().find(|t| &t.agent == a);

        if let Some(target) = target {
            let binding = bindings
                .get(&parsed.server)
                .and_then(|m| m.get(&target.agent));
            let def_ref = match parsed.action.as_str() {
                "enable" => binding
                    .map(|b| &b.definition)
                    .or_else(|| library.definitions.get(&parsed.server)),
                _ => library
                    .definitions
                    .get(&parsed.server)
                    .or_else(|| binding.map(|b| &b.definition)),
            };

            let should_act = match parsed.action.as_str() {
                "add" | "update" => true,
                "enable" => binding.map_or(false, |b| b.assigned),
                "disable" => binding.map_or(false, |b| {
                    b.assigned
                        && (b.enabled
                            || extract_entry(&target.path, target.format.clone(), &parsed.server)
                                .map(|(e, m)| e.is_some() && m)
                                .unwrap_or(false))
                }),
                "remove" => {
                    binding.map_or(false, |b| b.assigned) || {
                        let (e, m) =
                            extract_entry(&target.path, target.format.clone(), &parsed.server)?;
                        e.is_some() && m
                    }
                }
                _ => false,
            };

            if should_act {
                // 显式接管标注：--allow-takeover 的 add/update 且目标存在未受管同名条目时，
                // 预览步骤必须让用户看到「将替换未受管条目」（独立字段 + 说明文案）。
                let takeover = parsed.allow_takeover
                    && matches!(parsed.action.as_str(), "add" | "update")
                    && extract_entry(&target.path, target.format.clone(), &parsed.server)
                        .map(|(e, m)| e.is_some() && !m)
                        .unwrap_or(false);

                let note = match parsed.action.as_str() {
                    "disable" => format!("撤下 {} 在 {} 中的配置入口", parsed.server, target.agent),
                    "remove" => format!("清理 {} 在 {} 中的配置", parsed.server, target.agent),
                    _ if takeover => format!(
                        "{} mcp:{} 到 {}（接管：将替换目标文件中的未受管同名条目）",
                        parsed.action, parsed.server, target.agent
                    ),
                    _ => format!(
                        "{} mcp:{} 到 {}",
                        parsed.action, parsed.server, target.agent
                    ),
                };

                let should_write_file = match parsed.action.as_str() {
                    "add" | "update" | "enable" => true,
                    "disable" | "remove" => target.path.exists(),
                    _ => false,
                };

                if should_write_file {
                    let entry_opt = if matches!(parsed.action.as_str(), "add" | "update" | "enable")
                    {
                        if let Some(def) = def_ref {
                            Some(build_entry(
                                def,
                                &parsed.server,
                                target.format.clone(),
                                &target.agent,
                            )?)
                        } else {
                            None
                        }
                    } else {
                        None
                    };

                    let rendered = render_target_content(
                        &target.path,
                        &target.format,
                        &parsed.server,
                        entry_opt.as_ref(),
                    )?;
                    let before_digest = rendered
                        .old_bytes
                        .as_deref()
                        .map(|bytes| format!("sha256:{}", sha256_hex_bytes(bytes)));
                    let after_digest = Some(format!(
                        "sha256:{}",
                        sha256_hex_bytes(rendered.content.as_bytes())
                    ));

                    let mut file_action = json!({
                        "op": "write-file",
                        "element": format!("mcp:{}", parsed.server),
                        "target": target.path.to_string_lossy(),
                        "risk": "write-local",
                        "before_digest": before_digest,
                        "after_digest": after_digest,
                        "backup_path": Value::Null,
                        "cmd": Value::Null,
                        "note": note
                    });
                    if takeover {
                        file_action
                            .as_object_mut()
                            .expect("object")
                            .insert("takeover".into(), Value::Bool(true));
                    }
                    actions.push(file_action);
                }

                actions.push(json!({
                    "op": "write-file",
                    "element": format!("mcp:{}", parsed.server),
                    "target": format!("native-manager.db:mcp_bindings:{}", target.agent),
                    "risk": "write-local",
                    "before_digest": Value::Null,
                    "after_digest": Value::Null,
                    "backup_path": Value::Null,
                    "cmd": Value::Null,
                    "note": format!("更新 {} 在 {} 的绑定记录", parsed.server, target.agent)
                }));
            }
        }
    } else {
        match parsed.action.as_str() {
            "enable" | "disable" => {
                actions.push(json!({
                    "op": "write-file",
                    "element": format!("mcp:{}", parsed.server),
                    "target": "native-manager.db:policy",
                    "risk": "write-local",
                    "before_digest": Value::Null,
                    "after_digest": Value::Null,
                    "backup_path": Value::Null,
                    "cmd": Value::Null,
                    "note": format!(
                        "{} mcp 服务 '{}' 库级总开关",
                        if parsed.action == "enable" { "启用" } else { "禁用" },
                        parsed.server
                    )
                }));
            }
            "remove" => {
                actions.push(json!({
                    "op": "write-file",
                    "element": format!("mcp:{}", parsed.server),
                    "target": "native-manager.db:policy",
                    "risk": "write-local",
                    "before_digest": Value::Null,
                    "after_digest": Value::Null,
                    "backup_path": Value::Null,
                    "cmd": Value::Null,
                    "note": format!("从策略中移除 mcp 服务 '{}' 定义与开关", parsed.server)
                }));
                actions.push(json!({
                    "op": "write-file",
                    "element": format!("mcp:{}", parsed.server),
                    "target": "native-manager.db:mcp_bindings",
                    "risk": "write-local",
                    "before_digest": Value::Null,
                    "after_digest": Value::Null,
                    "backup_path": Value::Null,
                    "cmd": Value::Null,
                    "note": format!("清理 mcp 服务 '{}' 的所有助手绑定记录", parsed.server)
                }));

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
                    if (has_binding || (entry_opt.is_some() && is_managed)) && target.path.exists()
                    {
                        let rendered = render_target_content(
                            &target.path,
                            &target.format,
                            &parsed.server,
                            None,
                        )?;
                        let before_digest = rendered
                            .old_bytes
                            .as_deref()
                            .map(|bytes| format!("sha256:{}", sha256_hex_bytes(bytes)));
                        let after_digest = Some(format!(
                            "sha256:{}",
                            sha256_hex_bytes(rendered.content.as_bytes())
                        ));
                        actions.push(json!({
                            "op": "write-file",
                            "element": format!("mcp:{}", parsed.server),
                            "target": target.path.to_string_lossy(),
                            "risk": "write-local",
                            "before_digest": before_digest,
                            "after_digest": after_digest,
                            "backup_path": Value::Null,
                            "cmd": Value::Null,
                            "note": format!("清理 {} 在 {} 中的配置", parsed.server, target.agent)
                        }));
                    }
                }
            }
            _ => {}
        }
    }

    let summary = json!({
        "read": 0,
        "write-local": actions.len(),
        "exec-local": 0,
        "network": 0,
        "install-remove": 0
    });

    Ok(json!({
        "domain": "mcp",
        "actions": actions,
        "summary": summary
    }))
}
