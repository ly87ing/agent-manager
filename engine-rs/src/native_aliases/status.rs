//! aliases 域只读状态闭环：目标固定为 model::targets()，来源只从
//! alias_sources 当前有效 JSON 取得（缺 DB/表用编译期默认），运行时不读
//! checkout。status 不创建 data/home 目录、DB、表或文件；来源/绑定损坏
//! fail-closed；目标路径安全与读取问题降级为 drifted + blocked_reason
//!（aliases 元素状态冻结为 managed-ok/drifted/missing/not-materialized，
//! 阻塞不新增状态，preflight/apply 对 blocked 目标一律拒绝）。输出不含来源正文。

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Result;
use serde_json::{json, Map, Value};

use super::model::{self, TargetSpec};
use super::persistence::{load_bindings_from, Binding};
use crate::native::Context;
use crate::native_mcp::model::{sha256_hex, sha256_hex_bytes};
use crate::native_mcp::targets::{check_symlink_metadata, validate_target_path};
use crate::native_sources;
use crate::store;

struct Snapshot {
    source: String,
    bindings: BTreeMap<String, Binding>,
}

fn load_snapshot(ctx: &Context) -> Result<Snapshot> {
    match store::open_readonly(ctx)? {
        Some(mut conn) => {
            // 显式 Deferred 只读事务：两个 SELECT 在同一 SQLite snapshot 上执行；
            // 不抢写锁，不写 DB。任一读取失败即传播 Err，事务随之回滚。
            let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Deferred)?;
            let snap = Snapshot {
                source: native_sources::current_alias_source_from(&tx)?,
                bindings: load_bindings_from(&tx)?,
            };
            tx.commit()?;
            Ok(snap)
        }
        None => Ok(Snapshot {
            source: native_sources::current_alias_source_readonly(ctx)?,
            bindings: BTreeMap::new(),
        }),
    }
}

/// 解析并校验来源 canonical JSON；损坏 fail-closed，错误不回显正文。
pub(crate) fn parse_source(canonical: &str) -> Result<Map<String, Value>> {
    let value: Value = serde_json::from_str(canonical)
        .map_err(|_| anyhow::anyhow!("aliases 来源 aliases.shell 不是合法 JSON 对象"))?;
    let obj = value
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("aliases 来源 aliases.shell 不是合法 JSON 对象"))?;
    model::validate_source(obj)
        .map_err(|err| anyhow::anyhow!("aliases 来源 aliases.shell 结构非法: {err:#}"))?;
    Ok(obj.clone())
}

enum TargetProbe {
    Missing,
    Present {
        bytes: Vec<u8>,
    },
    Blocked {
        exists: Option<bool>,
        reason: String,
    },
}

