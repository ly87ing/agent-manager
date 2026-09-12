//! hooks 域 apply 核心：preflight 纯读校验 + 单 Immediate 事务内重检、
//! 原子写入、binding 登记与单条审计。任何阻塞（外部修改/不安全路径/
//! 来源或绑定损坏/目标 JSON 损坏）fail-closed；失败时 RollbackGuard 恢复
//! 文件、DB 事务整体回滚。错误与审计只含 digest/revision 元数据。

use anyhow::{anyhow, bail, Result};
use rusqlite::TransactionBehavior;
use serde_json::{json, Value};

use super::model::{self, TargetSpec};
use super::persistence::Binding;
use super::{args, plan, status};
use crate::native::{Context, Request};
use crate::native_mcp::model::now_iso;
use crate::native_mcp::rollback::{atomic_write_bytes_checked, read_existing_bytes, RollbackGuard};
use crate::native_mcp::targets::validate_target_path;
use crate::store;

/// 构造等价 hooks:plan 请求：保留归一后的 --only 与可选 --offline，
/// 去掉 --dry-run。
fn equivalent_plan_request(req: &Request, parsed: &args::HookArgs) -> Request {
    let mut plan_args: Vec<String> = Vec::new();
    for id in &parsed.only {
        plan_args.push("--only".to_string());
        plan_args.push(id.clone());
    }
    if parsed.offline {
        plan_args.push("--offline".to_string());
    }
    Request {
        command: "hooks:plan".to_string(),
        args: plan_args,
        run_id: req.run_id.clone(),
    }
}

/// 预检：严格解析 + 等价 plan 纯读校验。返回 true 表示合法非 dry-run
/// apply（execute 稍后必须取得全局 apply lock）；dry-run 返回 false。
/// 无效/阻塞预检不创建 data/home/DB/表/文件。
pub(crate) fn preflight_apply(ctx: &Context, req: &Request) -> Result<bool> {
    let parsed = args::parse_apply_args(req)?;
    let plan_req = equivalent_plan_request(req, &parsed);
    plan::build_plan(ctx, &plan_req)?;
    Ok(!parsed.dry_run)
}

enum Decision {
    Skip,
    Write {
        expected: Vec<u8>,
        old_bytes: Option<Vec<u8>>,
        binding: Binding,
    },
}

fn result_json(dry_run: bool, steps: Vec<Value>, skipped: Vec<String>) -> Value {
    json!({
        "domain": "hooks",
        "dry_run": dry_run,
        "steps": steps,
        "snapshot": Value::Null,
        "skipped": skipped,
    })
}

