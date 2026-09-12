//! rules 域纯读 plan：先严格解析参数，再只读调用 status::build_status
//! 取得状态；不创建/修改 data_dir、home_dir、DB、表或目标文件。
//! 选中的 enabled 目标若 unmanaged-detected 或带 blocked_reason，整个
//! plan fail-closed，不输出半份动作；输出与错误均不含规则正文。

use anyhow::{bail, Result};
use serde_json::{json, Value};

use super::{args, model, status};
use crate::native::{Context, Request};

fn action_for(element: &Value) -> Option<Value> {
    let state = element["state"].as_str().unwrap_or("");
    if state != "missing" && state != "drifted" {
        return None;
    }
    let observed = &element["observed"];
    let before_digest = if state == "missing" {
        Value::Null
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

pub(crate) fn build_plan(ctx: &Context, req: &Request) -> Result<Value> {
    let parsed = args::parse_plan_args(req)?;
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
        {
            bail!("plan 拒绝: 选中的规则目标存在阻塞，未生成任何动作");
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
            if let Some(action) = action_for(element) {
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
    use crate::native_mcp::model::sha256_hex;
    use crate::native_rules::model::{render_rules, targets, Binding, TargetSpec};
    use crate::native_rules::persistence::ensure_table;
    use crate::native_sources::{self, KEY_SHARED};
    use crate::store;
    use std::path::Path;

    fn temp_root(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "native-rules-plan-test-{}-{}-{:x}",
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
            command: "rules:plan".to_string(),
            args: args.iter().map(|s| s.to_string()).collect(),
            run_id: "run-plan-test".to_string(),
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

    fn expect_full_summary(plan: &Value, write_count: usize) {
        let summary = &plan["summary"];
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
    fn no_db_no_home_defaults_to_two_write_file_actions_and_creates_nothing() {
        let root = temp_root("empty");
        let ctx = ctx_at(&root);
        let plan = build_plan(&ctx, &req(&[])).unwrap();
        assert_eq!(plan["domain"], "rules");
        let actions = plan["actions"].as_array().unwrap();
        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0]["element"], "rules:codex_global");
        assert_eq!(actions[1]["element"], "rules:claude_global");
        for action in actions {
            expect_full_action(action);
            assert_eq!(action["before_digest"], Value::Null);
            assert!(action["after_digest"].as_str().is_some());
        }
        expect_full_summary(&plan, 2);
        assert!(!ctx.data_dir.exists());
        assert!(!ctx.home_dir.exists());
        drop_root(&root);
    }

    #[test]
    fn only_single_and_reversed_pair_are_stable() {
        let root = temp_root("only");
        let ctx = ctx_at(&root);
        let plan = build_plan(&ctx, &req(&["--only", "rules:claude_global"])).unwrap();
        let actions = plan["actions"].as_array().unwrap();
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0]["element"], "rules:claude_global");
        expect_full_summary(&plan, 1);

        let plan = build_plan(
            &ctx,
            &req(&[
                "--only",
                "rules:claude_global",
                "--only",
                "rules:codex_global",
            ]),
        )
        .unwrap();
        let actions = plan["actions"].as_array().unwrap();
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
        std::fs::create_dir_all(ctx.home_dir.join(".claude")).unwrap();
        let claude = &targets()[1];
        let rendered = current_rendered(&ctx, claude);
        std::fs::write(ctx.home_dir.join(claude.target), &rendered).unwrap();
        write_binding(&ctx, &binding_for(&ctx, claude, &rendered));

        let plan = build_plan(&ctx, &req(&[])).unwrap();
        assert_eq!(plan["actions"].as_array().unwrap().len(), 0);
        expect_full_summary(&plan, 0);
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

        let plan = build_plan(&ctx, &req(&["--only", "rules:codex_global"])).unwrap();
        let actions = plan["actions"].as_array().unwrap();
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

        let plan = build_plan(&ctx, &req(&["--only", "rules:claude_global"])).unwrap();
        let actions = plan["actions"].as_array().unwrap();
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
    fn selected_unmanaged_fails_whole_plan_but_unselected_does_not_block() {
        let root = temp_root("blocked");
        let ctx = ctx_at(&root);
        std::fs::create_dir_all(ctx.home_dir.join(".codex")).unwrap();
        std::fs::write(
            ctx.home_dir.join(".codex/AGENTS.md"),
            "external marker-free\n",
        )
        .unwrap();

        let err = build_plan(&ctx, &req(&[])).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("plan 拒绝"));
        assert!(!msg.contains("external marker-free"));

        let plan = build_plan(&ctx, &req(&["--only", "rules:claude_global"])).unwrap();
        let actions = plan["actions"].as_array().unwrap();
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0]["element"], "rules:claude_global");
        drop_root(&root);
    }

    #[test]
    fn plan_json_never_contains_rule_bodies() {
        let root = temp_root("noleak");
        let ctx = ctx_at(&root);
        let marker = "PLAN-LEAK-MARKER-7f3d9a";
        write_source(&ctx, KEY_SHARED, &format!("{marker}\n"));
        let plan = build_plan(&ctx, &req(&[])).unwrap();
        let serialized = serde_json::to_string(&plan).unwrap();
        assert!(!serialized.contains(marker), "plan JSON 泄漏规则正文");
        drop_root(&root);
    }

    #[test]
    fn existing_db_and_target_bytes_unchanged_after_plan() {
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

        let plan = build_plan(&ctx, &req(&[])).unwrap();
        assert_eq!(plan["actions"].as_array().unwrap().len(), 1);
        assert_eq!(plan["actions"][0]["element"], "rules:claude_global");

        assert_eq!(std::fs::read(&db).unwrap(), db_before);
        assert_eq!(
            std::fs::read(ctx.home_dir.join(codex.target)).unwrap(),
            target_before
        );
        drop_root(&root);
    }
}
