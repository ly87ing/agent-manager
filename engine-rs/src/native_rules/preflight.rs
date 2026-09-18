//! rules 域私有 preflight/action 计算：基于只读 status::build_status
//! 推导 apply 将执行的动作；不创建/修改 data_dir、home_dir、DB、表或
//! 目标文件。选中的 enabled 目标若 unmanaged-detected 或带 blocked_reason，
//! 整个 preflight fail-closed，不输出半份动作；输出与错误均不含规则正文。
//! 该计算只在 apply 锁前/写前预检内部使用，不再对应任何公开命令。

use anyhow::{bail, Result};
use serde_json::{json, Value};

use super::{args, model, status};
use crate::native::Context;

fn action_for(element: &Value, allow_takeover: bool) -> Option<Value> {
    let state = element["state"].as_str().unwrap_or("");
    // 显式接管：unmanaged-detected 且当前内容读得到时也生成写动作，
    // before_digest 取现有内容的摘要，用户因此能看清会被替换掉什么。
    let taking_over = state == "unmanaged-detected" && takeover_eligible(element, allow_takeover);
    if state != "missing" && state != "drifted" && !taking_over {
        return None;
    }
    let observed = &element["observed"];
    let before_digest = if state == "missing" {
        Value::Null
    } else if taking_over {
        observed["actual_digest"].clone()
    } else if let Some(applied) = observed["applied_digest"].as_str() {
        json!(applied)
    } else {
        // 无 binding 的 registration-needed 场景：当前字节即期望字节。
        observed["expected_digest"].clone()
    };
    let id = element["id"].as_str().unwrap_or("");
    Some(json!({
        "op": "write-file",
        "element": id,
        "target": observed["target"],
        "risk": "write-local",
        "before_digest": before_digest,
        "after_digest": observed["expected_digest"],
        "backup_path": Value::Null,
        "cmd": Value::Null,
        "note": format!("同步 {id} 规则文件"),
    }))
}

/// 预检与动作计算：入参为已通过严格解析的 apply 参数；返回动作与汇总
/// （仅测试断言使用，apply 只依赖其 fail-closed 语义）。
/// 能否显式接管：只有「当前内容读得到」的冲突才给接管。路径不安全、读不了的
/// 目标 actual_digest 为 null，一律不提供接管入口。
fn takeover_eligible(element: &Value, allow_takeover: bool) -> bool {
    allow_takeover && element["observed"]["actual_digest"].is_string()
}

