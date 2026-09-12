//! hooks 域只读状态闭环：目标固定为 model::targets()，来源只从
//! hook_sources 当前有效 JSON 取得（缺 DB/表用编译期默认），运行时不读
//! checkout。status 不创建 data/home 目录、DB、表或文件；来源/数据库损坏
//! fail-closed，目标路径安全与解析问题降级为 unmanaged-detected +
//! blocked_reason。输出不含来源/目标正文。

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Result;
use serde_json::{json, Map, Value};

use super::model::{
    self, collect_present_signatures, collect_signatures, count_removed_present, TargetSpec,
};
use super::persistence::{load_bindings_from, Binding};
use crate::native::Context;
use crate::native_mcp::targets::{check_symlink_metadata, validate_target_path};
use crate::native_sources;
use crate::store;

pub(crate) fn target_enabled(policy: &Value, agent_id: &str) -> bool {
    let defs = policy
        .get("agents")
        .and_then(|v| v.get("definitions"))
        .and_then(Value::as_object);
    let Some(defs) = defs else {
        return false;
    };
    let Some(def) = defs.get(agent_id) else {
        return false;
    };
    if !def.is_object() {
        return false;
    }
    if def.get("_delete") == Some(&Value::Bool(true)) {
        return false;
    }
    policy
        .get("agents")
        .and_then(|v| v.get(agent_id))
        .and_then(|v| v.get("enabled"))
        .and_then(Value::as_bool)
        != Some(false)
}

struct Snapshot {
    policy: Value,
    sources: BTreeMap<String, String>,
    bindings: BTreeMap<String, Binding>,
}

fn load_snapshot(ctx: &Context) -> Result<Snapshot> {
    match store::open_readonly(ctx)? {
        Some(mut conn) => {
            // 显式 Deferred 只读事务：三个 SELECT 在同一 SQLite snapshot 上执行；
            // 不抢写锁，不写 DB。任一读取失败即传播 Err，事务随之回滚。
            let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Deferred)?;
            let snap = Snapshot {
                policy: store::policy_from_connection(&tx)?,
                sources: native_sources::current_hook_sources_from(&tx)?,
                bindings: load_bindings_from(&tx)?,
            };
            tx.commit()?;
            Ok(snap)
        }
        None => Ok(Snapshot {
            policy: store::builtin_policy(),
            sources: native_sources::current_hook_sources_readonly(ctx)?,
            bindings: BTreeMap::new(),
        }),
    }
}

/// 解析并校验来源 canonical JSON；损坏 fail-closed，错误不回显正文。
pub(crate) fn parse_source(
    spec: &TargetSpec,
    canonical: &str,
) -> Result<(model::HookMap, Vec<model::RemovedRule>)> {
    let value: Value = serde_json::from_str(canonical)
        .map_err(|_| anyhow::anyhow!("hooks 来源 {} 不是合法 JSON 对象", spec.source_key))?;
    let obj = value
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("hooks 来源 {} 不是合法 JSON 对象", spec.source_key))?;
    model::validate_source(spec.source_key, obj)
        .map_err(|err| anyhow::anyhow!("hooks 来源 {} 结构非法: {err:#}", spec.source_key))
}

enum TargetProbe {
    Missing,
    Present {
        bytes: Vec<u8>,
        json: Value,
    },
    Blocked {
        exists: Option<bool>,
        reason: String,
    },
}

/// 只读探测目标：路径安全校验 + JSON 对象/结构检查；绝不写文件。
fn probe_target(abs_path: &Path) -> TargetProbe {
    if let Err(err) = validate_target_path(abs_path) {
        return TargetProbe::Blocked {
            exists: None,
            reason: format!("{err:#}"),
        };
    }
    let meta = match check_symlink_metadata(abs_path) {
        Ok(Some(meta)) => meta,
        Ok(None) => return TargetProbe::Missing,
        Err(err) => {
            return TargetProbe::Blocked {
                exists: None,
                reason: format!("{err:#}"),
            }
        }
    };
    if !meta.is_file() {
        return TargetProbe::Blocked {
            exists: Some(true),
            reason: "目标路径不是普通文件".to_string(),
        };
    }
    let bytes = match std::fs::read(abs_path) {
        Ok(bytes) => bytes,
        Err(err) => {
            return TargetProbe::Blocked {
                exists: Some(true),
                reason: format!("无法读取目标文件: {err}"),
            }
        }
    };
    let json: Value = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        Err(_) => {
            return TargetProbe::Blocked {
                exists: Some(true),
                reason: "目标文件不是合法 JSON".to_string(),
            }
        }
    };
    if !json.is_object() {
        return TargetProbe::Blocked {
            exists: Some(true),
            reason: "目标文件 JSON 不是对象".to_string(),
        };
    }
    if let Err(err) = model::validate_target_hooks(&json) {
        return TargetProbe::Blocked {
            exists: Some(true),
            reason: format!("{err:#}"),
        };
    }
    TargetProbe::Present { bytes, json }
}

