//! hooks 域私有 preflight/action 计算：基于只读 status::build_status 推导
//! apply 将执行的动作；不创建/修改 data_dir、home_dir、DB、表或目标文件。
//! 选中的 enabled 目标若 unmanaged-detected 或带 blocked_reason，整个
//! preflight fail-closed；唯一的例外是 --allow-takeover（仅 --only 单个
//! 目标）且目标文件本身可读、结构合法（actual_digest 存在）：claude 走
//! 既有 signature-set 合并，codex whole-file 允许替换并标注 takeover。
//! 该计算只在 apply 锁前/写前预检内部使用，不再对应任何公开命令。

use anyhow::{bail, Result};
use serde_json::{json, Value};

use super::{args, model, status};
use crate::native::Context;

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
    } else if let Some(actual) = observed["actual_digest"].as_str() {
        // 无 binding 的 drifted（含安全合并）：before 为当前实际字节摘要。
        json!(actual)
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
        "note": format!("同步 {id} 钩子文件"),
    }))
}

/// 目标是否处于阻塞态（unmanaged-detected 或带 blocked_reason）。
fn element_blocked(element: &Value) -> bool {
    element["state"].as_str() == Some("unmanaged-detected")
        || !element["observed"]["blocked_reason"].is_null()
}

/// 阻塞目标是否可显式接管：--allow-takeover 已携带（调用方保证单目标），
/// 且 status 探测证明目标文件可读、JSON 与 hooks 子树结构合法
/// （actual_digest 存在）。结构损坏/路径不安全的目标依然 fail-closed。
fn takeover_eligible(element: &Value, allow_takeover: bool) -> bool {
    allow_takeover && element["observed"]["actual_digest"].is_string()
}

/// 显式接管动作：codex whole-file 替换标注 takeover 并在 note 明示整体替换
/// 风险；claude 走既有 signature-set 合并（保留用户字段与非本工具 hook），
/// 动作形态与常规同步一致。
fn takeover_action_for(element: &Value) -> Value {
    let observed = &element["observed"];
    let id = element["id"].as_str().unwrap_or("");
    let whole_file = element["spec"]["merge"].as_str() == Some("whole-file");
    let note = if whole_file {
        format!(
            "同步 {id} 钩子文件（whole-file 替换：目标文件现有内容将被整体替换，请先把需要保留的 hook 并入 hooks.codex 来源）"
        )
    } else {
        format!("同步 {id} 钩子文件")
    };
    let mut action = json!({
        "op": "write-file",
        "element": id,
        "target": observed["target"],
        "risk": "write-local",
        "before_digest": observed["actual_digest"],
        "after_digest": observed["expected_digest"],
        "backup_path": Value::Null,
        "cmd": Value::Null,
        "note": note,
    });
    if whole_file {
        action
            .as_object_mut()
            .expect("object")
            .insert("takeover".into(), Value::Bool(true));
    }
    action
}

