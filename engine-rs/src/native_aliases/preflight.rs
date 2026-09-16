//! aliases 域私有 preflight/action 计算：基于只读 status::build_status 推导
//! apply 将执行的动作；不创建/修改 data_dir、home_dir、DB、表或目标文件。
//! 选中的目标若带 blocked_reason（外部修改/路径不安全/不可读），整个
//! preflight fail-closed。before/after 均为受管 marker 区块内容的 sha256
//! （无区块/空期望为 null）。该计算只在 apply 锁前/写前预检内部使用，
//! 不再对应任何公开命令。

use anyhow::{bail, Result};
use serde_json::{json, Value};

use super::{args, model, status};
use crate::native::Context;

fn action_for(element: &Value) -> Option<Value> {
    let state = element["state"].as_str().unwrap_or("");
    if !matches!(state, "drifted" | "missing" | "not-materialized") {
        return None;
    }
    let observed = &element["observed"];
    let before_digest = if observed["block_present"].as_bool() == Some(true) {
        // 区块存在：before 为当前区块内容 sha256（drifted 才有此组合）。
        observed["actual_block_digest"].clone()
    } else {
        Value::Null
    };
    let id = element["id"].as_str().unwrap_or("");
    let after_digest = if observed["alias_count"].as_u64().unwrap_or(0) == 0 {
        // 期望区块为空：收敛即移除区块，after 为 null。
        Value::Null
    } else {
        observed["expected_digest"].clone()
    };
    Some(json!({
        "op": "write-file",
        "element": id,
        "target": observed["target"],
        "risk": "write-local",
        "before_digest": before_digest,
        "after_digest": after_digest,
        "backup_path": Value::Null,
        "cmd": Value::Null,
        "note": format!("同步 {id} 受管 alias 区块；marker 区块外内容不动"),
    }))
}