pub(crate) fn build_preflight(ctx: &Context, parsed: &args::RuleArgs) -> Result<Value> {
    let status = status::build_status(ctx)?;

    let empty = Vec::new();
    let elements = status["elements"].as_array().unwrap_or(&empty);

    for id in &parsed.only {
        let Some(element) = elements.iter().find(|e| e["id"].as_str() == Some(id)) else {
            continue;
        };
        let enabled = element["policy"]["enabled"].as_bool().unwrap_or(false);
        if enabled
            && (element["state"].as_str() == Some("unmanaged-detected")
                || !element["observed"]["blocked_reason"].is_null())
            && !takeover_eligible(element, parsed.allow_takeover)
        {
            bail!(
                "preflight 拒绝: {} 被本工具以外的方式改过，已停下没有覆盖。要用这里的内容替换它，用「接管原内容」。",
                id
            );
        }
    }

    let mut actions = Vec::new();
    for spec in model::targets() {
        if !parsed.only.iter().any(|id| id == spec.target_id) {
            continue;
        }
        if let Some(element) = elements
            .iter()
            .find(|e| e["id"].as_str() == Some(spec.target_id))
        {
            if let Some(action) = action_for(element, parsed.allow_takeover) {
                actions.push(action);
            }
        }
    }

    let write_count = actions.len();
    Ok(json!({
        "domain": "rules",
        "actions": actions,
        "summary": {
            "read": 0,
            "write-local": write_count,
            "exec-local": 0,
            "network": 0,
            "install-remove": 0,
        },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::Request;
    use crate::native_mcp::model::sha256_hex;
    use crate::native_rules::model::{render_rules, targets, Binding, TargetSpec};
    use crate::native_rules::persistence::ensure_table;
    use crate::native_sources::{self, KEY_SHARED};
    use crate::store;
    use std::path::Path;

    fn temp_root(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "native-rules-preflight-test-{}-{}-{:x}",
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

    fn run_preflight(ctx: &Context, args: &[&str]) -> Result<Value> {
        let req = Request {
            command: "rules:apply".to_string(),
            args: args.iter().map(|s| s.to_string()).collect(),
            run_id: "run-preflight-test".to_string(),
        };
        let parsed = args::parse_apply_args(&req)?;
        build_preflight(ctx, &parsed)
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

    fn write_binding(ctx: &Context, binding: &Binding) {
        let mut conn = store::open_write(ctx).unwrap();
        ensure_table(&conn).unwrap();
        let tx = conn.transaction().unwrap();
        crate::native_rules::persistence::write_binding(&tx, binding).unwrap();
        tx.commit().unwrap();
    }

    fn current_rendered(ctx: &Context, spec: &TargetSpec) -> String {
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

    fn binding_for(ctx: &Context, spec: &TargetSpec, rendered: &str) -> Binding {
        let conn = store::open_readonly(ctx).unwrap();
        let (shared_rev, adapter_rev) = match conn {
            Some(c) => {
                let sources = native_sources::current_sources_from(&c).unwrap();
                (
                    sha256_hex(&sources[KEY_SHARED]),
                    sha256_hex(&sources[spec.adapter_key]),
                )
            }
            None => (
                sha256_hex(native_sources::default_value(KEY_SHARED).unwrap()),
                sha256_hex(native_sources::default_value(spec.adapter_key).unwrap()),
            ),
        };
        Binding {
            target_id: spec.target_id.to_string(),
            applied_digest: sha256_hex(rendered),
            shared_revision: shared_rev,
            adapter_revision: adapter_rev,
            updated_at: "2026-09-12T00:00:00Z".to_string(),
        }
    }

    fn expect_full_action(action: &Value) {
        for key in [
            "op",
            "element",
            "target",
            "risk",
            "before_digest",
            "after_digest",
            "backup_path",
            "cmd",
            "note",
        ] {
            assert!(action.get(key).is_some(), "action 缺字段 {key}");
        }
        assert_eq!(action["op"], "write-file");
        assert_eq!(action["risk"], "write-local");
        assert_eq!(action["backup_path"], Value::Null);
        assert_eq!(action["cmd"], Value::Null);
    }

    fn expect_full_summary(preflight: &Value, write_count: usize) {
        let summary = &preflight["summary"];
        for key in [
            "read",
            "write-local",
            "exec-local",
            "network",
            "install-remove",
        ] {
            assert!(summary.get(key).is_some(), "summary 缺 key {key}");
        }
        assert_eq!(summary["read"], 0);
        assert_eq!(summary["write-local"], write_count as u64);
        assert_eq!(summary["exec-local"], 0);
        assert_eq!(summary["network"], 0);
        assert_eq!(summary["install-remove"], 0);
    }

    #[test]
    fn no_db_no_home_defaults_to_all_write_file_actions_and_creates_nothing() {
        let root = temp_root("empty");
        let ctx = ctx_at(&root);
        let preflight = run_preflight(&ctx, &[]).unwrap();
        assert_eq!(preflight["domain"], "rules");
        let actions = preflight["actions"].as_array().unwrap();
        // 目标全集：codex、claude、opencode
        assert_eq!(actions.len(), 3);
        assert_eq!(actions[0]["element"], "rules:codex_global");
        assert_eq!(actions[1]["element"], "rules:claude_global");
        assert_eq!(actions[2]["element"], "rules:opencode_global");
        for action in actions {
            expect_full_action(action);
            assert_eq!(action["before_digest"], Value::Null);
            assert!(action["after_digest"].as_str().is_some());
        }
        expect_full_summary(&preflight, 3);
        assert!(!ctx.data_dir.exists());
        assert!(!ctx.home_dir.exists());
        drop_root(&root);
    }

    #[test]
    fn only_single_and_reversed_pair_are_stable() {
        let root = temp_root("only");
        let ctx = ctx_at(&root);
        let preflight = run_preflight(&ctx, &["--only", "rules:claude_global"]).unwrap();
        let actions = preflight["actions"].as_array().unwrap();
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0]["element"], "rules:claude_global");
        expect_full_summary(&preflight, 1);

        let preflight = run_preflight(
            &ctx,
            &[
                "--only",
                "rules:claude_global",
                "--only",
                "rules:codex_global",
            ],
        )
        .unwrap();
        let actions = preflight["actions"].as_array().unwrap();
        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0]["element"], "rules:codex_global");
        assert_eq!(actions[1]["element"], "rules:claude_global");
        drop_root(&root);
    }

    #[test]
    fn disabled_and_managed_ok_produce_no_action() {
        let root = temp_root("skip");
        let ctx = ctx_at(&root);
        write_policy_disabled_agent(&ctx, "codex");
        // codex 被停用，其余目标（claude、opencode）都已是受管且一致的状态
        for target in [&targets()[1], &targets()[2]] {
            let path = ctx.home_dir.join(target.target);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            let rendered = current_rendered(&ctx, target);
            std::fs::write(&path, &rendered).unwrap();
            write_binding(&ctx, &binding_for(&ctx, target, &rendered));
        }

        let preflight = run_preflight(&ctx, &[]).unwrap();
        assert_eq!(preflight["actions"].as_array().unwrap().len(), 0);
        expect_full_summary(&preflight, 0);
        drop_root(&root);
    }

    #[test]
    fn drifted_after_source_change_has_distinct_before_after_digests() {
        let root = temp_root("drift");
        let ctx = ctx_at(&root);
        std::fs::create_dir_all(ctx.home_dir.join(".codex")).unwrap();
        let codex = &targets()[0];
        let rendered = current_rendered(&ctx, codex);
        let binding = binding_for(&ctx, codex, &rendered);
        std::fs::write(ctx.home_dir.join(codex.target), &rendered).unwrap();
        write_binding(&ctx, &binding);
        write_source(&ctx, KEY_SHARED, "changed shared v2\n");

        let preflight = run_preflight(&ctx, &["--only", "rules:codex_global"]).unwrap();
        let actions = preflight["actions"].as_array().unwrap();
        assert_eq!(actions.len(), 1);
        let action = &actions[0];
        expect_full_action(action);
        assert_eq!(action["element"], "rules:codex_global");
        assert_eq!(
            action["before_digest"].as_str().unwrap(),
            binding.applied_digest
        );
        let after = action["after_digest"].as_str().unwrap();
        assert_ne!(after, binding.applied_digest);
        assert_eq!(
            action["note"].as_str().unwrap(),
            "同步 rules:codex_global 规则文件"
        );
        drop_root(&root);
    }

    #[test]
    fn drifted_registration_needed_uses_expected_digest_as_before() {
        let root = temp_root("regneed");
        let ctx = ctx_at(&root);
        std::fs::create_dir_all(ctx.home_dir.join(".claude")).unwrap();
        let claude = &targets()[1];
        let rendered = current_rendered(&ctx, claude);
        std::fs::write(ctx.home_dir.join(claude.target), &rendered).unwrap();

        let preflight = run_preflight(&ctx, &["--only", "rules:claude_global"]).unwrap();
        let actions = preflight["actions"].as_array().unwrap();
        assert_eq!(actions.len(), 1);
        let action = &actions[0];
        expect_full_action(action);
        assert_eq!(
            action["before_digest"].as_str().unwrap(),
            sha256_hex(&rendered)
        );
        assert_eq!(action["before_digest"], action["after_digest"]);
        drop_root(&root);
    }

    #[test]
    fn selected_unmanaged_fails_whole_preflight_but_unselected_does_not_block() {
        let root = temp_root("blocked");
        let ctx = ctx_at(&root);
        std::fs::create_dir_all(ctx.home_dir.join(".codex")).unwrap();
        std::fs::write(
            ctx.home_dir.join(".codex/AGENTS.md"),
            "external marker-free\n",
        )
        .unwrap();

        let err = run_preflight(&ctx, &[]).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("preflight 拒绝"));
        assert!(!msg.contains("external marker-free"));

        let preflight = run_preflight(&ctx, &["--only", "rules:claude_global"]).unwrap();
        let actions = preflight["actions"].as_array().unwrap();
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0]["element"], "rules:claude_global");
        drop_root(&root);
    }

    #[test]
    fn preflight_json_never_contains_rule_bodies() {
        let root = temp_root("noleak");
        let ctx = ctx_at(&root);
        let marker = "PREFLIGHT-LEAK-MARKER-7f3d9a";
        write_source(&ctx, KEY_SHARED, &format!("{marker}\n"));
        let preflight = run_preflight(&ctx, &[]).unwrap();
        let serialized = serde_json::to_string(&preflight).unwrap();
        assert!(!serialized.contains(marker), "preflight JSON 泄漏规则正文");
        drop_root(&root);
    }

    #[test]
    fn existing_db_and_target_bytes_unchanged_after_preflight() {
        let root = temp_root("readonly");
        let ctx = ctx_at(&root);
        std::fs::create_dir_all(ctx.home_dir.join(".codex")).unwrap();
        let codex = &targets()[0];
        let rendered = current_rendered(&ctx, codex);
        std::fs::write(ctx.home_dir.join(codex.target), &rendered).unwrap();
        write_binding(&ctx, &binding_for(&ctx, codex, &rendered));

        let db = store::db_path(&ctx);
        let db_before = std::fs::read(&db).unwrap();
        let target_before = std::fs::read(ctx.home_dir.join(codex.target)).unwrap();

        let preflight = run_preflight(&ctx, &[]).unwrap();
        assert_eq!(preflight["actions"].as_array().unwrap().len(), 2);
        assert_eq!(preflight["actions"][0]["element"], "rules:claude_global");
        assert_eq!(preflight["actions"][1]["element"], "rules:opencode_global");

        assert_eq!(std::fs::read(&db).unwrap(), db_before);
        assert_eq!(
            std::fs::read(ctx.home_dir.join(codex.target)).unwrap(),
            target_before
        );
        drop_root(&root);
    }
}