/// 预检与动作计算：入参为已通过严格解析的 apply 参数；返回动作与汇总
/// （仅测试断言使用，apply 只依赖其 fail-closed 语义）。
pub(crate) fn build_preflight(ctx: &Context, parsed: &args::HookArgs) -> Result<Value> {
    let status = status::build_status(ctx)?;

    let empty = Vec::new();
    let elements = status["elements"].as_array().unwrap_or(&empty);

    for id in &parsed.only {
        let Some(element) = elements.iter().find(|e| e["id"].as_str() == Some(id)) else {
            continue;
        };
        let enabled = element["policy"]["enabled"].as_bool().unwrap_or(false);
        if enabled && element_blocked(element) && !takeover_eligible(element, parsed.allow_takeover)
        {
            bail!("preflight 拒绝: 选中的 hooks 目标存在阻塞，未生成任何动作");
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
            } else {
                let enabled = element["policy"]["enabled"].as_bool().unwrap_or(false);
                if enabled
                    && element_blocked(element)
                    && takeover_eligible(element, parsed.allow_takeover)
                {
                    actions.push(takeover_action_for(element));
                }
            }
        }
    }

    let write_count = actions.len();
    Ok(json!({
        "domain": "hooks",
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
    use crate::native_hooks::persistence::{ensure_table, Binding};
    use crate::native_mcp::model::sha256_hex;
    use crate::store;
    use std::path::Path;

    fn temp_root(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "native-hooks-preflight-test-{}-{}-{:x}",
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
            command: "hooks:apply".to_string(),
            args: args.iter().map(|s| s.to_string()).collect(),
            run_id: "run-preflight-test".to_string(),
        };
        let parsed = args::parse_apply_args(&req)?;
        build_preflight(ctx, &parsed)
    }

    fn canonicalize_value(value: &Value) -> Value {
        match value {
            Value::Object(map) => {
                let mut entries: Vec<(&String, &Value)> = map.iter().collect();
                entries.sort_by(|a, b| a.0.cmp(b.0));
                let mut out = serde_json::Map::new();
                for (key, item) in entries {
                    out.insert(key.clone(), canonicalize_value(item));
                }
                Value::Object(out)
            }
            Value::Array(items) => Value::Array(items.iter().map(canonicalize_value).collect()),
            other => other.clone(),
        }
    }

    fn managed_claude_source(command: &str) -> Value {
        json!({
            "hooks": {"SessionStart": [{"hooks": [{"type": "command", "command": command}]}]},
            "removed_signatures": [{
                "event": "PreToolUse",
                "matcher": "Write",
                "command_contains": "legacy-hook.sh"
            }]
        })
    }

    fn managed_codex_source(command: &str) -> Value {
        json!({
            "hooks": {"SessionStart": [{"hooks": [{"type": "command", "command": command}]}]}
        })
    }

    fn write_hook_source(ctx: &Context, key: &str, value: &Value) {
        let conn = store::open_write(ctx).unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS hook_sources (
                key TEXT PRIMARY KEY NOT NULL,
                value_json TEXT NOT NULL,
                revision TEXT NOT NULL,
                updated_at TEXT NOT NULL
            )",
        )
        .unwrap();
        let canonical = serde_json::to_string(&canonicalize_value(value)).unwrap();
        let revision = sha256_hex(&canonical);
        conn.execute(
            "INSERT INTO hook_sources (key, value_json, revision, updated_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, revision = excluded.revision, updated_at = excluded.updated_at",
            rusqlite::params![key, canonical, revision, "2026-09-12T00:00:00Z"],
        )
        .unwrap();
    }

    fn write_binding(ctx: &Context, binding: &Binding) {
        let mut conn = store::open_write(ctx).unwrap();
        ensure_table(&conn).unwrap();
        let tx = conn.transaction().unwrap();
        crate::native_hooks::persistence::write_binding(&tx, binding).unwrap();
        tx.commit().unwrap();
    }

    /// 用当前来源渲染目标期望字节与摘要（缺失现状按空 settings）。
    fn expected_bytes_and_revision(
        spec: &model::TargetSpec,
        source: &Value,
    ) -> (Vec<u8>, String, String) {
        let canonical = serde_json::to_string(&canonicalize_value(source)).unwrap();
        let (managed, removed) = status::parse_source(spec, &canonical).unwrap();
        let expected_value = status::expected_render(spec, &managed, &removed, None);
        let expected = model::render_bytes(&expected_value);
        let digest = model::digest_bytes(&expected);
        (expected, digest, sha256_hex(&canonical))
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
    fn fresh_context_zero_actions_summary_keys_deterministic_and_creates_nothing() {
        let root = temp_root("fresh");
        let ctx = ctx_at(&root);
        // 默认来源：claude 只有 removed_signatures、codex hooks 为空 → 无动作。
        let first = run_preflight(&ctx, &[]).unwrap();
        let second = run_preflight(&ctx, &[]).unwrap();
        assert_eq!(
            serde_json::to_string(&first).unwrap(),
            serde_json::to_string(&second).unwrap(),
            "preflight 动作计算必须确定性一致"
        );
        assert_eq!(first["domain"], "hooks");
        assert_eq!(first["actions"].as_array().unwrap().len(), 0);
        assert_eq!(first["summary"]["write-local"], 0);
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
    fn drifted_after_source_change_action_shape_and_distinct_digests() {
        let root = temp_root("drift");
        let ctx = ctx_at(&root);
        let spec = &model::TARGET_CODEX;
        let source_v1 = managed_codex_source("echo v1");
        write_hook_source(&ctx, spec.source_key, &source_v1);
        let (expected_v1, digest_v1, revision_v1) = expected_bytes_and_revision(spec, &source_v1);
        let abs = ctx.home_dir.join(spec.target);
        std::fs::create_dir_all(abs.parent().unwrap()).unwrap();
        std::fs::write(&abs, &expected_v1).unwrap();
        write_binding(
            &ctx,
            &Binding {
                target_id: spec.target_id.to_string(),
                applied_digest: digest_v1.clone(),
                source_revision: revision_v1,
                updated_at: "2026-09-12T00:00:00Z".to_string(),
            },
        );
        write_hook_source(&ctx, spec.source_key, &managed_codex_source("echo v2"));

        let preflight = run_preflight(&ctx, &["--only", "hooks:codex"]).unwrap();
        let actions = preflight["actions"].as_array().unwrap();
        assert_eq!(actions.len(), 1);
        let action = &actions[0];
        expect_full_action(action);
        assert_eq!(action["element"], "hooks:codex");
        assert_eq!(action["before_digest"].as_str().unwrap(), digest_v1);
        assert_ne!(action["after_digest"].as_str().unwrap(), digest_v1);
        drop_root(&root);
    }

    #[test]
    fn unbound_merge_before_digest_uses_actual_bytes() {
        let root = temp_root("before-digest");
        let ctx = ctx_at(&root);
        let spec = &model::TARGET_CLAUDE;
        write_hook_source(
            &ctx,
            spec.source_key,
            &managed_claude_source("echo new-managed"),
        );
        let abs = ctx.home_dir.join(spec.target);
        std::fs::create_dir_all(abs.parent().unwrap()).unwrap();
        let existing = json!({"model": "m", "hooks": {"SessionStart": [{"hooks": [{"type": "command", "command": "echo user"}]}]}});
        std::fs::write(
            &abs,
            format!("{}\n", serde_json::to_string_pretty(&existing).unwrap()),
        )
        .unwrap();
        let old_sha = crate::native_mcp::model::sha256_hex_bytes(&std::fs::read(&abs).unwrap());

        let preflight = run_preflight(&ctx, &["--only", "hooks:claude"]).unwrap();
        let actions = preflight["actions"].as_array().unwrap();
        assert_eq!(actions.len(), 1);
        let action = &actions[0];
        expect_full_action(action);
        assert_eq!(
            action["before_digest"].as_str().unwrap(),
            old_sha,
            "before=旧字节 sha"
        );
        assert_ne!(
            action["after_digest"].as_str().unwrap(),
            old_sha,
            "after=渲染 sha 且与 before 不同"
        );
        drop_root(&root);
    }

    #[test]
    fn selected_unmanaged_fails_whole_preflight_but_unselected_does_not_block() {
        let root = temp_root("blocked");
        let ctx = ctx_at(&root);
        write_hook_source(&ctx, "hooks.codex", &managed_codex_source("echo managed"));
        let abs = ctx.home_dir.join(".codex/hooks.json");
        std::fs::create_dir_all(abs.parent().unwrap()).unwrap();
        std::fs::write(
            &abs,
            json!({"hooks": {"Other": []}, "foreign": true}).to_string(),
        )
        .unwrap();

        let err = run_preflight(&ctx, &[]).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("preflight 拒绝"));
        assert!(!msg.contains("foreign"));

        let preflight = run_preflight(&ctx, &["--only", "hooks:claude"]).unwrap();
        assert_eq!(preflight["actions"].as_array().unwrap().len(), 0);
        drop_root(&root);
    }

    #[test]
    fn takeover_codex_whole_file_action_is_marked_with_risk_note() {
        let root = temp_root("takeover-codex");
        let ctx = ctx_at(&root);
        write_hook_source(&ctx, "hooks.codex", &managed_codex_source("echo managed"));
        let abs = ctx.home_dir.join(".codex/hooks.json");
        std::fs::create_dir_all(abs.parent().unwrap()).unwrap();
        let foreign = json!({
            "hooks": {"SessionStart": [{"hooks": [{"type": "command", "command": "sh user-own-session.sh"}]}]},
            "foreign": true
        });
        std::fs::write(
            &abs,
            format!("{}\n", serde_json::to_string_pretty(&foreign).unwrap()),
        )
        .unwrap();
        let foreign_sha = crate::native_mcp::model::sha256_hex_bytes(&std::fs::read(&abs).unwrap());

        let preflight =
            run_preflight(&ctx, &["--only", "hooks:codex", "--allow-takeover"]).unwrap();
        let actions = preflight["actions"].as_array().unwrap();
        assert_eq!(actions.len(), 1);
        let action = &actions[0];
        expect_full_action(action);
        assert_eq!(
            action["takeover"], true,
            "codex whole-file 替换必须标注 takeover"
        );
        let note = action["note"].as_str().unwrap();
        assert!(
            note.contains("whole-file 替换") && note.contains("并入 hooks.codex 来源"),
            "note 必须明示整体替换与先并来源建议: {note}"
        );
        assert_eq!(action["before_digest"].as_str().unwrap(), foreign_sha);
        assert_ne!(action["after_digest"].as_str().unwrap(), foreign_sha);
        drop_root(&root);
    }

    #[test]
    fn takeover_claude_merge_action_is_not_marked() {
        let root = temp_root("takeover-claude");
        let ctx = ctx_at(&root);
        let spec = &model::TARGET_CLAUDE;
        let source = managed_claude_source("echo managed-takeover");
        write_hook_source(&ctx, spec.source_key, &source);
        let (expected, digest, revision) = expected_bytes_and_revision(spec, &source);
        let abs = ctx.home_dir.join(spec.target);
        std::fs::create_dir_all(abs.parent().unwrap()).unwrap();
        std::fs::write(&abs, &expected).unwrap();
        write_binding(
            &ctx,
            &Binding {
                target_id: spec.target_id.to_string(),
                applied_digest: digest,
                source_revision: revision,
                updated_at: "2026-09-12T00:00:00Z".to_string(),
            },
        );

        // 绑定后外部修改：加用户根字段与用户自有 hook。
        let mut current: Value =
            serde_json::from_str(&std::fs::read_to_string(&abs).unwrap()).unwrap();
        current["model"] = json!("user-model");
        current["hooks"]["SessionStart"]
            .as_array_mut()
            .unwrap()
            .push(json!({"hooks": [{"type": "command", "command": "echo user-own"}]}));
        std::fs::write(&abs, serde_json::to_string_pretty(&current).unwrap()).unwrap();
        let actual_sha = crate::native_mcp::model::sha256_hex_bytes(&std::fs::read(&abs).unwrap());

        // 无 flag：preflight 拒绝外部修改目标。
        let err = run_preflight(&ctx, &["--only", "hooks:claude"]).unwrap_err();
        assert!(format!("{err:#}").contains("preflight 拒绝"));

        // 有 flag：signature-set 合并，动作不标 takeover（合并天然保留用户内容）。
        let preflight =
            run_preflight(&ctx, &["--only", "hooks:claude", "--allow-takeover"]).unwrap();
        let actions = preflight["actions"].as_array().unwrap();
        assert_eq!(actions.len(), 1);
        let action = &actions[0];
        expect_full_action(action);
        assert!(action.get("takeover").is_none(), "claude 合并不标 takeover");
        assert_eq!(action["before_digest"].as_str().unwrap(), actual_sha);
        drop_root(&root);
    }
}