/// 只读探测目标：路径安全校验 + 读取字节；绝不写文件。区块结构与 UTF-8
/// 合法性在收敛计算时判定，status 只做字节级观察。
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
    match std::fs::read(abs_path) {
        Ok(bytes) => {
            if std::str::from_utf8(&bytes).is_err() {
                return TargetProbe::Blocked {
                    exists: Some(true),
                    reason: "目标文件不是合法 UTF-8".to_string(),
                };
            }
            TargetProbe::Present { bytes }
        }
        Err(err) => TargetProbe::Blocked {
            exists: Some(true),
            reason: format!("无法读取目标文件: {err}"),
        },
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

/// 逐条生效回读：每条快捷命令的期望取值、目标里的实际取值与是否一致。
///
/// 只回答「配置文件里现在是什么」——终端会话是否已加载是另一回事（要新开窗口），
/// 那个本工具看不到，也不假装能看到。
fn alias_entries(alias_lines: &[String], actual_block: Option<&str>) -> Value {
    let actual = actual_block
        .map(model::parse_alias_lines)
        .unwrap_or_default();
    let mut out = Vec::new();
    for expected_line in alias_lines {
        let name = expected_line
            .strip_prefix("alias ")
            .and_then(|rest| rest.split_once('='))
            .map(|(name, _)| name.trim().to_string())
            .unwrap_or_default();
        let actual_line = actual.get(&name).cloned();
        let state = match (&actual_line, actual_block.is_some()) {
            (Some(line), _) if line == expected_line => "applied",
            (Some(_), _) => "outdated",
            // 区块都还没写进去，与「区块在但少了这条」区分开
            (None, true) => "missing",
            (None, false) => "not-materialized",
        };
        out.push(json!({
            "name": name,
            "expected": expected_line,
            "actual": actual_line,
            "state": state,
        }));
    }
    Value::Array(out)
}

fn build_element(
    ctx: &Context,
    snap: &Snapshot,
    spec: &TargetSpec,
    cfg: &Map<String, Value>,
) -> Value {
    let alias_lines = model::render_alias_lines(cfg);
    let expected_inner = alias_lines.join("\n");
    let expected_digest = sha256_hex(&expected_inner);
    let source_revision = sha256_hex(&snap.source);
    let binding = snap.bindings.get(spec.target_id);
    let applied_digest = binding.map(|b| b.applied_digest.clone());

    let abs_target = ctx.home_dir.join(spec.target);
    let abs_display = abs_target.display().to_string();

    let mut actual_block: Option<String> = None;
    let (state, target_exists, block_present, actual_digest, actual_block_digest, blocked_reason) =
        match probe_target(&abs_target) {
            TargetProbe::Blocked { exists, reason } => (
                "drifted",
                exists.map_or(Value::Null, Value::Bool),
                Value::Null,
                Value::Null,
                Value::Null,
                Value::String(reason),
            ),
            TargetProbe::Missing => (
                if alias_lines.is_empty() {
                    "managed-ok"
                } else {
                    "not-materialized"
                },
                Value::Bool(false),
                Value::Bool(false),
                Value::Null,
                Value::Null,
                Value::Null,
            ),
            TargetProbe::Present { bytes } => {
                let digest = sha256_hex_bytes(&bytes);
                let text = String::from_utf8_lossy(&bytes);
                let (found, inner) = model::extract_alias_block(&text);
                if found {
                    actual_block = Some(inner.clone());
                }
                let block_digest = if found {
                    Value::String(sha256_hex(&inner))
                } else {
                    Value::Null
                };
                if let Some(b) = binding {
                    // 只比受管区块：区块外的字节本来就会保留，用户改自己的那部分
                    // 不该阻塞同步。旧行（没有区块摘要）回退到整文件比较。
                    let block_conflict = if b.applied_block_digest.is_empty() {
                        digest != b.applied_digest
                    } else {
                        found && sha256_hex(&inner) != b.applied_block_digest
                    };
                    if block_conflict {
                        (
                            "drifted",
                            Value::Bool(true),
                            Value::Bool(found),
                            Value::String(digest),
                            block_digest,
                            Value::String("受管区块被外部修改，拒绝覆盖".to_string()),
                        )
                    } else {
                        let state = if expected_inner.is_empty() && !found {
                            "managed-ok"
                        } else if found && inner == expected_inner {
                            "managed-ok"
                        } else if found {
                            "drifted"
                        } else {
                            "missing"
                        };
                        (
                            state,
                            Value::Bool(true),
                            Value::Bool(found),
                            Value::String(digest),
                            block_digest,
                            Value::Null,
                        )
                    }
                } else {
                    // 无绑定：只按受管区块判收敛（legacy bash 托管文件可安全登记）。
                    let state = if found && inner == expected_inner {
                        "managed-ok"
                    } else if found {
                        "drifted"
                    } else if alias_lines.is_empty() {
                        "managed-ok"
                    } else {
                        "missing"
                    };
                    (
                        state,
                        Value::Bool(true),
                        Value::Bool(found),
                        Value::String(digest),
                        block_digest,
                        Value::Null,
                    )
                }
            }
        };

    let has_sync =
        blocked_reason.is_null() && matches!(state, "drifted" | "missing" | "not-materialized");
    let names: Vec<String> = cfg
        .get("aliases")
        .and_then(Value::as_object)
        .map(|m| {
            let mut names: Vec<String> = m.keys().cloned().collect();
            names.sort();
            names
        })
        .unwrap_or_default();
    json!({
        "id": spec.target_id,
        "domain": "aliases",
        "spec": {
            "name": spec.name,
            "source_key": native_sources::KEY_ALIAS_SHELL,
            "target": format!("~/{}", spec.target),
            "marker": "agent-manager shell aliases",
            "aliases": names,
        },
        "policy": { "enabled": true },
        "observed": {
            "target": abs_display,
            "target_exists": target_exists,
            "block_present": block_present,
            "alias_count": alias_lines.len() as u64,
            "expected_digest": expected_digest,
            "applied_digest": applied_digest,
            "actual_digest": actual_digest,
            "actual_block_digest": actual_block_digest,
            "source_revision": source_revision,
            "blocked_reason": blocked_reason,
            "registration_needed": false,
            "entries": alias_entries(&alias_lines, actual_block.as_deref()),
        },
        "state": state,
        "freshness": freshness_json(),
        "actions": if has_sync { sync_action() } else { json!([]) },
    })
}

pub(crate) fn build_status(ctx: &Context) -> Result<Value> {
    let snap = load_snapshot(ctx)?;
    let cfg = parse_source(&snap.source)?;
    let mut elements = Vec::new();
    for spec in model::targets() {
        elements.push(build_element(ctx, &snap, &spec, &cfg));
    }
    Ok(json!({
        "domain": "aliases",
        "elements": elements,
    }))
}
