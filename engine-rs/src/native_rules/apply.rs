//! rules 域 apply 核心：preflight 纯读校验 + 单 Immediate 事务内重检、
//! 原子写入、binding 登记与单条审计。任何阻塞（外部修改/不安全路径/
//! 来源或绑定损坏）fail-closed；失败时 RollbackGuard 恢复文件、DB 事务
//! 整体回滚。错误与审计只含 digest/revision 元数据，严禁规则正文。

use anyhow::{anyhow, bail, Result};
use rusqlite::TransactionBehavior;
use serde_json::{json, Value};

use super::model::{render_rules, Binding};
use super::{args, preflight, status};
use crate::native::{Context, Request};
use crate::native_mcp::model::{now_iso, sha256_hex};
use crate::native_mcp::rollback::{atomic_write_bytes_checked, read_existing_bytes, RollbackGuard};
use crate::native_mcp::targets::validate_target_path;
use crate::native_sources::{self, KEY_SHARED};
use crate::store;

/// 锁前预检：严格解析 + 私有 preflight 纯读校验。Ok(()) 表示合法 apply
///（execute 稍后必须取得全局 apply lock）；参数无效（含已退役的
/// --dry-run）或目标阻塞即 Err。无效/阻塞预检不创建 data/home/DB/表/文件。
pub(crate) fn preflight_apply(ctx: &Context, req: &Request) -> Result<()> {
    let parsed = args::parse_apply_args(req)?;
    preflight::build_preflight(ctx, &parsed)?;
    Ok(())
}

enum Decision {
    Skip,
    Write {
        expected: Vec<u8>,
        old_bytes: Option<Vec<u8>>,
        binding: Binding,
    },
}

fn result_json(steps: Vec<Value>, skipped: Vec<String>) -> Value {
    json!({
        "domain": "rules",
        "steps": steps,
        "skipped": skipped,
    })
}

