//! rules 域只读状态闭环：目标固定为 model::targets()，来源只从
//! native_sources 当前有效文本取得，运行时不读 checkout。
//! DB 缺失走 builtin policy + 默认来源 + 空 bindings；DB 存在时只打开一次
//! readonly 连接并在同一 snapshot 读 policy/sources/bindings。status 不创建
//! data/home 目录、DB、表或文件；来源/数据库损坏 fail-closed，目标路径安全
//! 问题降级为 unmanaged-detected + blocked_reason。

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Result;
use serde_json::{json, Value};

use super::model::{render_rules, Binding, TargetSpec};
use super::persistence::load_bindings_from;
use crate::native::Context;
use crate::native_mcp::model::sha256_hex;
use crate::native_mcp::targets::{check_symlink_metadata, validate_target_path};
use crate::native_sources::{self, KEY_SHARED};
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
                sources: native_sources::current_sources_from(&tx)?,
                bindings: load_bindings_from(&tx)?,
            };
            tx.commit()?;
            Ok(snap)
        }
        None => {
            let mut sources = BTreeMap::new();
            for key in native_sources::SOURCE_KEYS {
                sources.insert(
                    key.to_string(),
                    native_sources::default_value(key)
                        .expect("已知来源 key")
                        .to_string(),
                );
            }
            Ok(Snapshot {
                policy: store::builtin_policy(),
                sources,
                bindings: BTreeMap::new(),
            })
        }
    }
}

enum TargetProbe {
    Missing,
    Digest(String),
    Blocked {
        exists: Option<bool>,
        reason: String,
    },
}

/// 只读探测目标：先做路径安全校验，符号链接/祖先链接/目录/非普通文件/
/// 元数据/读取失败一律降级为 Blocked，绝不写文件。
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
            reason: format!("目标路径不是普通文件: {}", abs_path.display()),
        };
    }
    match std::fs::read(abs_path) {
        Ok(bytes) => TargetProbe::Digest(crate::native_mcp::model::sha256_hex_bytes(&bytes)),
        Err(err) => TargetProbe::Blocked {
            exists: Some(true),
            reason: format!("无法读取目标文件 {}: {err}", abs_path.display()),
        },
    }
}