/// 单目标判定：返回 Skip 或 Write。任何外部修改/结构损坏 fail-closed。
fn decide(
    ctx: &Context,
    spec: &TargetSpec,
    sources: &std::collections::BTreeMap<String, String>,
    bindings: &std::collections::BTreeMap<String, Binding>,
) -> Result<Decision> {
    let canonical = &sources[spec.source_key];
    let (managed_hooks, removed) = status::parse_source(spec, canonical)?;
    let source_revision = crate::native_mcp::model::sha256_hex(canonical);
    let managed_signatures = model::collect_signatures(&managed_hooks);

    let abs = ctx.home_dir.join(spec.target);
    validate_target_path(&abs)?;
    let old_bytes = read_existing_bytes(&abs)?;
    let binding = bindings.get(spec.target_id);

    let make_binding = |applied_digest: String| Binding {
        target_id: spec.target_id.to_string(),
        applied_digest,
        source_revision: source_revision.clone(),
        updated_at: now_iso(),
    };

    let Some(old) = old_bytes else {
        if managed_signatures.is_empty() {
            // 缺失且无 managed hooks（removed 仅清理已有文件）：不建空文件。
            return Ok(Decision::Skip);
        }
        let expected_value = status::expected_render(spec, &managed_hooks, &removed, None);
        let expected = model::render_bytes(&expected_value);
        let applied = model::digest_bytes(&expected);
        return Ok(Decision::Write {
            expected,
            old_bytes: None,
            binding: make_binding(applied),
        });
    };

    let current: Value = serde_json::from_slice::<Value>(&old)
        .ok()
        .filter(|v| v.is_object())
        .ok_or_else(|| anyhow!("目标 {} 不是合法 JSON 对象，拒绝覆盖", spec.target_id))?;
    // 锁内重读后与 status 探测使用同一严格子树校验，防止 preflight 后
    // 结构被换坏仍容错合并。
    model::validate_target_hooks(&current)
        .map_err(|err| anyhow!("目标 {} hooks 结构非法，拒绝覆盖: {err:#}", spec.target_id))?;
    let expected_value = status::expected_render(spec, &managed_hooks, &removed, Some(&current));
    let expected = model::render_bytes(&expected_value);
    let expected_digest = model::digest_bytes(&expected);
    let actual_digest = model::digest_bytes(&old);

    match binding {
        Some(b) => {
            if actual_digest != b.applied_digest {
                bail!("目标 {} 存在外部修改，拒绝覆盖", spec.target_id);
            }
            if actual_digest == expected_digest {
                Ok(Decision::Skip)
            } else {
                Ok(Decision::Write {
                    expected,
                    old_bytes: Some(old),
                    binding: make_binding(expected_digest),
                })
            }
        }
        None => {
            if spec.merge == "whole-file" {
                // Codex：仅当实际 JSON 语义与整文件期望相等才允许登记绑定。
                if current != expected_value {
                    bail!("目标 {} 已有外部内容且无受管绑定，拒绝覆盖", spec.target_id);
                }
                Ok(Decision::Write {
                    expected,
                    old_bytes: Some(old),
                    binding: make_binding(expected_digest),
                })
            } else {
                // Claude：按签名集合可安全合并有效 JSON（含语义已符合的登记）。
                Ok(Decision::Write {
                    expected,
                    old_bytes: Some(old),
                    binding: make_binding(expected_digest),
                })
            }
        }
    }
}