pub(crate) fn apply(ctx: &Context, req: &Request) -> Result<Value> {
    // 自身再次解析 + 等价 preflight 预检：直接调用 apply 不能绕过安全校验。
    let parsed = args::parse_apply_args(req)?;
    preflight::build_preflight(ctx, &parsed)?;

    let selected = parsed.only.clone();

    let mut conn = store::open_write(ctx)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

    // Immediate 事务内基于同一 snapshot 重新读取并重检，不沿用锁前
    // preflight 的正文/摘要。
    let policy = store::policy_from_connection(&tx)?;
    let sources = native_sources::current_sources_from(&tx)?;
    let bindings = super::persistence::load_bindings_from(&tx)?;

    let mut skipped: Vec<String> = Vec::new();
    let mut pending: Vec<(String, std::path::PathBuf, Decision)> = Vec::new();

    for spec in super::model::targets() {
        if !selected.iter().any(|id| id == spec.target_id) {
            continue;
        }
        if !status::target_enabled(&policy, spec.agent_id) {
            // disabled：绝不探测路径。
            skipped.push(spec.target_id.to_string());
            continue;
        }
        let shared = &sources[KEY_SHARED];
        let adapter = &sources[spec.adapter_key];
        let expected = render_rules(shared, adapter);
        let expected_digest = sha256_hex(&expected);
        let shared_revision = sha256_hex(shared);
        let adapter_revision = sha256_hex(adapter);
        let binding = bindings.get(spec.target_id);

        let abs = ctx.home_dir.join(spec.target);
        validate_target_path(&abs)?;
        // 只读取一次旧字节，作为本次写入的 CAS 基准。
        let old_bytes = read_existing_bytes(&abs)?;
        let actual_digest = old_bytes
            .as_deref()
            .map(crate::native_mcp::model::sha256_hex_bytes);

        let decision = match (&actual_digest, binding) {
            (None, _) => Decision::Write {
                expected: expected.into_bytes(),
                old_bytes,
                binding: Binding {
                    target_id: spec.target_id.to_string(),
                    applied_digest: expected_digest,
                    shared_revision,
                    adapter_revision,
                    updated_at: now_iso(),
                },
            },
            (Some(actual), Some(b)) => {
                if *actual != b.applied_digest && !parsed.allow_takeover {
                    bail!("目标 {} 存在外部修改，拒绝覆盖", spec.target_id);
                }
                if *actual == expected_digest {
                    skipped.push(spec.target_id.to_string());
                    Decision::Skip
                } else {
                    Decision::Write {
                        expected: expected.into_bytes(),
                        old_bytes,
                        binding: Binding {
                            target_id: spec.target_id.to_string(),
                            applied_digest: expected_digest,
                            shared_revision,
                            adapter_revision,
                            updated_at: now_iso(),
                        },
                    }
                }
            }
            (Some(actual), None) => {
                if *actual == expected_digest {
                    // 字节一致但无绑定：登记受管绑定。
                    Decision::Write {
                        expected: expected.into_bytes(),
                        old_bytes,
                        binding: Binding {
                            target_id: spec.target_id.to_string(),
                            applied_digest: expected_digest,
                            shared_revision,
                            adapter_revision,
                            updated_at: now_iso(),
                        },
                    }
                } else if parsed.allow_takeover {
                    // 显式接管：用本工具的内容整体替换现有文件，并登记受管绑定。
                    Decision::Write {
                        expected: expected.into_bytes(),
                        old_bytes,
                        binding: Binding {
                            target_id: spec.target_id.to_string(),
                            applied_digest: expected_digest,
                            shared_revision,
                            adapter_revision,
                            updated_at: now_iso(),
                        },
                    }
                } else {
                    bail!("目标 {} 已有外部内容，拒绝覆盖", spec.target_id);
                }
            }
        };
        if !matches!(decision, Decision::Skip) {
            pending.push((spec.target_id.to_string(), abs, decision));
        }
    }

    // 没有任何待写：直接提交（不建表、不写审计）。
    if pending.is_empty() {
        tx.commit()?;
        return Ok(result_json(vec![], selected));
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
                "name": format!("rules:sync {id}"),
                "ok": true,
                "exit_code": 0,
            }));
            audit_targets.push(json!({
                "target_id": binding.target_id,
                "applied_digest": binding.applied_digest,
                "shared_revision": binding.shared_revision,
                "adapter_revision": binding.adapter_revision,
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
    let audit_target = if ids.len() == 1 { ids[0] } else { "rules" };
    let audit_result = store::record_operation(
        &tx,
        Some(&req.run_id),
        "rules:apply",
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

    Ok(result_json(steps, skipped))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_rules::model::targets;
    use crate::native_sources::KEY_SHARED;
    use std::path::Path;

    fn temp_root(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "native-rules-apply-test-{}-{}-{:x}",
            std::process::id(),
            tag,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn ctx_at(root: &Path) -> Context {
        Context {
            data_dir: root.join("data"),
            home_dir: root.join("home"),
        }
    }

    fn drop_root(root: &Path) {
        let _ = std::fs::remove_dir_all(root);
    }

    fn req(args: &[&str]) -> Request {
        Request {
            command: "rules:apply".to_string(),
            args: args.iter().map(|s| s.to_string()).collect(),
            run_id: "run-apply-test".to_string(),
        }
    }

    fn write_policy_disabled_agent(ctx: &Context, agent_id: &str) {
        let mut policy = store::builtin_policy();
        policy["agents"][agent_id]["enabled"] = Value::Bool(false);
        let conn = store::open_write(ctx).unwrap();
        store::write_policy(&conn, &policy).unwrap();
    }

    fn write_source(ctx: &Context, key: &str, value: &str) {
        let conn = store::open_write(ctx).unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS rule_sources (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL,
                revision TEXT NOT NULL,
                updated_at TEXT NOT NULL
            )",
        )
        .unwrap();
        let revision = sha256_hex(value);
        conn.execute(
            "INSERT INTO rule_sources (key, value, revision, updated_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, revision = excluded.revision, updated_at = excluded.updated_at",
            rusqlite::params![key, value, revision, "2026-09-12T00:00:00Z"],
        )
        .unwrap();
    }

    fn current_rendered(ctx: &Context, spec: &super::super::model::TargetSpec) -> String {
        let conn = store::open_readonly(ctx).unwrap();
        let (shared, adapter) = match conn {
            Some(c) => {
                let sources = native_sources::current_sources_from(&c).unwrap();
                (
                    sources[KEY_SHARED].clone(),
                    sources[spec.adapter_key].clone(),
                )
            }
            None => (
                native_sources::default_value(KEY_SHARED)
                    .unwrap()
                    .to_string(),
                native_sources::default_value(spec.adapter_key)
                    .unwrap()
                    .to_string(),
            ),
        };
        render_rules(&shared, &adapter)
    }

    fn read_binding(ctx: &Context, target_id: &str) -> Option<Binding> {
        let conn = store::open_readonly(ctx).unwrap()?;
        super::super::persistence::load_bindings_from(&conn)
            .unwrap()
            .remove(target_id)
    }

    fn operation_log_rows(ctx: &Context) -> Vec<(String, Option<String>, Option<String>)> {
        let Some(conn) = store::open_readonly(ctx).unwrap() else {
            return Vec::new();
        };
        let mut stmt = conn
            .prepare("SELECT operation_type, target, detail_json FROM operation_logs WHERE operation_type = 'rules:apply'")
            .unwrap();
        stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, Option<String>>(2)?,
            ))
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
    }

    #[test]
    fn dry_run_is_rejected_before_any_write_and_creates_nothing() {
        let root = temp_root("dryrun");
        let ctx = ctx_at(&root);
        let err = apply(&ctx, &req(&["--dry-run"])).unwrap_err();
        assert!(format!("{err:#}").contains("参数错误"));
        assert!(preflight_apply(&ctx, &req(&["--dry-run"])).is_err());
        assert!(!ctx.data_dir.exists());
        assert!(!ctx.home_dir.exists());
        drop_root(&root);
    }

    #[test]
    fn invalid_or_blocked_apply_creates_nothing() {
        let root = temp_root("invalid");
        let ctx = ctx_at(&root);
        assert!(apply(&ctx, &req(&["--bogus"])).is_err());
        assert!(apply(&ctx, &req(&["--only", "rules:unknown"])).is_err());
        assert!(!ctx.data_dir.exists());
        assert!(!ctx.home_dir.exists());

        // 阻塞：外部内容 + 无绑定，preflight 预检 fail-closed。
        std::fs::create_dir_all(ctx.home_dir.join(".codex")).unwrap();
        std::fs::write(
            ctx.home_dir.join(".codex/AGENTS.md"),
            "external body marker\n",
        )
        .unwrap();
        let err = apply(&ctx, &req(&[])).unwrap_err();
        assert!(!format!("{err:#}").contains("external body marker"));
        assert!(!ctx.data_dir.exists());
        drop_root(&root);
    }

    #[test]
    fn only_writes_selected_and_leaves_other_sentinel() {
        let root = temp_root("only");
        let ctx = ctx_at(&root);
        std::fs::create_dir_all(ctx.home_dir.join(".claude")).unwrap();
        std::fs::write(ctx.home_dir.join(".claude/CLAUDE.md"), "sentinel\n").unwrap();

        let result = apply(&ctx, &req(&["--only", "rules:codex_global"])).unwrap();
        let steps = result["steps"].as_array().unwrap();
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0]["name"], "rules:sync rules:codex_global");
        assert_eq!(result["skipped"], json!([]));

        let spec = &targets()[0];
        assert_eq!(
            std::fs::read(ctx.home_dir.join(spec.target)).unwrap(),
            current_rendered(&ctx, spec).into_bytes()
        );
        assert_eq!(
            std::fs::read(ctx.home_dir.join(".claude/CLAUDE.md")).unwrap(),
            b"sentinel\n"
        );
        drop_root(&root);
    }

    #[test]
    fn new_file_bytes_mode_binding_and_audit_are_exact() {
        let root = temp_root("fresh");
        let ctx = ctx_at(&root);
        let marker = "AUDIT-BODY-LEAK-MARKER-9c4e1b";
        write_source(&ctx, KEY_SHARED, &format!("{marker}\n"));
        let result = apply(&ctx, &req(&["--only", "rules:codex_global"])).unwrap();
        assert_eq!(result["steps"].as_array().unwrap().len(), 1);
        assert!(
            !serde_json::to_string(&result).unwrap().contains(marker),
            "ApplyResultData 泄漏规则正文"
        );

        let spec = &targets()[0];
        let rendered = current_rendered(&ctx, spec);
        assert!(rendered.contains(marker), "渲染正文必须包含 marker");
        let bytes = std::fs::read(ctx.home_dir.join(spec.target)).unwrap();
        assert_eq!(bytes, rendered.as_bytes());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(ctx.home_dir.join(spec.target))
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);
        }

        let binding = read_binding(&ctx, "rules:codex_global").unwrap();
        assert_eq!(binding.applied_digest, sha256_hex(&rendered));
        assert_eq!(binding.shared_revision, sha256_hex(&format!("{marker}\n")));
        assert!(!binding.updated_at.is_empty());
        for field in [
            &binding.target_id,
            &binding.applied_digest,
            &binding.shared_revision,
            &binding.adapter_revision,
            &binding.updated_at,
        ] {
            assert!(!field.contains(marker), "Binding 字段泄漏规则正文");
        }

        let logs = operation_log_rows(&ctx);
        assert_eq!(logs.len(), 1);
        let (op, target, detail) = &logs[0];
        assert_eq!(op, "rules:apply");
        assert_eq!(target.as_deref(), Some("rules:codex_global"));
        let detail = detail.as_deref().unwrap_or("");
        assert!(!detail.contains(marker), "审计 detail_json 泄漏规则正文");
        assert!(!detail.contains(rendered.as_str()));
        drop_root(&root);
    }

    #[test]
    fn second_apply_is_noop_with_empty_steps_and_no_new_audit() {
        let root = temp_root("noop");
        let ctx = ctx_at(&root);
        apply(&ctx, &req(&[])).unwrap();
        let logs_before = operation_log_rows(&ctx).len();
        let result = apply(&ctx, &req(&[])).unwrap();
        assert_eq!(result["steps"], json!([]));
        assert_eq!(
            result["skipped"],
            json!(["rules:codex_global", "rules:claude_global"])
        );
        assert_eq!(operation_log_rows(&ctx).len(), logs_before);
        drop_root(&root);
    }

    #[test]
    fn source_change_updates_file_and_binding() {
        let root = temp_root("update");
        let ctx = ctx_at(&root);
        apply(&ctx, &req(&["--only", "rules:codex_global"])).unwrap();
        write_source(&ctx, KEY_SHARED, "shared v2 marker\n");

        let result = apply(&ctx, &req(&["--only", "rules:codex_global"])).unwrap();
        assert_eq!(result["steps"].as_array().unwrap().len(), 1);
        let spec = &targets()[0];
        let rendered = current_rendered(&ctx, spec);
        assert_eq!(
            std::fs::read(ctx.home_dir.join(spec.target)).unwrap(),
            rendered.as_bytes()
        );
        let binding = read_binding(&ctx, "rules:codex_global").unwrap();
        assert_eq!(binding.applied_digest, sha256_hex(&rendered));
        drop_root(&root);
    }

    #[test]
    fn external_modification_is_rejected_keeping_file_and_binding() {
        let root = temp_root("tamper");
        let ctx = ctx_at(&root);
        apply(&ctx, &req(&["--only", "rules:codex_global"])).unwrap();
        let binding_before = read_binding(&ctx, "rules:codex_global").unwrap();
        let path = ctx.home_dir.join(targets()[0].target);
        std::fs::write(&path, "tampered external\n").unwrap();

        let err = apply(&ctx, &req(&["--only", "rules:codex_global"])).unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("外部修改") || msg.contains("preflight 拒绝"),
            "fail-closed 消息不符: {msg}"
        );
        assert!(!msg.contains("tampered external"));
        assert_eq!(std::fs::read(&path).unwrap(), b"tampered external\n");
        assert_eq!(
            read_binding(&ctx, "rules:codex_global").unwrap(),
            binding_before
        );
        drop_root(&root);
    }

    #[test]
    fn disabled_target_is_skipped_without_probing_or_writing() {
        let root = temp_root("disabled");
        let ctx = ctx_at(&root);
        write_policy_disabled_agent(&ctx, "codex");
        let result = apply(&ctx, &req(&[])).unwrap();
        assert_eq!(result["skipped"], json!(["rules:codex_global"]));
        // 产品只支持 Claude/Codex；Codex 停用后只写 Claude。
        assert_eq!(result["steps"].as_array().unwrap().len(), 1);
        assert!(!ctx.home_dir.join(".codex").exists());
        assert!(read_binding(&ctx, "rules:codex_global").is_none());
        assert!(read_binding(&ctx, "rules:claude_global").is_some());
        drop_root(&root);
    }

    #[test]
    fn late_db_failure_rolls_back_files_bindings_and_audit() {
        let root = temp_root("rollback");
        let ctx = ctx_at(&root);
        // 先写好 claude，让第二次 apply 覆盖两个目标的更新/登记路径。
        apply(&ctx, &req(&["--only", "rules:claude_global"])).unwrap();
        write_source(&ctx, KEY_SHARED, "shared v3 rollback\n");
        let claude_bytes_before = std::fs::read(ctx.home_dir.join(".claude/CLAUDE.md")).unwrap();

        // 末端失败注入：审计写入 operation_logs 时触发。
        {
            let conn = store::open_write(&ctx).unwrap();
            conn.execute_batch(
                "CREATE TRIGGER fail_operation_logs BEFORE INSERT ON operation_logs
                 BEGIN SELECT RAISE(ABORT, 'injected-end-failure'); END;",
            )
            .unwrap();
        }

        let err = apply(&ctx, &req(&[])).unwrap_err();
        assert!(format!("{err:#}").contains("injected-end-failure"));

        // 两个文件均恢复：codex 回到缺失，claude 回到旧 bytes。
        assert!(!ctx.home_dir.join(".codex/AGENTS.md").exists());
        assert_eq!(
            std::fs::read(ctx.home_dir.join(".claude/CLAUDE.md")).unwrap(),
            claude_bytes_before
        );
        // 绑定与审计均回滚。
        assert!(read_binding(&ctx, "rules:codex_global").is_none());
        let claude_binding = read_binding(&ctx, "rules:claude_global").unwrap();
        assert_ne!(
            claude_binding.shared_revision,
            sha256_hex("shared v3 rollback\n")
        );
        let logs = operation_log_rows(&ctx);
        assert_eq!(logs.len(), 1, "注入失败后不得新增审计");
        assert!(!logs[0].2.as_deref().unwrap_or("").contains("shared v3"));
        drop_root(&root);
    }

    #[test]
    fn result_schema_matches_apply_result_contract() {
        let root = temp_root("schema");
        let ctx = ctx_at(&root);
        let result = apply(&ctx, &req(&["--only", "rules:codex_global"])).unwrap();
        for key in ["domain", "steps", "skipped"] {
            assert!(result.get(key).is_some(), "缺字段 {key}");
        }
        assert!(
            result.get("dry_run").is_none(),
            "apply 输出不再携带 dry_run 字段"
        );
        assert!(
            result.get("snapshot").is_none(),
            "apply 输出不再携带 snapshot 字段"
        );
        assert_eq!(result["domain"], "rules");
        assert!(result["steps"].is_array());
        assert!(result["skipped"].is_array());
        for step in result["steps"].as_array().unwrap() {
            for key in ["name", "ok", "exit_code"] {
                assert!(step.get(key).is_some(), "step 缺字段 {key}");
            }
            assert_eq!(step["ok"], true);
            assert_eq!(step["exit_code"], 0);
        }
        drop_root(&root);
    }
}
