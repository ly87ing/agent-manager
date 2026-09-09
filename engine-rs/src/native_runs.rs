use rusqlite::TransactionBehavior;
use serde_json::{json, Value};

use crate::native::{Context, DispatchError, DispatchResult, Request};
use crate::store;

pub(crate) fn runs_list(ctx: &Context) -> DispatchResult {
    let mut runs: Vec<Value> = Vec::new();
    if let Some(conn) = store::open_readonly(ctx).map_err(DispatchError::from)? {
        let mut stmt = conn.prepare(
            "SELECT run_id, COUNT(*) AS ops, MAX(created_at) AS last_at
             FROM operation_logs WHERE run_id IS NOT NULL
             GROUP BY run_id ORDER BY last_at DESC",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(|err| DispatchError::new("INTERNAL", err.to_string()))?;
        for row in rows {
            let (run_id, ops, last_at) = row?;
            runs.push(json!({ "run_id": run_id, "operations": ops, "last_at": last_at }));
        }
    }
    Ok((json!({ "runs": runs }), vec![]))
}

pub(crate) fn runs_get(ctx: &Context, request: &Request) -> DispatchResult {
    let run_id = request
        .value("--run")
        .ok_or_else(|| DispatchError::new("USAGE_ERROR", "runs:get 需要 --run <run_id>"))?;
    let mut operations: Vec<Value> = Vec::new();
    if let Some(conn) = store::open_readonly(ctx).map_err(DispatchError::from)? {
        let mut stmt = conn.prepare(
            "SELECT id, operation_type, target, detail_json, created_at
             FROM operation_logs WHERE run_id = ?1 ORDER BY created_at ASC",
        )?;
        let rows = stmt
            .query_map([run_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })
            .map_err(|err| DispatchError::new("INTERNAL", err.to_string()))?;
        for row in rows {
            let (id, operation_type, target, detail_json, created_at) = row?;
            operations.push(json!({
                "id": id,
                "operation_type": operation_type,
                "target": target,
                "detail": detail_json
                    .as_deref()
                    .and_then(|raw| serde_json::from_str::<Value>(raw).ok()),
                "created_at": created_at,
            }));
        }
    }
    if operations.is_empty() {
        return Err(DispatchError::new(
            "RUN_NOT_FOUND",
            format!("未找到运行记录: {run_id}"),
        ));
    }
    Ok((
        json!({ "run_id": run_id, "operations": operations }),
        vec![],
    ))
}

pub(crate) fn runs_clear(ctx: &Context) -> DispatchResult {
    let Some(_) = store::open_readonly(ctx).map_err(DispatchError::from)? else {
        return Ok((json!({ "domain": "runs", "deleted": 0 }), vec![]));
    };
    let mut conn = store::open_write(ctx).map_err(DispatchError::from)?;
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(DispatchError::from)?;
    let deleted = store::clear_operation_logs(&tx).map_err(DispatchError::from)?;
    tx.commit().map_err(DispatchError::from)?;
    Ok((json!({ "domain": "runs", "deleted": deleted }), vec![]))
}