fn build_element(ctx: &Context, snap: &Snapshot, spec: &TargetSpec) -> Value {
    let enabled = target_enabled(&snap.policy, spec.agent_id);
    let shared = &snap.sources[KEY_SHARED];
    let adapter = &snap.sources[spec.adapter_key];
    let shared_revision = sha256_hex(shared);
    let adapter_revision = sha256_hex(adapter);
    let expected = render_rules(shared, adapter);
    let expected_digest = sha256_hex(&expected);
    let binding = snap.bindings.get(spec.target_id);
    let applied_digest = binding.map(|b| b.applied_digest.clone());

    let abs_target = ctx.home_dir.join(spec.target);
    let abs_display = abs_target.display().to_string();

    if !enabled {
        return json!({
            "id": spec.target_id,
            "domain": "rules",
            "spec": spec_json(spec),
            "policy": { "enabled": false, "agent_id": spec.agent_id },
            "observed": {
                "target": abs_display,
                "target_exists": Value::Null,
                "expected_digest": expected_digest,
                "applied_digest": applied_digest,
                "shared_revision": shared_revision,
                "adapter_revision": adapter_revision,
                "blocked_reason": Value::Null,
                "registration_needed": false,
            },
            "state": "disabled",
            "freshness": freshness_json(),
            "actions": [],
        });
    }

    // actual_digest 只在「能安全读到当前内容」时给出；路径不安全/读不了的目标它为 null，
    // 接管入口据此判定——读都读不了的目标不提供接管。
    let mut actual_digest = Value::Null;
    let (state, target_exists, blocked_reason, registration_needed) =
        match probe_target(&abs_target) {
            TargetProbe::Missing => ("missing", Value::Bool(false), Value::Null, false),
            TargetProbe::Blocked { exists, reason } => (
                "unmanaged-detected",
                exists.map_or(Value::Null, Value::Bool),
                Value::String(reason),
                false,
            ),
            TargetProbe::Digest(actual) => {
                actual_digest = Value::String(actual.clone());
                match binding {
                    Some(b) => {
                        if actual == b.applied_digest && actual == expected_digest {
                            ("managed-ok", Value::Bool(true), Value::Null, false)
                        } else if actual == b.applied_digest {
                            ("drifted", Value::Bool(true), Value::Null, false)
                        } else {
                            (
                                "unmanaged-detected",
                                Value::Bool(true),
                                Value::String("目标文件存在外部修改，拒绝覆盖".to_string()),
                                false,
                            )
                        }
                    }
                    None => {
                        if actual == expected_digest {
                            ("drifted", Value::Bool(true), Value::Null, true)
                        } else {
                            (
                                "unmanaged-detected",
                                Value::Bool(true),
                                Value::String(
                                    "目标文件已存在外部内容且无受管绑定，拒绝覆盖".to_string(),
                                ),
                                false,
                            )
                        }
                    }
                }
            }
        };

    let actions =
        if enabled && blocked_reason.is_null() && (state == "missing" || state == "drifted") {
            json!([{ "id": "sync", "risk": "write-local", "needs_lock": true, "offline_ok": true }])
        } else {
            json!([])
        };

    json!({
        "id": spec.target_id,
        "domain": "rules",
        "spec": spec_json(spec),
        "policy": { "enabled": true, "agent_id": spec.agent_id },
        "observed": {
            "target": abs_display,
            "target_exists": target_exists,
            "expected_digest": expected_digest,
            "applied_digest": applied_digest,
            "actual_digest": actual_digest,
            "shared_revision": shared_revision,
            "adapter_revision": adapter_revision,
            "blocked_reason": blocked_reason,
            "registration_needed": registration_needed,
        },
        "state": state,
        "freshness": freshness_json(),
        "actions": actions,
    })
}

