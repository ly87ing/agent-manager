//! aliases 域写入：按名编辑 ~/.zshrc 与 ~/.zprofile 受管区块里的一条别名。
//!
//! 操作是新增/改写（--set，可带 --rename-from）、删除（--remove）、添加一条内置推荐
//! （--recommended）或加入一条旧版本保存的别名（--legacy）。要写哪些文件由调用方给出的
//! 写入基准决定（--zshrc-base / --zprofile-base，取自 status 的 observed.base），每个文件
//! 独立比对：任一不一致即整体拒绝（「已在外部修改」），文件与库都不变。只替换或删除定义了
//! 相关名称的行，区块内其他行与区块外字节原样保留；区块空了连同两行 marker 一起移除。
//! 锁前预检零写；锁内 Immediate 事务重检、逐文件原子 CAS 写入、旧来源的写路径清理、单条
//! 审计；任一步失败回滚已写文件、事务整体回滚。错误与审计只含名称/摘要，不含取值。

use std::collections::HashSet;

use anyhow::{anyhow, bail, Result};
use rusqlite::TransactionBehavior;
use serde_json::{json, Map, Value};

use super::args::{self, ApplyArgs, Op};
use super::legacy::{self, Pruned};
use super::model::{self, Edit};
use super::status::{self, Probe, View};
use crate::native::{reject_secret_text, Context, Request};
use crate::native_mcp::rollback::{atomic_write_bytes_checked, RollbackGuard};
use crate::store;

/// 一个要写的文件。
struct Write {
    target_id: &'static str,
    /// 实际写入的文件：rc 文件是符号链接时为解析后的最终文件，链接本身不变。
    abs: std::path::PathBuf,
    old_bytes: Option<Vec<u8>>,
    new_bytes: Vec<u8>,
    before_base: String,
    after_base: String,
}

/// 解析好、可以写的一次操作。
struct Prepared {
    edit: Edit,
    writes: Vec<Write>,
    /// 选中但内容不变、不用写的文件。
    skipped: Vec<&'static str>,
    /// 写入前后在实际区块里的 (名称, 取值)，供旧来源的写路径清理。
    present: HashSet<(String, String)>,
}

fn validate_value(value: &str) -> Result<()> {
    if value.contains('\0') {
        bail!("快捷命令的取值不能包含 NUL 字符");
    }
    if !model::valid_alias_text(value) {
        bail!("快捷命令的取值不能包含单引号或换行");
    }
    if let Some(kind) = reject_secret_text(value) {
        bail!("拒绝写入疑似凭据（{kind}）：请改用 ${{VAR}} 引用，把取值放进外部密钥文件");
    }
    Ok(())
}

fn pairs(split: &model::Split) -> impl Iterator<Item = (String, String)> {
    split
        .entries()
        .into_iter()
        .filter_map(|e| e.value.map(|value| (e.name, value)))
}

/// 把操作落成一次对区块的编辑（推荐与旧版别名在这里按当前实际内容查找）。
fn edit_of(op: &Op, views: &[View], row: Option<&Map<String, Value>>) -> Result<Edit> {
    Ok(match op {
        Op::Set {
            name,
            value,
            rename_from,
        } => {
            validate_value(value)?;
            Edit::Set {
                name: name.clone(),
                value: value.clone(),
                rename_from: rename_from.clone().filter(|old| old != name),
            }
        }
        Op::Remove { name } => Edit::Remove { name: name.clone() },
        Op::Recommended(name) => {
            let (name, value) = status::recommendations(views)
                .into_iter()
                .find(|(n, _)| n == name)
                .ok_or_else(|| anyhow!("这条推荐已不适用（同名快捷命令已存在），请刷新后重试"))?;
            Edit::Set {
                name,
                value,
                rename_from: None,
            }
        }
        Op::Legacy(id) => {
            let item = status::legacy_suggestions(views, row)
                .into_iter()
                .find(|c| &c.id == id)
                .ok_or_else(|| {
                    anyhow!("这条已保存的快捷命令已不存在（可能已加入或已丢弃），请刷新后重试")
                })?;
            Edit::Set {
                name: item.name,
                value: item.value,
                rename_from: None,
            }
        }
    })
}

/// 基于一次读取的实际文件与旧来源，确定写入内容。锁前预检与锁内执行都调用它，
/// 锁内结果才是最终依据。
fn prepare(
    ctx: &Context,
    row: Option<&Map<String, Value>>,
    parsed: &ApplyArgs,
) -> Result<Prepared> {
    let views = status::observe(ctx);
    let mut present: HashSet<(String, String)> = HashSet::new();
    for view in &views {
        if let Some(split) = view.probe.split() {
            present.extend(pairs(&split));
        }
    }
    // 先逐文件比对基准，再算编辑：外部改过时不给出任何「推荐已不适用」之类的次要错误。
    let mut selected = Vec::new();
    for (spec, base) in &parsed.bases {
        let view = views
            .iter()
            .find(|v| v.spec.target_id == spec.target_id)
            .expect("固定目标");
        let (old_bytes, split) = match &view.probe {
            Probe::Blocked { reason, .. } => bail!("~/{} 当前不能读写：{reason}", spec.target),
            Probe::Missing => (None, view.probe.split().expect("文件不存在时可拆")),
            Probe::Present { bytes, split } => (Some(bytes.clone()), split.clone()),
        };
        if &split.base() != base {
            bail!(
                "~/{} 已在外部修改（与打开时读到的内容不同），没有写入；请刷新后重新操作",
                spec.target
            );
        }
        selected.push((view, old_bytes, split));
    }
    let edit = edit_of(&parsed.op, &views, row)?;

    let mut writes = Vec::new();
    let mut skipped = Vec::new();
    for (view, old_bytes, split) in selected {
        let lines = split.block.clone().unwrap_or_default();
        let new_lines = model::edit_lines(&lines, &edit);
        let inner = model::block_inner(&new_lines);
        let Some(new_bytes) =
            model::converge_content(old_bytes.as_deref().unwrap_or_default(), &inner)?
        else {
            skipped.push(view.spec.target_id);
            continue;
        };
        let after = model::split_block(std::str::from_utf8(&new_bytes)?)?;
        present.extend(pairs(&after));
        writes.push(Write {
            target_id: view.spec.target_id,
            abs: view.real.clone(),
            old_bytes,
            new_bytes,
            before_base: split.base(),
            after_base: after.base(),
        });
    }
    Ok(Prepared {
        edit,
        writes,
        skipped,
        present,
    })
}

