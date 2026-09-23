//! rules 域写入：把新内容写进一个规则目标的实际文件。
//!
//! 新内容来自直接编辑（--value）、已保存模板、内置推荐或另一目标的实际正文。
//! 写入前重读实际内容并与调用方传入的 base（status 输出）比对，不一致即拒绝
//!（「已在外部修改」），文件与库都不变。用模板/推荐/复制覆盖之前先回填：目标现有
//! 内容非空、且对不上任何已保存模板或内置推荐时，另存为备份模板。锁前预检零写；
//! 锁内 Immediate 事务重检、原子 CAS 写入、单条审计；失败时回滚文件、事务整体回滚。
//! 错误与审计只含 id/摘要等元数据，不含规则正文。

use anyhow::{anyhow, bail, Result};
use rusqlite::TransactionBehavior;
use serde_json::{json, Value};

use super::args::{self, ApplyArgs, Source};
use super::model::{self, Target, RECOMMENDED_BODY};
use super::status::{self, Probe};
use super::templates::{self, Template};
use crate::native::{Context, Request};
use crate::native_mcp::model::sha256_hex_bytes;
use crate::native_mcp::rollback::{atomic_write_bytes_checked, RollbackGuard};
use crate::store;

/// 解析好、可以写的一次操作。
struct Prepared {
    target: Target,
    old_bytes: Option<Vec<u8>>,
    new_body: String,
    /// 需要先另存为备份模板的现有内容。
    backup: Option<String>,
}

fn find<'a>(targets: &'a [Target], id: &str) -> Result<&'a Target> {
    targets
        .iter()
        .find(|t| t.id == id)
        .ok_or_else(|| anyhow!("规则目标不存在（实例可能已被删除）"))
}

/// 目标能否写入；能写时返回现有字节与文本（不存在为 None）。
fn writable_content(target: &Target) -> Result<(Option<Vec<u8>>, Option<String>, String)> {
    let probe = status::probe(target);
    let base = probe.base();
    match probe {
        Probe::Disabled => bail!("{} 所属助手已停用，不写规则文件", target.name),
        Probe::Blocked { reason, .. } => bail!("{} 当前不能读写：{reason}", target.name),
        Probe::Missing {
            reason: Some(reason),
        } => bail!("{reason}"),
        Probe::Missing { reason: None } => Ok((None, None, base.expect("missing base"))),
        Probe::Present { bytes, text } => Ok((Some(bytes), Some(text), base.expect("base"))),
    }
}

/// 基于一次读取的 policy/模板与实际文件，确定写入内容与是否需要备份。
/// 锁前预检与锁内执行都调用它，锁内结果才是最终依据。
fn prepare(targets: &[Target], list: &[Template], parsed: &ApplyArgs) -> Result<Prepared> {
    let target = find(targets, &parsed.target)?.clone();
    let (old_bytes, old_text, base) = writable_content(&target)?;
    if base != parsed.base {
        bail!(
            "{} 已在外部修改（与打开时读到的内容不同），没有写入；请刷新后重新操作",
            target.name
        );
    }
    let new_body = match &parsed.source {
        Source::Value(text) => {
            if text.contains('\0') {
                bail!("规则正文不能包含 NUL 字符");
            }
            text.clone()
        }
        Source::Template(id) => list
            .iter()
            .find(|t| &t.id == id)
            .map(|t| t.body.clone())
            .ok_or_else(|| anyhow!("模板不存在或已被删除，请刷新后重试"))?,
        Source::Recommended => RECOMMENDED_BODY.to_string(),
        Source::From(from) => {
            if from == &target.id {
                bail!("来源与目标是同一个规则文件");
            }
            let source = find(targets, from)?;
            match writable_content(source)? {
                (_, Some(text), _) => text,
                (_, None, _) => bail!("{} 还没有规则文件，没有可复制的内容", source.name),
            }
        }
    };
    // 回填：只在用别处的内容覆盖时做；直接编辑本身就是在改这份内容。
    let backup = match (&parsed.source, old_text) {
        (Source::Value(_), _) | (_, None) => None,
        (_, Some(text)) => {
            let known = text == RECOMMENDED_BODY || list.iter().any(|t| t.body == text);
            if text.trim().is_empty() || known || text == new_body {
                None
            } else {
                if let Some(kind) = crate::native::reject_secret_text(&text) {
                    bail!(
                        "{} 现有内容疑似含凭据（{kind}），不能另存为备份模板，已停止；请先编辑去掉后再应用",
                        target.name
                    );
                }
                Some(text)
            }
        }
    };
    Ok(Prepared {
        target,
        old_bytes,
        new_body,
        backup,
    })
}

/// 锁前预检：严格解析 + 只读校验（目标可写、base 一致、来源可用）。
/// 无效/阻塞预检不创建 data/home/DB/表/文件。
pub(crate) fn preflight_apply(ctx: &Context, req: &Request) -> Result<()> {
    let parsed = args::parse_apply_args(req)?;
    let (policy, list) = status::load_snapshot(ctx)?;
    let targets = model::resolve_targets(ctx, &policy);
    prepare(&targets, &list, &parsed)?;
    Ok(())
}

fn source_meta(source: &Source) -> Value {
    match source {
        Source::Value(_) => json!({ "kind": "edit" }),
        Source::Template(id) => json!({ "kind": "template", "template": id }),
        Source::Recommended => json!({ "kind": "recommended" }),
        Source::From(from) => json!({ "kind": "copy", "from": from }),
    }
}