fn spec_json(spec: &TargetSpec) -> Value {
    json!({
        "name": match spec.target_id {
            "rules:codex_global" => "Codex 全局规则",
            "rules:opencode_global" => "OpenCode 全局规则",
            _ => "Claude Code 全局规则",
        },
        "group": "global-rules",
        "source_keys": [KEY_SHARED, spec.adapter_key],
        "target": format!("~/{}", spec.target),
    })
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

pub(crate) fn build_status(ctx: &Context) -> Result<Value> {
    let snap = load_snapshot(ctx)?;
    let elements: Vec<Value> = super::model::targets()
        .iter()
        .map(|spec| build_element(ctx, &snap, spec))
        .collect();
    Ok(json!({
        "domain": "rules",
        "elements": elements,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_mcp::model::sha256_hex;
    use crate::native_rules::model::targets;
    use crate::native_rules::persistence::ensure_table;
    use crate::native_sources::{KEY_CLAUDE, KEY_CODEX};

    fn temp_root(tag: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "native-rules-status-test-{}-{}-{:x}",
            std::process::id(),
            tag,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        root
    }

    fn ctx_at(root: &Path) -> Context {
        Context {
            data_dir: root.join("data"),
            home_dir: root.join("home"),
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

    fn binding_for(ctx: &Context, spec: &TargetSpec) -> Binding {
        let snap = load_snapshot(ctx).unwrap();
        let shared = &snap.sources[KEY_SHARED];
        let adapter = &snap.sources[spec.adapter_key];
        let rendered = render_rules(shared, adapter);
        Binding {
            target_id: spec.target_id.to_string(),
            applied_digest: sha256_hex(&rendered),
            shared_revision: sha256_hex(shared),
            adapter_revision: sha256_hex(adapter),
            updated_at: "2026-09-12T00:00:00Z".to_string(),
        }
    }

    fn element(status: &Value, idx: usize) -> &Value {
        &status["elements"][idx]
    }

    fn drop_root(root: &Path) {
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn no_db_no_dirs_returns_all_targets_without_creating_anything() {
        let root = temp_root("empty");
        let ctx = ctx_at(&root);
        let status = build_status(&ctx).unwrap();
        assert_eq!(status["domain"], "rules");
        let els = status["elements"].as_array().unwrap();
        assert_eq!(els.len(), 3);
        assert_eq!(els[0]["id"], "rules:codex_global");
        assert_eq!(els[1]["id"], "rules:claude_global");
        assert_eq!(els[2]["id"], "rules:opencode_global");
        assert!(!ctx.data_dir.exists());
        assert!(!ctx.home_dir.exists());
        assert_eq!(els[0]["state"], "missing");
        assert_eq!(els[0]["observed"]["target_exists"], Value::Bool(false));
        assert_eq!(els[0]["freshness"]["probe"], "native");
        drop_root(&root);
    }

    #[test]
    fn existing_db_bytes_unchanged_and_no_new_tables() {
        let root = temp_root("db");
        let ctx = ctx_at(&root);
        write_source(&ctx, KEY_CODEX, "custom codex adapter\n");
        let db = store::db_path(&ctx);
        let before = std::fs::read(&db).unwrap();
        let tables_before: Vec<String> = {
            let conn = rusqlite::Connection::open(&db).unwrap();
            let mut stmt = conn
                .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
                .unwrap();
            stmt.query_map([], |r| r.get(0))
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap()
        };
        let status = build_status(&ctx).unwrap();
        let after = std::fs::read(&db).unwrap();
        assert_eq!(before, after);
        let tables_after: Vec<String> = {
            let conn = rusqlite::Connection::open(&db).unwrap();
            let mut stmt = conn
                .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
                .unwrap();
            stmt.query_map([], |r| r.get(0))
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap()
        };
        assert_eq!(tables_before, tables_after);
        assert!(!tables_after.contains(&"rule_target_bindings".to_string()));
        assert!(status["elements"].is_array());
        drop_root(&root);
    }

    #[test]
    fn disabled_target_is_not_probed() {
        let root = temp_root("disabled");
        let ctx = ctx_at(&root);
        write_policy_disabled_agent(&ctx, "codex");
        // 目标位置已存在外部内容；若被探测会 unmanaged-detected。
        std::fs::create_dir_all(ctx.home_dir.join(".codex")).unwrap();
        std::fs::write(ctx.home_dir.join(".codex/AGENTS.md"), "anything\n").unwrap();
        let status = build_status(&ctx).unwrap();
        let codex = element(&status, 0);
        assert_eq!(codex["state"], "disabled");
        assert_eq!(codex["observed"]["target_exists"], Value::Null);
        assert_eq!(codex["observed"]["blocked_reason"], Value::Null);
        assert_eq!(codex["actions"], json!([]));
        // 对照：enabled 的 claude 目标确实被探测（路径缺失）。
        let claude = element(&status, 1);
        assert_eq!(claude["state"], "missing");
        drop_root(&root);
    }

    #[cfg(unix)]
    #[test]
    fn disabled_target_is_not_probed_under_symlink_ancestor() {
        let root = temp_root("disabled-link");
        let ctx = ctx_at(&root);
        write_policy_disabled_agent(&ctx, "codex");
        // codex 目标放在一个符号链接祖先之下；若被探测会 unmanaged-detected。
        let real = root.join("real");
        std::fs::create_dir_all(real.join(".codex")).unwrap();
        std::fs::write(real.join(".codex/AGENTS.md"), "anything\n").unwrap();
        let home = ctx.home_dir.clone();
        std::os::unix::fs::symlink(&real, &home).unwrap();
        let status = build_status(&ctx).unwrap();
        let codex = element(&status, 0);
        assert_eq!(codex["state"], "disabled");
        assert_eq!(codex["observed"]["target_exists"], Value::Null);
        assert_eq!(codex["observed"]["blocked_reason"], Value::Null);
        assert_eq!(codex["actions"], json!([]));
        let claude = element(&status, 1);
        // 对照：enabled 的 claude 目标确实被探测，并因祖先符号链接被阻塞。
        assert_eq!(claude["state"], "unmanaged-detected");
        assert!(claude["observed"]["blocked_reason"].as_str().is_some());
        drop_root(&root);
    }

    #[test]
    fn missing_when_enabled_and_absent() {
        let root = temp_root("missing");
        let ctx = ctx_at(&root);
        std::fs::create_dir_all(ctx.home_dir.join(".codex")).unwrap();
        let status = build_status(&ctx).unwrap();
        let codex = element(&status, 0);
        assert_eq!(codex["state"], "missing");
        assert_eq!(
            codex["actions"],
            json!([{ "id": "sync", "risk": "write-local", "needs_lock": true, "offline_ok": true }])
        );
        drop_root(&root);
    }

    #[test]
    fn managed_ok_when_binding_and_bytes_match() {
        let root = temp_root("managed");
        let ctx = ctx_at(&root);
        std::fs::create_dir_all(ctx.home_dir.join(".codex")).unwrap();
        let spec = &targets()[0];
        let binding = binding_for(&ctx, spec);
        let snap = load_snapshot(&ctx).unwrap();
        let rendered = render_rules(&snap.sources[KEY_SHARED], &snap.sources[KEY_CODEX]);
        std::fs::write(ctx.home_dir.join(spec.target), &rendered).unwrap();
        write_binding(&ctx, &binding);
        let status = build_status(&ctx).unwrap();
        let codex = element(&status, 0);
        assert_eq!(codex["state"], "managed-ok");
        assert_eq!(codex["actions"], json!([]));
        assert_eq!(codex["observed"]["applied_digest"], binding.applied_digest);
        drop_root(&root);
    }

    #[test]
    fn drifted_when_sources_change_after_binding() {
        let root = temp_root("drift");
        let ctx = ctx_at(&root);
        std::fs::create_dir_all(ctx.home_dir.join(".codex")).unwrap();
        let spec = &targets()[0];
        let binding = binding_for(&ctx, spec);
        let snap = load_snapshot(&ctx).unwrap();
        let rendered = render_rules(&snap.sources[KEY_SHARED], &snap.sources[KEY_CODEX]);
        std::fs::write(ctx.home_dir.join(spec.target), &rendered).unwrap();
        write_binding(&ctx, &binding);
        write_source(&ctx, KEY_SHARED, "new shared content v2\n");
        let status = build_status(&ctx).unwrap();
        let codex = element(&status, 0);
        assert_eq!(codex["state"], "drifted");
        assert!(codex["observed"]["registration_needed"] == Value::Bool(false));
        assert_eq!(
            codex["actions"],
            json!([{ "id": "sync", "risk": "write-local", "needs_lock": true, "offline_ok": true }])
        );
        drop_root(&root);
    }

    #[test]
    fn drifted_registration_needed_when_bytes_match_without_binding() {
        let root = temp_root("regneed");
        let ctx = ctx_at(&root);
        std::fs::create_dir_all(ctx.home_dir.join(".claude")).unwrap();
        let spec = &targets()[1];
        let snap = load_snapshot(&ctx).unwrap();
        let rendered = render_rules(&snap.sources[KEY_SHARED], &snap.sources[KEY_CLAUDE]);
        std::fs::write(ctx.home_dir.join(spec.target), &rendered).unwrap();
        let status = build_status(&ctx).unwrap();
        let claude = element(&status, 1);
        assert_eq!(claude["state"], "drifted");
        assert_eq!(claude["observed"]["registration_needed"], Value::Bool(true));
        assert_eq!(
            claude["actions"],
            json!([{ "id": "sync", "risk": "write-local", "needs_lock": true, "offline_ok": true }])
        );
        drop_root(&root);
    }

    #[test]
    fn unmanaged_detected_without_binding_or_external_edit() {
        // 无绑定且内容不同
        let root = temp_root("ext");
        let ctx = ctx_at(&root);
        std::fs::create_dir_all(ctx.home_dir.join(".codex")).unwrap();
        std::fs::write(ctx.home_dir.join(".codex/AGENTS.md"), "external content\n").unwrap();
        let status = build_status(&ctx).unwrap();
        let codex = element(&status, 0);
        assert_eq!(codex["state"], "unmanaged-detected");
        assert!(codex["observed"]["blocked_reason"].as_str().unwrap().len() > 0);
        assert_eq!(codex["actions"], json!([]));
        drop_root(&root);

        // 绑定后外部修改
        let root = temp_root("tamper");
        let ctx = ctx_at(&root);
        std::fs::create_dir_all(ctx.home_dir.join(".codex")).unwrap();
        let spec = &targets()[0];
        let binding = binding_for(&ctx, spec);
        write_binding(&ctx, &binding);
        std::fs::write(ctx.home_dir.join(spec.target), "tampered bytes\n").unwrap();
        let status = build_status(&ctx).unwrap();
        let codex = element(&status, 0);
        assert_eq!(codex["state"], "unmanaged-detected");
        assert!(codex["observed"]["blocked_reason"].as_str().is_some());
        drop_root(&root);
    }

    #[cfg(unix)]
    #[test]
    fn unmanaged_detected_target_symlink() {
        let root = temp_root("link");
        let ctx = ctx_at(&root);
        std::fs::create_dir_all(ctx.home_dir.join(".codex")).unwrap();
        let elsewhere = root.join("elsewhere.md");
        std::fs::write(&elsewhere, "x\n").unwrap();
        std::os::unix::fs::symlink(&elsewhere, ctx.home_dir.join(".codex/AGENTS.md")).unwrap();
        let status = build_status(&ctx).unwrap();
        let codex = element(&status, 0);
        assert_eq!(codex["state"], "unmanaged-detected");
        assert!(codex["observed"]["blocked_reason"].as_str().is_some());
        drop_root(&root);
    }

    #[cfg(unix)]
    #[test]
    fn unmanaged_detected_ancestor_symlink() {
        let root = temp_root("ancestor");
        let ctx = ctx_at(&root);
        let real = root.join("real");
        std::fs::create_dir_all(real.join(".codex")).unwrap();
        std::fs::write(real.join(".codex/AGENTS.md"), "y\n").unwrap();
        std::fs::create_dir_all(&ctx.home_dir).unwrap();
        std::os::unix::fs::symlink(real.join(".codex"), ctx.home_dir.join(".codex")).unwrap();
        let status = build_status(&ctx).unwrap();
        let codex = element(&status, 0);
        assert_eq!(codex["state"], "unmanaged-detected");
        assert!(codex["observed"]["blocked_reason"].as_str().is_some());
        drop_root(&root);
    }

    #[test]
    fn status_json_never_contains_rule_bodies() {
        let root = temp_root("noleak");
        let ctx = ctx_at(&root);
        std::fs::create_dir_all(ctx.home_dir.join(".codex")).unwrap();
        let snap = load_snapshot(&ctx).unwrap();
        let rendered = render_rules(&snap.sources[KEY_SHARED], &snap.sources[KEY_CODEX]);
        std::fs::write(ctx.home_dir.join(".codex/AGENTS.md"), &rendered).unwrap();
        let status = build_status(&ctx).unwrap();
        let serialized = serde_json::to_string(&status).unwrap();
        for text in [
            snap.sources[KEY_SHARED].as_str(),
            snap.sources[KEY_CLAUDE].as_str(),
            snap.sources[KEY_CODEX].as_str(),
            rendered.as_str(),
        ] {
            for line in text.lines().filter(|l| !l.trim().is_empty()) {
                assert!(
                    !serialized.contains(line),
                    "status JSON 泄漏规则正文行: {line:?}"
                );
            }
        }
        drop_root(&root);
    }
}