/// 锁前预检：严格解析 + 只读校验（目标可写、各文件基准一致、推荐/旧版别名仍在、取值合法）。
/// 无效/阻塞预检不创建 data/home/DB/表/文件。
pub(crate) fn preflight_apply(ctx: &Context, req: &Request) -> Result<()> {
    let parsed = args::parse_apply_args(req)?;
    let row = legacy::read_row_readonly(ctx)?;
    prepare(ctx, row.as_ref(), &parsed)?;
    Ok(())
}

fn op_meta(op: &Op, edit: &Edit) -> Value {
    let name = match edit {
        Edit::Set { name, .. } | Edit::Remove { name } => name,
    };
    match op {
        Op::Set { .. } => json!({
            "kind": "set",
            "name": name,
            "renamed_from": match edit { Edit::Set { rename_from, .. } => json!(rename_from), _ => Value::Null },
        }),
        Op::Remove { .. } => json!({ "kind": "remove", "name": name }),
        Op::Recommended(_) => json!({ "kind": "recommended", "name": name }),
        Op::Legacy(id) => json!({ "kind": "legacy", "suggestion": id, "name": name }),
    }
}

pub(crate) fn apply(ctx: &Context, req: &Request) -> Result<Value> {
    // 自身再次预检：直接调用 apply 不能绕过校验。
    preflight_apply(ctx, req)?;
    let parsed = args::parse_apply_args(req)?;

    let mut conn = store::open_write(ctx)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    // 锁内基于同一 snapshot 重新读取并重检，不沿用锁前的结果。
    let row = legacy::read_row(&tx);
    let prep = prepare(ctx, row.as_ref(), &parsed)?;

    if prep.writes.is_empty() {
        tx.commit()?;
        return Ok(json!({ "domain": "aliases", "steps": [], "skipped": prep.skipped }));
    }

    let mut guard = RollbackGuard::new();
    let run = (|| -> Result<Vec<Value>> {
        let mut steps = Vec::new();
        for write in &prep.writes {
            // CAS 比对的是读到时的文件字节：锁内重读之后若又被改过，这里拒绝替换。
            atomic_write_bytes_checked(&write.abs, &write.new_bytes, write.old_bytes.as_deref())?;
            guard.record_written(&write.abs, write.old_bytes.clone(), write.new_bytes.clone());
            steps.push(json!({
                "name": format!("aliases:write {}", write.target_id),
                "ok": true,
                "exit_code": 0,
            }));
        }
        // 写路径清理：写入前后已在实际区块里的旧版别名从库里删掉。
        let pruned = legacy::prune_in_tx(&tx, row.as_ref(), &|c| {
            prep.present.contains(&(c.name.clone(), c.value.clone()))
        })?;
        let ids: Vec<&str> = prep.writes.iter().map(|w| w.target_id).collect();
        let bases: Map<String, Value> = prep
            .writes
            .iter()
            .map(|w| {
                (
                    w.target_id.to_string(),
                    json!({ "before": w.before_base, "after": w.after_base }),
                )
            })
            .collect();
        store::record_operation(
            &tx,
            Some(&req.run_id),
            "aliases:apply",
            Some(if ids.len() == 1 { ids[0] } else { "aliases" }),
            Some(&json!({
                "action": "write",
                "source": op_meta(&parsed.op, &prep.edit),
                "targets": ids,
                "bases": bases,
                "legacy_pruned": pruned != Pruned::Unchanged,
            })),
        )?;
        Ok(steps)
    })();

    let steps = match run {
        Ok(steps) => steps,
        Err(err) => {
            drop(tx);
            if let Err(rb_err) = guard.rollback() {
                return Err(anyhow!(
                    "执行失败: {err:#}；终端配置文件恢复同时失败: {rb_err:#}"
                ));
            }
            return Err(err);
        }
    };
    if let Err(err) = tx.commit() {
        if let Err(rb_err) = guard.rollback() {
            return Err(anyhow!(
                "提交失败: {err:#}；终端配置文件恢复同时失败: {rb_err:#}"
            ));
        }
        return Err(err.into());
    }
    guard.commit();

    Ok(json!({ "domain": "aliases", "steps": steps, "skipped": prep.skipped }))
}