pub(crate) fn apply(ctx: &Context, req: &Request) -> Result<Value> {
    // 自身再次解析 + 等价 plan 预检：直接调用 apply 不能绕过安全校验。
    let parsed = args::parse_apply_args(req)?;
    let plan_req = equivalent_plan_request(req, &parsed);
    plan::build_plan(ctx, &plan_req)?;

    let selected = parsed.only.clone();
    if parsed.dry_run {
        return Ok(result_json(true, vec![], selected));
    }

    let mut conn = store::open_write(ctx)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

    // Immediate 事务内基于同一 snapshot 重新读取并重检，不沿用锁前
    // plan 的正文/摘要。
    let policy = store::policy_from_connection(&tx)?;
    let sources = crate::native_sources::current_hook_sources_from(&tx)?;
    let bindings = super::persistence::load_bindings_from(&tx)?;

    let mut skipped: Vec<String> = Vec::new();
    let mut pending: Vec<(String, std::path::PathBuf, Decision)> = Vec::new();

    for spec in model::targets() {
        if !selected.iter().any(|id| id == spec.target_id) {
            continue;
        }
        if !status::target_enabled(&policy, spec.agent_id) {
            // disabled：绝不探测路径。
            skipped.push(spec.target_id.to_string());
            continue;
        }
        let decision = decide(ctx, &spec, &sources, &bindings)?;
        if !matches!(decision, Decision::Skip) {
            pending.push((
                spec.target_id.to_string(),
                ctx.home_dir.join(spec.target),
                decision,
            ));
        }
    }

    // 没有任何待写：直接提交（不建表、不写审计）。
    if pending.is_empty() {
        tx.commit()?;
        return Ok(result_json(false, vec![], selected));
    }

    super::persistence::ensure_table(&tx)?;
    let mut guard = RollbackGuard::new();

    let run = (|| -> Result<(Vec<Value>, Vec<Value>)> {
        let mut steps = Vec::new();
        let mut audit_targets = Vec::new();
        for (id, abs, decision) in &pending {
            let Decision::Write {
                expected,
                old_bytes,
                binding,
            } = decision
            else {
                continue;
            };
            atomic_write_bytes_checked(abs, expected, old_bytes.as_deref())?;
            guard.record_written(abs, old_bytes.clone(), expected.clone());
            super::persistence::write_binding(&tx, binding)?;
            steps.push(json!({
                "name": format!("hooks:sync {id}"),
                "ok": true,
                "exit_code": 0,
            }));
            audit_targets.push(json!({
                "target_id": binding.target_id,
                "applied_digest": binding.applied_digest,
                "source_revision": binding.source_revision,
            }));
        }
        Ok((steps, audit_targets))
    })();

    let (steps, audit_targets) = match run {
        Ok(v) => v,
        Err(e) => {
            drop(tx);
            if let Err(rb_err) = guard.rollback() {
                return Err(anyhow!("执行失败: {e:#}；配置回滚同时失败: {rb_err:#}"));
            }
            return Err(e);
        }
    };

    let ids: Vec<&str> = pending.iter().map(|(id, _, _)| id.as_str()).collect();
    let audit_target = if ids.len() == 1 { ids[0] } else { "hooks" };
    let audit_result = store::record_operation(
        &tx,
        Some(&req.run_id),
        "hooks:apply",
        Some(audit_target),
        Some(&json!({
            "action": "sync",
            "targets": ids,
            "count": ids.len(),
            "meta": audit_targets,
        })),
    );

    if let Err(e) = audit_result {
        drop(tx);
        if let Err(rb_err) = guard.rollback() {
            return Err(anyhow!("执行失败: {e:#}；配置回滚同时失败: {rb_err:#}"));
        }
        return Err(e.into());
    }

    if let Err(e) = tx.commit() {
        if let Err(rb_err) = guard.rollback() {
            return Err(anyhow!("提交失败: {e:#}；配置回滚同时失败: {rb_err:#}"));
        }
        return Err(e.into());
    }
    guard.commit();

    Ok(result_json(false, steps, skipped))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "native-hooks-apply-unit-{}-{}-{:x}",
            std::process::id(),
            tag,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    /// decide 本身（锁内重判路径）必须复用 validate_target_hooks：
    /// 结构被换坏的目标即使语义可合并也必须拒绝，而非只靠 preflight 名称。
    #[test]
    fn decide_rejects_structurally_invalid_target_before_render() {
        let root = temp_root("decide-strict");
        let _ = std::fs::remove_dir_all(&root);
        let ctx = Context {
            data_dir: root.join("data"),
            home_dir: root.join("home"),
        };
        let settings = ctx.home_dir.join(".claude/settings.json");
        std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
        // 合法 JSON 对象但 hooks 子树非法：hook 缺 command。
        std::fs::write(
            &settings,
            json!({"hooks": {"SessionStart": [{"hooks": [{"type": "command"}]}]}}).to_string(),
        )
        .unwrap();

        let mut sources = std::collections::BTreeMap::new();
        sources.insert(
            "hooks.claude".to_string(),
            "{\"hooks\":{\"SessionStart\":[{\"hooks\":[{\"command\":\"echo a\",\"type\":\"command\"}]}]}}".to_string(),
        );
        sources.insert("hooks.codex".to_string(), "{\"hooks\":{}}".to_string());
        let bindings = std::collections::BTreeMap::new();

        let result = decide(&ctx, &model::TARGET_CLAUDE, &sources, &bindings);
        let err = match result {
            Err(err) => err,
            Ok(_) => panic!("decide 必须拒绝坏结构目标"),
        };
        assert!(
            format!("{err:#}").contains("结构非法"),
            "decide 必须在渲染前拒绝坏结构: {err:#}"
        );
        assert_eq!(
            std::fs::read(&settings).unwrap(),
            json!({"hooks": {"SessionStart": [{"hooks": [{"type": "command"}]}]}})
                .to_string()
                .as_bytes(),
            "目标字节不变"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
