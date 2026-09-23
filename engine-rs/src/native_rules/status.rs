//! rules 域只读状态：逐目标输出实际规则文件的全文、base 摘要与匹配到的模板，
//! 另附已保存模板与内置推荐。
//!
//! 生效规则就是各助手的规则文件；status 只读零写：不创建 data/home 目录、DB、表或
//! 文件。DB 存在时只打开一次 readonly 连接，在同一 snapshot 读 policy 与模板；
//! 模板表损坏 fail-closed；目标路径安全问题降级为 unmanaged-detected + blocked_reason。

use std::path::Path;

use anyhow::Result;
use serde_json::{json, Value};

use super::model::{self, Target, RECOMMENDED_BODY, RECOMMENDED_ID, RECOMMENDED_NAME};
use super::templates::{self, Template};
use crate::native::Context;
use crate::native_mcp::model::{sha256_hex, sha256_hex_bytes};
use crate::native_mcp::targets::{check_symlink_metadata, validate_target_path};
use crate::store;

/// 文件不存在时的 base 取值：让「看的时候没有、写的时候已被别人建出来」也能被识别。
pub(crate) const BASE_MISSING: &str = "missing";

/// 一个目标的实际状态。
pub(crate) enum Probe {
    /// 助手或实例已停用：不探测。
    Disabled,
    /// 文件不存在；`reason` 非空表示当前不能新建（实例配置目录还不存在）。
    Missing { reason: Option<String> },
    /// 文件存在且是 UTF-8 文本。
    Present { bytes: Vec<u8>, text: String },
    /// 不能安全读写。
    Blocked {
        exists: Option<bool>,
        reason: String,
    },
}

impl Probe {
    /// 当前 base：存在为字节 sha256，不存在为 "missing"，其余为 None。
    pub(crate) fn base(&self) -> Option<String> {
        match self {
            Probe::Present { bytes, .. } => Some(sha256_hex_bytes(bytes)),
            Probe::Missing { .. } => Some(BASE_MISSING.to_string()),
            _ => None,
        }
    }

    pub(crate) fn writable(&self) -> bool {
        matches!(
            self,
            Probe::Present { .. } | Probe::Missing { reason: None }
        )
    }
}

/// 只读探测：符号链接/祖先链接/目录/非普通文件/读取失败/非 UTF-8 一律 Blocked，绝不写。
pub(crate) fn probe(target: &Target) -> Probe {
    if !target.enabled {
        return Probe::Disabled;
    }
    if let Some(reason) = &target.blocked {
        return Probe::Blocked {
            exists: None,
            reason: reason.clone(),
        };
    }
    let path = target.path.as_path();
    if let Err(err) = validate_target_path(path) {
        return Probe::Blocked {
            exists: None,
            reason: format!("{err:#}"),
        };
    }
    match check_symlink_metadata(path) {
        Ok(None) => {
            let reason = (target.needs_existing_dir && !parent_is_dir(path)).then(|| {
                "实例配置目录还不存在；先在实例页应用一次这个实例，再为它编辑规则".to_string()
            });
            return Probe::Missing { reason };
        }
        Ok(Some(meta)) if !meta.is_file() => {
            return Probe::Blocked {
                exists: Some(true),
                reason: format!("目标路径不是普通文件: {}", path.display()),
            }
        }
        Ok(Some(_)) => {}
        Err(err) => {
            return Probe::Blocked {
                exists: None,
                reason: format!("{err:#}"),
            }
        }
    }
    match std::fs::read(path) {
        Ok(bytes) => match String::from_utf8(bytes.clone()) {
            Ok(text) => Probe::Present { bytes, text },
            Err(_) => Probe::Blocked {
                exists: Some(true),
                reason: "规则文件不是 UTF-8 文本，这里无法显示或编辑".to_string(),
            },
        },
        Err(err) => Probe::Blocked {
            exists: Some(true),
            reason: format!("无法读取规则文件 {}: {err}", path.display()),
        },
    }
}

fn parent_is_dir(path: &Path) -> bool {
    path.parent().is_some_and(Path::is_dir)
}

