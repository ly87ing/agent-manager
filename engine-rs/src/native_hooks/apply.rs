//! hooks 域写入：把新 hooks 写进一个目标的实际文件。
//!
//! 新 hooks 来自直接编辑（--value）、一条清理建议（--cleanup，移除命中内置清理规则的那条）
//! 或一条旧版本保存的 hook（--legacy，加入实际配置）。Claude 只替换 settings.json 的
//! `hooks` 键，其余键（接入、运行参数与用户自己的设置）原样保留；Codex 写 hooks.json。
//! 写入前重读实际内容并与调用方传入的 base（status 输出）比对，不一致即拒绝（「已在外部
//! 修改」），文件与库都不变。锁前预检零写；锁内 Immediate 事务重检、原子 CAS 写入、
//! 对主助手目标做旧来源的写路径清理、单条审计；失败时回滚文件、事务整体回滚。
//! 错误与审计只含 id/摘要等元数据，不含 hook 命令。

use anyhow::{anyhow, bail, Result};
use rusqlite::TransactionBehavior;
use serde_json::{json, Value};

use super::args::{self, ApplyArgs, Source};
use super::legacy::{self, Pruned, Rows};
use super::model::{self, Target};
use super::status::{self, Probe};
use crate::native::{Context, Request};
use crate::native_mcp::rollback::{atomic_write_bytes_checked, RollbackGuard};
use crate::store;

/// 解析好、可以写的一次操作。
struct Prepared {
    target: Target,
    /// 实际写入的文件：目标是符号链接时为解析后的最终文件，链接本身不变。
    real: std::path::PathBuf,
    old_bytes: Option<Vec<u8>>,
    /// None：新 hooks 与现有一致（或文件不存在且新 hooks 为空），不写。
    new_bytes: Option<Vec<u8>>,
    before: Value,
    after: Value,
}

fn find<'a>(targets: &'a [Target], id: &str) -> Result<&'a Target> {
    targets
        .iter()
        .find(|t| t.id == id)
        .ok_or_else(|| anyhow!("Hooks 目标不存在（实例可能已被删除）"))
}

fn json_contains_nul(value: &Value) -> bool {
    match value {
        Value::String(s) => s.contains('\0'),
        Value::Array(items) => items.iter().any(json_contains_nul),
        Value::Object(map) => map
            .iter()
            .any(|(key, item)| key.contains('\0') || json_contains_nul(item)),
        _ => false,
    }
}

/// 基于一次读取的 policy/旧来源与实际文件，确定写入内容。锁前预检与锁内执行都调用它，
/// 锁内结果才是最终依据。
fn prepare(targets: &[Target], rows: &Rows, parsed: &ApplyArgs) -> Result<Prepared> {
    let target = find(targets, &parsed.target)?.clone();
    let (old_bytes, root, real) = match status::probe(&target) {
        Probe::Disabled => bail!("{} 所属助手已停用，不写 hooks", target.name),
        Probe::Blocked { reason, .. } => bail!("{} 当前不能读写：{reason}", target.name),
        Probe::Missing {
            reason: Some(reason),
            ..
        } => bail!("{reason}"),
        Probe::Missing { reason: None, real } => (None, None, real),
        Probe::Present { bytes, root, real } => (Some(bytes), Some(root), real),
    };
    let base = model::base_of(target.kind, old_bytes.as_deref().zip(root.as_ref()));
    if base != parsed.base {
        bail!(
            "{} 已在外部修改（与打开时读到的内容不同），没有写入；请刷新后重新操作",
            target.name
        );
    }
    let before = root
        .as_ref()
        .map(model::hooks_of)
        .unwrap_or_else(|| json!({}));
    let after = match &parsed.source {
        Source::Value(text) => {
            let value: Value =
                serde_json::from_str(text).map_err(|_| anyhow!("--value 不是合法 JSON"))?;
            if json_contains_nul(&value) {
                bail!("hooks 内容不能包含 NUL 字符");
            }
            let map = model::validate_hooks_map(Some(&value))
                .map_err(|err| anyhow!("hooks 结构非法: {err:#}"))?;
            Value::Object(map)
        }
        Source::Cleanup(id) => {
            let item = model::cleanup_suggestions(target.kind, &before)
                .into_iter()
                .find(|c| &c.id == id)
                .ok_or_else(|| anyhow!("这条清理建议已不存在，请刷新后重试"))?;
            model::remove_signature(&before, &item.signature)
        }
        Source::Legacy(id) => {
            let item = target
                .is_main()
                .then(|| legacy::suggestions(rows, target.kind, &before))
                .unwrap_or_default()
                .into_iter()
                .find(|c| &c.id == id)
                .ok_or_else(|| anyhow!("这条旧版 hooks 已不存在或不属于这个目标，请刷新后重试"))?;
            model::add_hook(&before, &item.event, &item.entry_fields, &item.hook)
        }
    };
    // 清理与加入的结果同样过一遍结构校验，不写出坏结构。
    let after_map = model::validate_hooks_map(Some(&after))
        .map_err(|err| anyhow!("hooks 结构非法: {err:#}"))?;
    let new_bytes = if after == before {
        None
    } else {
        model::render_target(target.kind, root.as_ref(), &after_map)
    };
    Ok(Prepared {
        target,
        real,
        old_bytes,
        new_bytes,
        before,
        after,
    })
}

