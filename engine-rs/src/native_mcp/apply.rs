//! mcp:apply：锁内重读文件、迁移旧库、按计划写入（CAS + 回读 + 条件回滚 + 审计）。

use anyhow::{anyhow, Result};
use rusqlite::TransactionBehavior;
use serde_json::{json, Value};

use super::inventory::{self, Inventory};
use super::preflight::{parse_action_args, plan};
use super::rollback::{atomic_write_bytes_checked, RollbackGuard};
use super::saved;
use crate::native::{Context, Request};
use crate::store;

/// 返回 (data, warnings)。warnings 只含变量名、字段名等提示，绝不含取值。
pub fn apply(ctx: &Context, req: &Request) -> Result<(Value, Vec<String>)> {
    let parsed = parse_action_args(req)?;
    let only = format!("mcp:{}", parsed.server);

    let mut conn = store::open_write(ctx)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

    // 锁内重读：文件内容与库都以此刻为准。旧库在同一事务里先迁移（只写库）。
    let policy = store::policy_from_connection(&tx)?;
    let scans = inventory::scan_targets(&policy, &ctx.home_dir);
    super::legacy::migrate(&tx, &scans, Some(&req.run_id))?;
    let inv = Inventory {
        saved: saved::load_from(&tx)?,
        scans,
    };
    let plan = plan(&parsed, &inv)?;

    let mut guard = RollbackGuard::new();
    let mut warnings = plan.warnings.clone();
    let result: Result<()> = (|| {
        if !plan.adoptions.is_empty() {
            crate::secret_values::merge_without_overwrite(ctx, &plan.adoptions)?;
            crate::secret_values::refresh(ctx);
        }
        for write in &plan.file_writes {
            atomic_write_bytes_checked(&write.path, &write.new_bytes, write.old_bytes.as_deref())?;
            guard.record_written(
                &write.path,
                write.old_bytes.clone(),
                write.new_bytes.clone(),
            );
        }
        if !plan.saved_upserts.is_empty() || !plan.saved_deletes.is_empty() {
            saved::ensure_table(&tx)?;
        }
        for (server, agent) in &plan.saved_deletes {
            saved::delete(&tx, server, agent)?;
        }
        for row in &plan.saved_upserts {
            saved::upsert(&tx, row)?;
        }
        // 审计只记动作与范围：不含 spec、摘要、命令参数、地址或任何取值。
        store::record_operation(
            &tx,
            Some(&req.run_id),
            &format!("mcp:{}", parsed.action.id()),
            Some(&only),
            Some(&json!({
                "action": parsed.action.id(),
                "agent": parsed.agent,
                "to": parsed.to,
                "ok": true,
            })),
        )?;
        Ok(())
    })();

    let result = result.and_then(|()| tx.commit().map_err(anyhow::Error::from));
    if let Err(err) = result {
        if let Err(rb_err) = guard.rollback() {
            return Err(anyhow!("执行失败: {err:#}；配置回滚同时失败: {rb_err:#}"));
        }
        return Err(err);
    }
    guard.commit();
    if plan.file_writes.is_empty() && plan.saved_upserts.is_empty() && plan.saved_deletes.is_empty()
    {
        warnings.push("内容没有变化，未改动配置文件".to_string());
    }

    Ok((
        json!({
            "domain": "mcp",
            "steps": [{
                "name": format!("mcp:{} {}", parsed.action.id(), only),
                "ok": true,
                "exit_code": 0
            }],
            "skipped": []
        }),
        warnings,
    ))
}