/// 同一 snapshot 读出的 policy 与模板；缺库时为内置 policy + 空模板（零写）。
pub(crate) fn load_snapshot(ctx: &Context) -> Result<(Value, Vec<Template>)> {
    match store::open_readonly(ctx)? {
        Some(mut conn) => {
            // 显式 Deferred 只读事务：两个读取在同一 SQLite snapshot 上执行，不抢写锁。
            let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Deferred)?;
            let policy = store::policy_from_connection(&tx)?;
            let list = templates::load_from(&tx)?;
            tx.commit()?;
            Ok((policy, list))
        }
        None => Ok((store::builtin_policy(), Vec::new())),
    }
}

fn element(target: &Target, templates: &[Template]) -> Value {
    let probe = probe(target);
    let base = probe.base();
    let writable = probe.writable();
    let (state, exists, content, blocked_reason) = match &probe {
        Probe::Disabled => ("disabled", Value::Null, None, None),
        Probe::Missing { reason } => ("missing", json!(false), None, reason.clone()),
        Probe::Present { text, .. } => ("managed-ok", json!(true), Some(text.as_str()), None),
        Probe::Blocked { exists, reason } => (
            "unmanaged-detected",
            exists.map_or(Value::Null, Value::Bool),
            None,
            Some(reason.clone()),
        ),
    };
    let matched = content.and_then(|text| templates.iter().find(|t| t.body == text));
    let actions = if writable {
        json!([{ "id": "write", "risk": "write-local", "needs_lock": true, "offline_ok": true }])
    } else {
        json!([])
    };
    json!({
        "id": target.id,
        "domain": "rules",
        "spec": {
            "name": target.name,
            "group": target.group,
            "kind": target.kind.as_str(),
            "profile_id": target.profile_id,
            "target": target.display,
        },
        "policy": { "enabled": target.enabled, "agent_id": target.agent_id },
        "observed": {
            "target": target.path.display().to_string(),
            "target_exists": exists,
            "content": content,
            "base": base,
            "matched_template": matched.map(|t| json!({ "id": t.id, "name": t.name })),
            "matches_recommended": content == Some(RECOMMENDED_BODY),
            "blocked_reason": blocked_reason,
        },
        "state": state,
        "freshness": {
            "state": "not-applicable",
            "current": Value::Null,
            "upstream": Value::Null,
            "checked_at": Value::Null,
            "probe": "native",
            "cache_age_sec": Value::Null,
        },
        "actions": actions,
    })
}

pub(crate) fn recommended_json() -> Value {
    json!({
        "id": RECOMMENDED_ID,
        "name": RECOMMENDED_NAME,
        "body": RECOMMENDED_BODY,
        "revision": sha256_hex(RECOMMENDED_BODY),
    })
}