/// 预检与动作计算：入参为已通过严格解析的 apply 参数；返回动作与汇总
/// （仅测试断言使用，apply 只依赖其 fail-closed 语义）。
pub(crate) fn build_preflight(ctx: &Context, parsed: &args::AliasArgs) -> Result<Value> {
    let status = status::build_status(ctx)?;

    let empty = Vec::new();
    let elements = status["elements"].as_array().unwrap_or(&empty);

    for id in &parsed.only {
        let Some(element) = elements.iter().find(|e| e["id"].as_str() == Some(id)) else {
            continue;
        };
        if !element["observed"]["blocked_reason"].is_null() {
            bail!("preflight 拒绝: 选中的 aliases 目标存在阻塞，未生成任何动作");
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
        "domain": "aliases",
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
    use crate::native_aliases::persistence::{ensure_table, Binding};
    use crate::native_mcp::model::sha256_hex;
    use crate::store;
    use std::path::Path;

    fn temp_root(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "native-aliases-preflight-test-{}-{}-{:x}",
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
            command: "aliases:apply".to_string(),
            args: args.iter().map(|s| s.to_string()).collect(),
            run_id: "run-preflight-test".to_string(),
        };
        let parsed = args::parse_apply_args(&req)?;
        build_preflight(ctx, &parsed)
    }

    fn seed_source(ctx: &Context, value: &Value) {
        let conn = store::open_write(ctx).unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS alias_sources (
                key TEXT PRIMARY KEY NOT NULL,
                value_json TEXT NOT NULL,
                revision TEXT NOT NULL,
                updated_at TEXT NOT NULL
            )",
        )
        .unwrap();
        let canonical = serde_json::to_string(value).unwrap();
        let revision = sha256_hex(&canonical);
        conn.execute(
            "INSERT INTO alias_sources (key, value_json, revision, updated_at) VALUES ('aliases.shell', ?1, ?2, '2026-09-12T00:00:00Z')
             ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, revision = excluded.revision, updated_at = excluded.updated_at",
            rusqlite::params![canonical, revision],
        )
        .unwrap();
    }

    fn write_binding(ctx: &Context, binding: &Binding) {
        let mut conn = store::open_write(ctx).unwrap();
        ensure_table(&conn).unwrap();
        let tx = conn.transaction().unwrap();
        crate::native_aliases::persistence::write_binding(&tx, binding).unwrap();
        tx.commit().unwrap();
    }

    fn expected_block(lines: &[&str]) -> String {
        format!(
            "{}\n{}\n{}\n",
            model::MARKER_BEGIN,
            lines.join("\n"),
            model::MARKER_END
        )
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

    #[test]
    fn fresh_context_two_actions_summary_keys_deterministic_and_creates_nothing() {
        let root = temp_root("fresh");
        let ctx = ctx_at(&root);
        let first = run_preflight(&ctx, &[]).unwrap();
        let second = run_preflight(&ctx, &[]).unwrap();
        assert_eq!(
            serde_json::to_string(&first).unwrap(),
            serde_json::to_string(&second).unwrap(),
            "preflight 动作计算必须确定性一致"
        );
        assert_eq!(first["domain"], "aliases");
        let actions = first["actions"].as_array().unwrap();
        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0]["element"], "aliases:zshrc");
        assert_eq!(actions[1]["element"], "aliases:zprofile");
        for action in actions {
            expect_full_action(action);
            assert_eq!(action["before_digest"], Value::Null);
            assert!(action["after_digest"].as_str().is_some());
        }
        assert_eq!(first["summary"]["write-local"], 2);
        for key in [
            "read",
            "write-local",
            "exec-local",
            "network",
            "install-remove",
        ] {
            assert!(first["summary"].get(key).is_some(), "summary 缺 {key}");
        }
        assert!(!ctx.data_dir.exists());
        assert!(!ctx.home_dir.exists());
        drop_root(&root);
    }

    #[test]
    fn source_change_action_digests_are_block_sha256() {
        let root = temp_root("source-drift");
        let ctx = ctx_at(&root);
        seed_source(&ctx, &json!({"aliases": {"ll": "ls -l"}}));
        std::fs::create_dir_all(&ctx.home_dir).unwrap();
        let zshrc = ctx.home_dir.join(".zshrc");
        let content_v1 = expected_block(&["alias ll='ls -l'"]);
        std::fs::write(&zshrc, &content_v1).unwrap();
        let revision =
            sha256_hex(&serde_json::to_string(&json!({"aliases": {"ll": "ls -l"}})).unwrap());
        write_binding(
            &ctx,
            &Binding {
                target_id: "aliases:zshrc".to_string(),
                applied_digest: crate::native_mcp::model::sha256_hex_bytes(content_v1.as_bytes()),
                source_revision: revision,
                updated_at: "2026-09-12T00:00:00Z".to_string(),
            },
        );
        seed_source(&ctx, &json!({"aliases": {"ll": "ls -alh"}}));

        let preflight = run_preflight(&ctx, &["--only", "aliases:zshrc"]).unwrap();
        let actions = preflight["actions"].as_array().unwrap();
        assert_eq!(actions.len(), 1);
        let action = &actions[0];
        expect_full_action(action);
        let before = action["before_digest"].as_str().unwrap();
        let after = action["after_digest"].as_str().unwrap();
        assert_ne!(before, after);
        assert_eq!(before, sha256_hex("alias ll='ls -l'"));
        assert_eq!(after, sha256_hex("alias ll='ls -alh'"));
        drop_root(&root);
    }

    #[test]
    fn empty_source_action_removes_block_with_null_after_digest() {
        let root = temp_root("empty-source");
        let ctx = ctx_at(&root);
        seed_source(&ctx, &json!({"aliases": {}}));
        std::fs::create_dir_all(&ctx.home_dir).unwrap();
        let zshrc = ctx.home_dir.join(".zshrc");
        std::fs::write(
            &zshrc,
            "pre\n# >>> agent-manager shell aliases >>>\nalias old='1'\n# <<< agent-manager shell aliases <<<\npost\n",
        )
        .unwrap();

        let preflight = run_preflight(&ctx, &["--only", "aliases:zshrc"]).unwrap();
        let actions = preflight["actions"].as_array().unwrap();
        assert_eq!(actions.len(), 1);
        let action = &actions[0];
        expect_full_action(action);
        assert_eq!(
            action["before_digest"].as_str().unwrap(),
            sha256_hex("alias old='1'")
        );
        assert_eq!(action["after_digest"], Value::Null);
        drop_root(&root);
    }

    #[test]
    fn selected_blocked_fails_whole_preflight_but_unselected_does_not_block() {
        let root = temp_root("blocked");
        let ctx = ctx_at(&root);
        seed_source(&ctx, &json!({"aliases": {"ll": "ls -l"}}));
        std::fs::create_dir_all(&ctx.home_dir).unwrap();
        let zshrc = ctx.home_dir.join(".zshrc");
        let content = expected_block(&["alias ll='ls -l'"]);
        std::fs::write(&zshrc, &content).unwrap();
        write_binding(
            &ctx,
            &Binding {
                target_id: "aliases:zshrc".to_string(),
                applied_digest: crate::native_mcp::model::sha256_hex_bytes(content.as_bytes()),
                source_revision: sha256_hex(
                    &serde_json::to_string(&json!({"aliases": {"ll": "ls -l"}})).unwrap(),
                ),
                updated_at: "2026-09-12T00:00:00Z".to_string(),
            },
        );
        // 绑定后外部追加：blocked_reason 出现。
        let mut bytes = std::fs::read(&zshrc).unwrap();
        bytes.extend_from_slice(b"# user external edit\n");
        std::fs::write(&zshrc, &bytes).unwrap();

        let db = store::db_path(&ctx);
        let db_before = std::fs::read(&db).unwrap();
        let target_before = std::fs::read(&zshrc).unwrap();

        let err = run_preflight(&ctx, &["--only", "aliases:zshrc"]).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("preflight 拒绝"));
        assert!(!msg.contains("user external edit"));

        // 未选中的 zprofile 不阻塞：仍给出单个 not-materialized 动作。
        let preflight = run_preflight(&ctx, &["--only", "aliases:zprofile"]).unwrap();
        let actions = preflight["actions"].as_array().unwrap();
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0]["element"], "aliases:zprofile");

        // 失败与成功的 preflight 均零写：DB 与目标字节不变。
        assert_eq!(std::fs::read(&db).unwrap(), db_before);
        assert_eq!(std::fs::read(&zshrc).unwrap(), target_before);
        drop_root(&root);
    }
}