/// 锁前预检：严格解析 + 只读校验（目标可写、base 一致、建议仍在、新 hooks 结构合法）。
/// 无效/阻塞预检不创建 data/home/DB/表/文件。
pub(crate) fn preflight_apply(ctx: &Context, req: &Request) -> Result<()> {
    let parsed = args::parse_apply_args(req)?;
    let (policy, rows) = status::load_snapshot(ctx)?;
    let targets = model::resolve_targets(ctx, &policy);
    prepare(&targets, &rows, &parsed)?;
    Ok(())
}

/// 与 apply 共用写入计算，只返回目标文件的候选原文；不创建数据库或文件。
pub(crate) fn preview(ctx: &Context, req: &Request) -> Result<Value> {
    let parsed = args::parse_apply_args(req)?;
    let (policy, rows) = status::load_snapshot(ctx)?;
    let targets = model::resolve_targets(ctx, &policy);
    let prep = prepare(&targets, &rows, &parsed)?;
    let will_write = prep.new_bytes.is_some();
    let bytes = prep.new_bytes.as_ref().or(prep.old_bytes.as_ref());
    let content = match bytes {
        Some(bytes) if bytes.len() <= 4 * 1024 * 1024 => {
            Some(std::str::from_utf8(bytes).map_err(|_| anyhow!("目标文件不是 UTF-8"))?)
        }
        Some(_) => bail!("目标文件超过预览大小限制（4 MiB）"),
        None => None,
    };
    Ok(json!({
        "path": prep.target.path.display().to_string(),
        "content": content,
        "exists": prep.old_bytes.is_some(),
        "will_write": will_write,
    }))
}

fn source_meta(source: &Source) -> Value {
    match source {
        Source::Value(_) => json!({ "kind": "edit" }),
        Source::Cleanup(id) => json!({ "kind": "cleanup", "suggestion": id }),
        Source::Legacy(id) => json!({ "kind": "legacy", "suggestion": id }),
    }
}

