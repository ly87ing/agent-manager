//! aliases 域 apply 核心：preflight 纯读校验 + 单 Immediate 事务内重检、
//! 原子 CAS 写入（精确旧字节）、binding 登记与单条审计。任何阻塞（外部
//! 修改/不安全路径/来源或绑定损坏/目标非 UTF-8）fail-closed；失败时
//! RollbackGuard 恢复文件、DB 事务整体回滚。错误与审计只含 digest/revision
//! 元数据，不含 alias 正文。不承诺备份。

use anyhow::{anyhow, bail, Result};
use rusqlite::TransactionBehavior;
use serde_json::{json, Value};

use super::model::{self, TargetSpec};
use super::persistence::Binding;
use super::{args, preflight, status};
use crate::native::{Context, Request};
use crate::native_mcp::model::{now_iso, sha256_hex, sha256_hex_bytes};
use crate::native_mcp::rollback::{atomic_write_bytes_checked, read_existing_bytes, RollbackGuard};
use crate::native_mcp::targets::validate_target_path;
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
        "domain": "aliases",
        "steps": steps,
        "skipped": skipped,
    })
}

/// 单目标判定：返回 Skip 或 Write。任何外部修改/不安全路径 fail-closed。
fn decide(
    ctx: &Context,
    spec: &TargetSpec,
    cfg: &serde_json::Map<String, Value>,
    source_revision: &str,
    bindings: &std::collections::BTreeMap<String, Binding>,
) -> Result<Decision> {
    let alias_lines = model::render_alias_lines(cfg);
    let expected_inner = alias_lines.join("\n");

    let abs = ctx.home_dir.join(spec.target);
    validate_target_path(&abs)?;
    let old_bytes = read_existing_bytes(&abs)?;
    let binding = bindings.get(spec.target_id);

    let make_binding = |applied_digest: String| Binding {
        target_id: spec.target_id.to_string(),
        applied_digest,
        // 写入后区块正文就是期望正文；期望为空表示区块被移除，记空串。
        applied_block_digest: if expected_inner.is_empty() {
            String::new()
        } else {
            sha256_hex(&expected_inner)
        },
        source_revision: source_revision.to_string(),
        updated_at: now_iso(),
    };

    let Some(old) = old_bytes else {
        if expected_inner.is_empty() {
            // 缺失且无 alias：不建空文件。
            return Ok(Decision::Skip);
        }
        let expected =
            model::converge_content(b"", &expected_inner)?.expect("空文件 + 非空期望必产生新内容");
        let applied = sha256_hex_bytes(&expected);
        return Ok(Decision::Write {
            expected,
            old_bytes: None,
            binding: make_binding(applied),
        });
    };

    // 锁内重读：只比受管区块。区块外的字节本来就会原样保留，用户往自己的
    // ~/.zshrc 里加东西不该让同步停摆；区块被人改过才 fail-closed 拒绝覆盖。
    if let Some(b) = binding {
        let text = String::from_utf8_lossy(&old);
        let (found, inner) = model::extract_alias_block(&text);
        if b.applied_block_digest.is_empty() {
            // 旧行（补列默认值）：没有区块摘要可比，回退到整文件比较。
            if sha256_hex_bytes(&old) != b.applied_digest {
                bail!("目标 {} 存在外部修改，拒绝覆盖", spec.target_id);
            }
        } else if found && sha256_hex(&inner) != b.applied_block_digest {
            bail!("目标 {} 的受管区块被外部修改，拒绝覆盖", spec.target_id);
        }
        // 区块整段不见了：按缺失处理，下面照常重新写入，不算冲突。
    }

    let Some(expected) = model::converge_content(&old, &expected_inner)? else {
        return Ok(Decision::Skip);
    };
    let applied = sha256_hex_bytes(&expected);
    Ok(Decision::Write {
        expected,
        old_bytes: Some(old),
        binding: make_binding(applied),
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
    let canonical = crate::native_sources::current_alias_source_from(&tx)?;
    let cfg = status::parse_source(&canonical)?;
    let source_revision = sha256_hex(&canonical);
    let bindings = super::persistence::load_bindings_from(&tx)?;

    let mut pending: Vec<(String, std::path::PathBuf, Decision)> = Vec::new();

    for spec in model::targets() {
        if !selected.iter().any(|id| id == spec.target_id) {
            continue;
        }
        let decision = decide(ctx, &spec, &cfg, &source_revision, &bindings)?;
        if !matches!(decision, Decision::Skip) {
            pending.push((
                spec.target_id.to_string(),
                ctx.home_dir.join(spec.target),
                decision,
            ));
        }
    }

    // 没有任何待写：直接提交（不建表、不写审计）；选中目标全部计入 skipped，
    // 与 rules/hooks 的 no-op apply 语义一致。
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
                "name": format!("aliases:sync {id}"),
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
    let audit_target = if ids.len() == 1 { ids[0] } else { "aliases" };
    let audit_result = store::record_operation(
        &tx,
        Some(&req.run_id),
        "aliases:apply",
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

    Ok(result_json(steps, vec![]))
}
