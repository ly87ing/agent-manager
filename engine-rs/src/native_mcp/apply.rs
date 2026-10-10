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
    let account = parsed.to.as_deref().or(parsed.agent.as_deref());
    let _account_lock = account
        .map(|account| crate::native_secrets::lock_credential_account(ctx, account, true))
        .transpose()?;

    let mut conn = store::open_write(ctx)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

    // 锁内重读：文件内容与库都以此刻为准。旧库在同一事务里先迁移（只写库）。
    let policy = store::policy_from_connection(&tx)?;
    let scans = inventory::scan_targets_checked(ctx, &policy);
    super::legacy::migrate(&tx, &scans, Some(&req.run_id))?;
    let inv = Inventory {
        saved: saved::load_from(&tx)?,
        scans,
    };
    let plan = plan(ctx, &parsed, &inv)?;

    let mut guard = RollbackGuard::new();
    let mut warnings = plan.warnings.clone();
    let result: Result<()> = (|| {
        if let Some(account) = parsed.to.as_deref().or(parsed.agent.as_deref()) {
            crate::native_secrets::claim_variable_ownership(
                ctx,
                &tx,
                account,
                &plan.variable_names,
            )?;
        }
        for write in &plan.file_writes {
            let target = inv
                .scans
                .iter()
                .find(|scan| scan.target.path == write.path)
                .ok_or_else(|| anyhow!("MCP 写入目标不在当前账号清单，拒绝写入"))?;
            crate::native_profiles::paths::validate_account_resource_path(
                ctx,
                &policy,
                &target.target.agent,
                &write.path,
            )?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn running_main_claude_session_blocks_mcp_identity_file_changes() {
        let root =
            std::env::temp_dir().join(format!("am-mcp-session-lock-{}", crate::store::uuid_v4()));
        let ctx = Context {
            home_dir: root.join("home"),
            data_dir: root.join("data"),
        };
        std::fs::create_dir_all(ctx.home_dir.join(".claude")).unwrap();
        let path = ctx.home_dir.join(".claude.json");
        let original = b"{\"mcpServers\":{}}";
        std::fs::write(&path, original).unwrap();
        let session = crate::native_account_context::lock_directory(
            &ctx,
            &ctx.home_dir.join(".claude"),
            false,
        )
        .unwrap();
        let req = Request {
            command: "mcp:apply".into(),
            run_id: "session-lock".into(),
            args: [
                "--only",
                "mcp:new",
                "--agent",
                "claude-code",
                "--action",
                "add",
                "--spec",
                "{\"command\":\"mcp\"}",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
        };
        assert!(apply(&ctx, &req)
            .unwrap_err()
            .to_string()
            .contains("正在运行"));
        assert_eq!(std::fs::read(&path).unwrap(), original);
        drop(session);
        assert!(apply(&ctx, &req).is_ok());
        std::fs::remove_dir_all(root).unwrap();
    }
}