pub(crate) fn apply(ctx: &Context, req: &Request) -> Result<Value> {
    // 自身再次预检：直接调用 apply 不能绕过校验。
    preflight_apply(ctx, req)?;
    let parsed = args::parse_apply_args(req)?;

    let mut conn = store::open_write(ctx)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    // 锁内基于同一 snapshot 重新读取并重检，不沿用锁前的结果。
    let policy = store::policy_from_connection(&tx)?;
    let rows = legacy::read_rows(&tx);
    let targets = model::resolve_targets(ctx, &policy);
    let prep = prepare(&targets, &rows, &parsed)?;
    let target_id = prep.target.id.clone();

    let Some(new_bytes) = prep.new_bytes.as_deref() else {
        tx.commit()?;
        return Ok(json!({ "domain": "hooks", "steps": [], "skipped": [target_id] }));
    };

    // 写失败时事务随 drop 回滚，文件由 CAS 写保证未被替换。
    // CAS 比对的是最终目标的字节；原子写落在目标所在目录，符号链接本身不动。
    atomic_write_bytes_checked(&prep.real, new_bytes, prep.old_bytes.as_deref())?;
    let mut guard = RollbackGuard::new();
    guard.record_written(&prep.real, prep.old_bytes.clone(), new_bytes.to_vec());
    let steps =
        vec![json!({ "name": format!("hooks:write {target_id}"), "ok": true, "exit_code": 0 })];

    let finish = (|| -> Result<()> {
        // 写路径清理：写入前后已在主助手实际配置里的旧版 hooks 从库里删掉。
        let pruned = if prep.target.is_main() {
            let before = model::collect_signatures(&prep.before);
            let after = model::collect_signatures(&prep.after);
            legacy::prune_in_tx(&tx, &rows, prep.target.kind, &|sig| {
                before.contains(sig) || after.contains(sig)
            })?
        } else {
            Pruned::Unchanged
        };
        let new_root: Value = serde_json::from_slice(new_bytes)?;
        store::record_operation(
            &tx,
            Some(&req.run_id),
            "hooks:apply",
            Some(&target_id),
            Some(&json!({
                "action": "write",
                "source": source_meta(&parsed.source),
                "base": parsed.base,
                "after_base": model::base_of(prep.target.kind, Some((new_bytes, &new_root))),
                "legacy_pruned": pruned != Pruned::Unchanged,
            })),
        )?;
        Ok(())
    })();
    if let Err(err) = finish {
        drop(tx);
        if let Err(rb_err) = guard.rollback() {
            return Err(anyhow!(
                "执行失败: {err:#}；Hooks 文件恢复同时失败: {rb_err:#}"
            ));
        }
        return Err(err);
    }
    if let Err(err) = tx.commit() {
        if let Err(rb_err) = guard.rollback() {
            return Err(anyhow!(
                "提交失败: {err:#}；Hooks 文件恢复同时失败: {rb_err:#}"
            ));
        }
        return Err(err.into());
    }
    guard.commit();

    Ok(json!({ "domain": "hooks", "steps": steps, "skipped": [] }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_mcp::model::sha256_hex_bytes;
    use std::path::Path;

    fn temp_root(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "native-hooks-apply-unit-{}-{}-{:x}",
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

    fn req(args: &[&str]) -> Request {
        Request {
            command: "hooks:apply".to_string(),
            args: args.iter().map(|s| s.to_string()).collect(),
            run_id: "run-apply-test".to_string(),
        }
    }

    #[test]
    fn late_audit_failure_restores_codex_file() {
        let root = temp_root("rollback");
        let ctx = ctx_at(&root);
        let path = ctx.home_dir.join(".codex/hooks.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let original = b"{\n  \"hooks\": {}\n}\n".to_vec();
        std::fs::write(&path, &original).unwrap();
        {
            let conn = store::open_write(&ctx).unwrap();
            conn.execute_batch(
                "CREATE TRIGGER fail_operation_logs BEFORE INSERT ON operation_logs
                 BEGIN SELECT RAISE(ABORT, 'injected-end-failure'); END;",
            )
            .unwrap();
        }
        let base = sha256_hex_bytes(&original);
        let value = r#"{"Stop":[{"hooks":[{"type":"command","command":"echo new"}]}]}"#;
        let err = apply(
            &ctx,
            &req(&["--only", "hooks:codex", "--base", &base, "--value", value]),
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("injected-end-failure"));
        assert_eq!(
            std::fs::read(&path).unwrap(),
            original,
            "审计失败时文件恢复原样"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 锁内重判同样复用结构校验：外部把 hooks 换坏后即使 base 对得上也拒绝。
    #[test]
    fn structurally_invalid_target_is_rejected_before_render() {
        let root = temp_root("strict");
        let ctx = ctx_at(&root);
        let settings = ctx.home_dir.join(".claude/settings.json");
        std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
        let bad =
            json!({"hooks": {"SessionStart": [{"hooks": [{"type": "command"}]}]}}).to_string();
        std::fs::write(&settings, &bad).unwrap();
        let err = apply(
            &ctx,
            &req(&[
                "--only",
                "hooks:claude",
                "--base",
                "missing",
                "--value",
                "{}",
            ]),
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("不能读写"), "{err:#}");
        assert_eq!(std::fs::read_to_string(&settings).unwrap(), bad);
        let _ = std::fs::remove_dir_all(&root);
    }
}