pub(crate) fn build_status(ctx: &Context) -> Result<Value> {
    let (policy, list) = load_snapshot(ctx)?;
    let elements: Vec<Value> = model::resolve_targets(ctx, &policy)
        .iter()
        .map(|target| element(target, &list))
        .collect();
    Ok(json!({
        "domain": "rules",
        "elements": elements,
        "templates": list.iter().map(Template::to_json).collect::<Vec<_>>(),
        "recommended": recommended_json(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "native-rules-status-test-{}-{}-{:x}",
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

    fn element_of<'a>(status: &'a Value, id: &str) -> &'a Value {
        status["elements"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["id"] == id)
            .unwrap()
    }

    #[test]
    fn no_db_no_dirs_returns_targets_and_recommended_without_creating_anything() {
        let root = temp_root("empty");
        let ctx = ctx_at(&root);
        let status = build_status(&ctx).unwrap();
        let els = status["elements"].as_array().unwrap();
        assert_eq!(els.len(), 2);
        assert_eq!(els[0]["id"], "rules:codex_global");
        assert_eq!(els[1]["id"], "rules:claude_global");
        for el in els {
            assert_eq!(el["state"], "missing");
            assert_eq!(el["observed"]["base"], "missing");
            assert_eq!(el["observed"]["content"], Value::Null);
            assert_eq!(el["actions"][0]["id"], "write");
        }
        assert_eq!(status["templates"], json!([]));
        assert_eq!(status["recommended"]["body"], RECOMMENDED_BODY);
        assert!(!ctx.data_dir.exists());
        assert!(!ctx.home_dir.exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn present_file_reports_full_text_base_and_recommended_match() {
        let root = temp_root("present");
        let ctx = ctx_at(&root);
        std::fs::create_dir_all(ctx.home_dir.join(".claude")).unwrap();
        std::fs::write(ctx.home_dir.join(".claude/CLAUDE.md"), RECOMMENDED_BODY).unwrap();
        std::fs::create_dir_all(ctx.home_dir.join(".codex")).unwrap();
        std::fs::write(ctx.home_dir.join(".codex/AGENTS.md"), "我自己的规则\n").unwrap();
        let status = build_status(&ctx).unwrap();
        let claude = element_of(&status, "rules:claude_global");
        assert_eq!(claude["state"], "managed-ok");
        assert_eq!(claude["observed"]["content"], RECOMMENDED_BODY);
        assert_eq!(claude["observed"]["matches_recommended"], true);
        assert_eq!(
            claude["observed"]["base"],
            sha256_hex_bytes(RECOMMENDED_BODY.as_bytes())
        );
        let codex = element_of(&status, "rules:codex_global");
        assert_eq!(codex["observed"]["content"], "我自己的规则\n");
        assert_eq!(codex["observed"]["matches_recommended"], false);
        assert_eq!(codex["observed"]["matched_template"], Value::Null);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn non_utf8_file_is_blocked_without_content_or_actions() {
        let root = temp_root("binary");
        let ctx = ctx_at(&root);
        std::fs::create_dir_all(ctx.home_dir.join(".codex")).unwrap();
        std::fs::write(ctx.home_dir.join(".codex/AGENTS.md"), [0xff, 0xfe, 0x00]).unwrap();
        let status = build_status(&ctx).unwrap();
        let codex = element_of(&status, "rules:codex_global");
        assert_eq!(codex["state"], "unmanaged-detected");
        assert_eq!(codex["observed"]["content"], Value::Null);
        assert_eq!(codex["observed"]["base"], Value::Null);
        assert!(codex["observed"]["blocked_reason"].as_str().is_some());
        assert_eq!(codex["actions"], json!([]));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_target_and_ancestor_are_blocked() {
        let root = temp_root("link");
        let ctx = ctx_at(&root);
        std::fs::create_dir_all(ctx.home_dir.join(".codex")).unwrap();
        let elsewhere = root.join("elsewhere.md");
        std::fs::write(&elsewhere, "x\n").unwrap();
        std::os::unix::fs::symlink(&elsewhere, ctx.home_dir.join(".codex/AGENTS.md")).unwrap();
        let real = root.join("real-claude");
        std::fs::create_dir_all(&real).unwrap();
        std::fs::write(real.join("CLAUDE.md"), "y\n").unwrap();
        std::os::unix::fs::symlink(&real, ctx.home_dir.join(".claude")).unwrap();
        let status = build_status(&ctx).unwrap();
        for id in ["rules:codex_global", "rules:claude_global"] {
            let el = element_of(&status, id);
            assert_eq!(el["state"], "unmanaged-detected", "{id}");
            assert_eq!(el["observed"]["content"], Value::Null, "{id}");
            assert_eq!(el["actions"], json!([]), "{id}");
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn disabled_agent_target_is_not_probed() {
        let root = temp_root("disabled");
        let ctx = ctx_at(&root);
        let mut policy = store::builtin_policy();
        policy["agents"]["codex"]["enabled"] = Value::Bool(false);
        let conn = store::open_write(&ctx).unwrap();
        store::write_policy(&conn, &policy).unwrap();
        drop(conn);
        std::fs::create_dir_all(ctx.home_dir.join(".codex")).unwrap();
        std::fs::write(ctx.home_dir.join(".codex/AGENTS.md"), "anything\n").unwrap();
        let status = build_status(&ctx).unwrap();
        let codex = element_of(&status, "rules:codex_global");
        assert_eq!(codex["state"], "disabled");
        assert_eq!(codex["observed"]["target_exists"], Value::Null);
        assert_eq!(codex["observed"]["content"], Value::Null);
        assert_eq!(codex["actions"], json!([]));
        let _ = std::fs::remove_dir_all(&root);
    }
}
