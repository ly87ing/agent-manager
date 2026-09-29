//! hooks 域只读状态：逐目标输出实际 hooks、写入基准 base，以及清理建议与旧版本保存的
//! hooks（未生效）建议。
//!
//! 生效 hooks 就是各助手的实际配置；status 只读零写：不创建 data/home 目录、DB、表或
//! 文件。DB 存在时只打开一次 readonly 连接，在同一 snapshot 读 policy 与旧来源；目标路径
//! 安全与解析问题降级为 unmanaged-detected + blocked_reason。目标是符号链接时与
//! agents:configure 同一裁决：解析到最终目标读写（链接本身不变），断链或指向目录拒绝。

use std::path::PathBuf;

use std::path::Path;

use anyhow::Result;
use serde_json::{json, Value};

use super::legacy::{self, Rows};
use super::model::{self, HookKind, Target};
use crate::native::Context;
use crate::native_mcp::targets::{check_symlink_metadata, validate_target_path};
use crate::store;

/// 一个目标的实际状态。
pub(crate) enum Probe {
    /// 助手或实例已停用：不探测。
    Disabled,
    /// 文件不存在；`reason` 非空表示当前不能新建（实例配置目录还不存在）。
    /// `real` 是要新建的落点（父目录已解析链接）。
    Missing {
        reason: Option<String>,
        real: PathBuf,
    },
    /// 文件存在、是 JSON 对象且 hooks 结构合法；`real` 是解析链接后的最终文件。
    Present {
        bytes: Vec<u8>,
        root: Value,
        real: PathBuf,
    },
    /// 不能安全读写。
    Blocked {
        exists: Option<bool>,
        reason: String,
    },
}

impl Probe {
    /// 实际 hooks：文件不存在或没有 hooks 键时为空对象；不能读时为 None。
    pub(crate) fn hooks(&self) -> Option<Value> {
        match self {
            Probe::Present { root, .. } => Some(model::hooks_of(root)),
            Probe::Missing { .. } => Some(json!({})),
            _ => None,
        }
    }

    pub(crate) fn base(&self, kind: HookKind) -> Option<String> {
        match self {
            Probe::Present { bytes, root, .. } => Some(model::base_of(kind, Some((bytes, root)))),
            Probe::Missing { .. } => Some(model::base_of(kind, None)),
            _ => None,
        }
    }

    pub(crate) fn writable(&self) -> bool {
        matches!(
            self,
            Probe::Present { .. } | Probe::Missing { reason: None, .. }
        )
    }
}

fn parent_is_dir(path: &Path) -> bool {
    path.parent().is_some_and(Path::is_dir)
}

/// 只读探测：链接先解析到最终目标；断链/指向目录/非普通文件/读取失败/非 JSON 对象/
/// hooks 结构非法一律 Blocked，绝不写。
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
    let real = match crate::native_agent_settings::resolve_write_target(&target.path) {
        Ok(real) => real,
        Err(err) => {
            return Probe::Blocked {
                exists: Some(true),
                reason: format!("{err:#}"),
            }
        }
    };
    let path = real.as_path();
    if let Err(err) = validate_target_path(path) {
        return Probe::Blocked {
            exists: None,
            reason: format!("{err:#}"),
        };
    }
    match check_symlink_metadata(path) {
        Ok(None) => {
            let reason = (target.needs_existing_dir && !parent_is_dir(path)).then(|| {
                "这个账号的配置目录还不存在；先在账号页把它准备好，再编辑自动命令".to_string()
            });
            return Probe::Missing { reason, real };
        }
        Ok(Some(meta)) if !meta.is_file() => {
            return Probe::Blocked {
                exists: Some(true),
                reason: "目标路径不是普通文件".to_string(),
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
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) => {
            return Probe::Blocked {
                exists: Some(true),
                reason: format!("无法读取目标文件: {err}"),
            }
        }
    };
    let root: Value = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        Err(_) => {
            return Probe::Blocked {
                exists: Some(true),
                reason: "目标文件不是合法 JSON".to_string(),
            }
        }
    };
    if !root.is_object() {
        return Probe::Blocked {
            exists: Some(true),
            reason: "目标文件 JSON 不是对象".to_string(),
        };
    }
    if let Err(err) = model::validate_target_hooks(&root) {
        return Probe::Blocked {
            exists: Some(true),
            reason: format!("{err:#}"),
        };
    }
    Probe::Present { bytes, root, real }
}

/// 同一 snapshot 读出的 policy 与旧来源；缺库时为内置 policy + 无旧来源（零写）。
pub(crate) fn load_snapshot(ctx: &Context) -> Result<(Value, Rows)> {
    match store::open_readonly(ctx)? {
        Some(mut conn) => {
            // 显式 Deferred 只读事务：两个读取在同一 SQLite snapshot 上执行，不抢写锁。
            let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Deferred)?;
            let policy = store::policy_from_connection(&tx)?;
            let rows = legacy::read_rows(&tx);
            tx.commit()?;
            Ok((policy, rows))
        }
        None => Ok((store::builtin_policy(), Rows::new())),
    }
}

fn element(target: &Target, rows: &Rows) -> Value {
    let probe = probe(target);
    let base = probe.base(target.kind);
    let writable = probe.writable();
    let hooks = probe.hooks();
    let (state, exists, blocked_reason) = match &probe {
        Probe::Disabled => ("disabled", Value::Null, None),
        Probe::Missing { reason, .. } => ("missing", json!(false), reason.clone()),
        Probe::Present { .. } => ("managed-ok", json!(true), None),
        Probe::Blocked { exists, reason } => (
            "unmanaged-detected",
            exists.map_or(Value::Null, Value::Bool),
            Some(reason.clone()),
        ),
    };
    // 建议只对能写的目标给：点了才能落地。
    let (cleanup, legacy) = match (&hooks, writable) {
        (Some(hooks), true) => (
            model::cleanup_suggestions(target.kind, hooks)
                .iter()
                .map(model::Cleanup::to_json)
                .collect::<Vec<_>>(),
            if target.is_main() {
                legacy::suggestions(rows, target.kind, hooks)
                    .iter()
                    .map(legacy::LegacyHook::to_json)
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            },
        ),
        _ => (Vec::new(), Vec::new()),
    };
    let actions = if writable {
        json!([{ "id": "write", "risk": "write-local", "needs_lock": true, "offline_ok": true }])
    } else {
        json!([])
    };
    json!({
        "id": target.id,
        "domain": "hooks",
        "spec": target.spec_json(),
        "policy": { "enabled": target.enabled, "agent_id": target.agent_id },
        "observed": {
            "target": target.path.display().to_string(),
            "target_exists": exists,
            "hooks": hooks,
            "hook_count": hooks.as_ref().map(model::hook_count).unwrap_or(0),
            "base": base,
            "cleanup": cleanup,
            "legacy": legacy,
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

pub(crate) fn build_status(ctx: &Context) -> Result<Value> {
    let (policy, rows) = load_snapshot(ctx)?;
    let elements: Vec<Value> = model::resolve_targets(ctx, &policy)
        .iter()
        .map(|target| element(target, &rows))
        .collect();
    Ok(json!({ "domain": "hooks", "elements": elements }))
}