/// 计算期望渲染（Claude 依赖现有内容；缺失/损坏按空 settings 渲染）。
pub(crate) fn expected_render(
    spec: &TargetSpec,
    managed_hooks: &model::HookMap,
    removed: &[model::RemovedRule],
    current: Option<&Value>,
) -> Value {
    match spec.merge {
        "signature-set" => model::claude_render(
            current.unwrap_or(&Value::Object(Map::new())),
            managed_hooks,
            removed,
        ),
        _ => model::codex_render(managed_hooks),
    }
}

fn freshness_json() -> Value {
    json!({
        "state": "not-applicable",
        "current": Value::Null,
        "upstream": Value::Null,
        "checked_at": Value::Null,
        "probe": "native",
        "cache_age_sec": Value::Null,
    })
}

fn sync_action() -> Value {
    json!([{ "id": "sync", "risk": "write-local", "needs_lock": true, "offline_ok": true }])
}

fn build_element(ctx: &Context, snap: &Snapshot, spec: &TargetSpec) -> Result<Value> {
    let enabled = target_enabled(&snap.policy, spec.agent_id);
    let canonical = &snap.sources[spec.source_key];
    let (managed_hooks, removed) = parse_source(spec, canonical)?;
    let source_revision = crate::native_mcp::model::sha256_hex(canonical);
    let managed_signatures = collect_signatures(&managed_hooks);
    let managed_hook_count = managed_signatures.len() as u64;
    let binding = snap.bindings.get(spec.target_id);
    let applied_digest = binding.map(|b| b.applied_digest.clone());

    let abs_target = ctx.home_dir.join(spec.target);
    let abs_display = abs_target.display().to_string();

    if !enabled {
        // disabled：绝不探测路径。expected_digest 基于空 settings 计算，保持确定性。
        let expected = expected_render(spec, &managed_hooks, &removed, None);
        let expected_digest = model::digest_bytes(&model::render_bytes(&expected));
        return Ok(json!({
            "id": spec.target_id,
            "domain": "hooks",
            "spec": spec_json(spec),
            "policy": { "enabled": false, "agent_id": spec.agent_id },
            "observed": {
                "target": abs_display,
                "target_exists": Value::Null,
                "expected_digest": expected_digest,
                "applied_digest": applied_digest,
                "actual_digest": Value::Null,
                "source_revision": source_revision,
                "blocked_reason": Value::Null,
                "registration_needed": false,
                "managed_hook_count": managed_hook_count,
                "missing_signatures": 0,
                "removed_signatures_present": 0,
            },
            "state": "disabled",
            "freshness": freshness_json(),
            "actions": [],
        }));
    }

    let probe = probe_target(&abs_target);
    let mut actual_digest_out = Value::Null;
    let (
        state,
        target_exists,
        blocked_reason,
        registration_needed,
        missing_signatures,
        removed_present,
        has_sync,
        expected_digest,
    ) = match probe {
        TargetProbe::Blocked { exists, reason } => (
            "unmanaged-detected",
            exists.map_or(Value::Null, Value::Bool),
            Value::String(reason),
            false,
            0u64,
            0u64,
            false,
            {
                let expected = expected_render(spec, &managed_hooks, &removed, None);
                model::digest_bytes(&model::render_bytes(&expected))
            },
        ),
        TargetProbe::Missing => {
            let expected = expected_render(spec, &managed_hooks, &removed, None);
            let expected_digest = model::digest_bytes(&model::render_bytes(&expected));
            if managed_signatures.is_empty() {
                // 无 managed hooks（removed 仅清理已有文件）：不建空文件。
                (
                    "managed-ok",
                    Value::Bool(false),
                    Value::Null,
                    false,
                    0,
                    0,
                    false,
                    expected_digest,
                )
            } else {
                (
                    "missing",
                    Value::Bool(false),
                    Value::Null,
                    false,
                    managed_hook_count,
                    0,
                    true,
                    expected_digest,
                )
            }
        }
        TargetProbe::Present {
            bytes,
            json: current,
        } => {
            let actual_digest = model::digest_bytes(&bytes);
            actual_digest_out = Value::String(actual_digest.clone());
            let expected = expected_render(spec, &managed_hooks, &removed, Some(&current));
            let expected_bytes = model::render_bytes(&expected);
            let expected_digest = model::digest_bytes(&expected_bytes);
            let hooks_value = current
                .get("hooks")
                .cloned()
                .unwrap_or(Value::Object(Map::new()));
            let present = collect_present_signatures(&hooks_value);
            let missing = managed_signatures
                .iter()
                .filter(|s| !present.contains(s))
                .count() as u64;
            let removed_present = count_removed_present(&removed, &present) as u64;
            match binding {
                Some(b) => {
                    if actual_digest != b.applied_digest {
                        (
                            "unmanaged-detected",
                            Value::Bool(true),
                            Value::String("目标文件存在外部修改，拒绝覆盖".to_string()),
                            false,
                            missing,
                            removed_present,
                            false,
                            expected_digest,
                        )
                    } else if actual_digest != expected_digest {
                        (
                            "drifted",
                            Value::Bool(true),
                            Value::Null,
                            false,
                            missing,
                            removed_present,
                            true,
                            expected_digest,
                        )
                    } else {
                        (
                            "managed-ok",
                            Value::Bool(true),
                            Value::Null,
                            false,
                            missing,
                            removed_present,
                            false,
                            expected_digest,
                        )
                    }
                }
                None => {
                    if spec.merge == "whole-file" {
                        // Codex：仅当实际 JSON 语义与整文件期望相等才允许登记。
                        if current == expected {
                            (
                                "drifted",
                                Value::Bool(true),
                                Value::Null,
                                true,
                                missing,
                                removed_present,
                                true,
                                expected_digest,
                            )
                        } else {
                            (
                                "unmanaged-detected",
                                Value::Bool(true),
                                Value::String(
                                    "目标文件已存在外部内容且无受管绑定，拒绝覆盖".to_string(),
                                ),
                                false,
                                missing,
                                removed_present,
                                false,
                                expected_digest,
                            )
                        }
                    } else if current != expected {
                        // Claude：按签名集合可安全合并有效 JSON。
                        (
                            "drifted",
                            Value::Bool(true),
                            Value::Null,
                            false,
                            missing,
                            removed_present,
                            true,
                            expected_digest,
                        )
                    } else {
                        (
                            "drifted",
                            Value::Bool(true),
                            Value::Null,
                            true,
                            missing,
                            removed_present,
                            true,
                            expected_digest,
                        )
                    }
                }
            }
        }
    };

    let actions = if has_sync { sync_action() } else { json!([]) };
    Ok(json!({
        "id": spec.target_id,
        "domain": "hooks",
        "spec": spec_json(spec),
        "policy": { "enabled": true, "agent_id": spec.agent_id },
        "observed": {
            "target": abs_display,
            "target_exists": target_exists,
            "expected_digest": expected_digest,
            "applied_digest": applied_digest,
            "actual_digest": actual_digest_out,
            "source_revision": source_revision,
            "blocked_reason": blocked_reason,
            "registration_needed": registration_needed,
            "managed_hook_count": managed_hook_count,
            "missing_signatures": missing_signatures,
            "removed_signatures_present": removed_present,
        },
        "state": state,
        "freshness": freshness_json(),
        "actions": actions,
    }))
}

fn spec_json(spec: &TargetSpec) -> Value {
    json!({
        "name": spec.name,
        "merge": spec.merge,
        "source_key": spec.source_key,
        "target": format!("~/{}", spec.target),
    })
}

pub(crate) fn build_status(ctx: &Context) -> Result<Value> {
    let snap = load_snapshot(ctx)?;
    let mut elements = Vec::new();
    for spec in model::targets() {
        elements.push(build_element(ctx, &snap, &spec)?);
    }
    Ok(json!({
        "domain": "hooks",
        "elements": elements,
    }))
}