fn backup_name(target: &Target) -> String {
    format!(
        "备份 · {} · {}",
        target.name,
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
    )
}

pub(crate) fn apply(ctx: &Context, req: &Request) -> Result<Value> {
    // 自身再次预检：直接调用 apply 不能绕过校验。
    preflight_apply(ctx, req)?;
    let parsed = args::parse_apply_args(req)?;

    let mut conn = store::open_write(ctx)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    // 锁内基于同一 snapshot 重新读取并重检，不沿用锁前的结果。
    let policy = store::policy_from_connection(&tx)?;
    let list = templates::load_from(&tx)?;
    let targets = model::resolve_targets(ctx, &policy);
    let prep = prepare(&targets, &list, &parsed)?;
    let target_id = prep.target.id.clone();

    if prep.old_bytes.as_deref() == Some(prep.new_body.as_bytes()) {
        tx.commit()?;
        return Ok(json!({ "domain": "rules", "steps": [], "skipped": [target_id] }));
    }

    let mut steps = Vec::new();
    let mut backup_id: Option<String> = None;
    if let Some(body) = &prep.backup {
        templates::ensure_table(&tx)?;
        let now = templates::now();
        let template = Template {
            id: templates::new_id(),
            name: templates::unique_name(&list, &backup_name(&prep.target)),
            body: body.clone(),
            origin: templates::ORIGIN_BACKUP.to_string(),
            created_at: now.clone(),
            updated_at: now,
        };
        templates::insert(&tx, &template)?;
        steps.push(
            json!({ "name": format!("rules:backup {target_id}"), "ok": true, "exit_code": 0 }),
        );
        backup_id = Some(template.id);
    }

    let new_bytes = prep.new_body.as_bytes();
    // 写失败时事务随 drop 回滚（备份模板不落库），文件由 CAS 写保证未被替换。
    atomic_write_bytes_checked(&prep.target.path, new_bytes, prep.old_bytes.as_deref())?;
    let mut guard = RollbackGuard::new();
    guard.record_written(
        &prep.target.path,
        prep.old_bytes.clone(),
        new_bytes.to_vec(),
    );
    steps.push(json!({ "name": format!("rules:write {target_id}"), "ok": true, "exit_code": 0 }));

    let finish = (|| -> Result<()> {
        store::record_operation(
            &tx,
            Some(&req.run_id),
            "rules:apply",
            Some(&target_id),
            Some(&json!({
                "action": "write",
                "source": source_meta(&parsed.source),
                "base": parsed.base,
                "after_digest": sha256_hex_bytes(new_bytes),
                "backup_template": backup_id,
            })),
        )?;
        Ok(())
    })();
    if let Err(err) = finish {
        drop(tx);
        if let Err(rb_err) = guard.rollback() {
            return Err(anyhow!(
                "执行失败: {err:#}；规则文件恢复同时失败: {rb_err:#}"
            ));
        }
        return Err(err);
    }
    if let Err(err) = tx.commit() {
        if let Err(rb_err) = guard.rollback() {
            return Err(anyhow!(
                "提交失败: {err:#}；规则文件恢复同时失败: {rb_err:#}"
            ));
        }
        return Err(err.into());
    }
    guard.commit();

    Ok(json!({ "domain": "rules", "steps": steps, "skipped": [] }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn temp_root(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "native-rules-apply-test-{}-{}-{:x}",
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
            command: "rules:apply".to_string(),
            args: args.iter().map(|s| s.to_string()).collect(),
            run_id: "run-apply-test".to_string(),
        }
    }

    fn base_of(path: &Path) -> String {
        match std::fs::read(path) {
            Ok(bytes) => sha256_hex_bytes(&bytes),
            Err(_) => "missing".to_string(),
        }
    }

    #[test]
    fn late_audit_failure_restores_file_and_drops_backup() {
        let root = temp_root("rollback");
        let ctx = ctx_at(&root);
        let path = ctx.home_dir.join(".claude/CLAUDE.md");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "用户原有规则\n").unwrap();
        {
            let conn = store::open_write(&ctx).unwrap();
            conn.execute_batch(
                "CREATE TRIGGER fail_operation_logs BEFORE INSERT ON operation_logs
                 BEGIN SELECT RAISE(ABORT, 'injected-end-failure'); END;",
            )
            .unwrap();
        }
        let base = base_of(&path);
        let err = apply(
            &ctx,
            &req(&[
                "--only",
                "rules:claude_global",
                "--base",
                &base,
                "--recommended",
            ]),
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("injected-end-failure"));
        assert_eq!(std::fs::read(&path).unwrap(), "用户原有规则\n".as_bytes());
        let conn = store::open_readonly(&ctx).unwrap().unwrap();
        assert!(
            templates::load_from(&conn).unwrap().is_empty(),
            "失败时备份模板随事务回滚"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn copying_a_target_onto_itself_is_rejected() {
        let root = temp_root("self-copy");
        let ctx = ctx_at(&root);
        let err = apply(
            &ctx,
            &req(&[
                "--only",
                "rules:codex_global",
                "--base",
                "missing",
                "--from",
                "rules:codex_global",
            ]),
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("同一个"));
        assert!(!ctx.data_dir.exists());
        assert!(!ctx.home_dir.exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}
