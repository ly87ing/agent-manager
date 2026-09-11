//! MCP SQLite 自管绑定表读写

use anyhow::Result;
use rusqlite::Connection;
use serde_json::Value;
use std::collections::BTreeMap;

use super::model::{now_iso, validate_binding, Binding, Bindings, TABLE_DDL};
use crate::native::Context;
use crate::store;

fn row_to_binding(row: &rusqlite::Row) -> rusqlite::Result<Binding> {
    let definition_json: String = row.get("definition_json")?;
    let definition: Value = serde_json::from_str(&definition_json).map_err(|err| {
        rusqlite::Error::FromSqlConversionFailure(3, rusqlite::types::Type::Text, Box::new(err))
    })?;
    Ok(Binding {
        version: row.get("version")?,
        definition,
        enabled: row.get::<_, i64>("enabled")? != 0,
        assigned: row.get::<_, i64>("assigned")? != 0,
        applied_digest: row.get("applied_digest")?,
    })
}

pub fn load_bindings_from(conn: &Connection) -> Result<Bindings> {
    let mut stmt = match conn.prepare(
        "SELECT server, agent, version, definition_json, enabled, assigned, applied_digest FROM mcp_bindings",
    ) {
        Ok(stmt) => stmt,
        Err(exc) => {
            if exc.to_string().contains("no such table") {
                return Ok(Bindings::new());
            }
            return Err(exc.into());
        }
    };
    let mut out: Bindings = BTreeMap::new();
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>("server")?,
            row.get::<_, String>("agent")?,
            row_to_binding(row)?,
        ))
    });
    let rows = match rows {
        Ok(rows) => rows,
        Err(exc) => {
            if exc.to_string().contains("no such table") {
                return Ok(Bindings::new());
            }
            return Err(exc.into());
        }
    };
    for row in rows {
        let (server, agent, binding) = row?;
        validate_binding(&server, &agent, &binding)?;
        out.entry(server).or_default().insert(agent, binding);
    }
    Ok(out)
}

pub fn load_bindings_readonly(ctx: &Context) -> Result<Bindings> {
    let Some(conn) = store::open_readonly(ctx)? else {
        return Ok(Bindings::new());
    };
    load_bindings_from(&conn)
}

pub fn ensure_table(conn: &Connection) -> Result<()> {
    conn.execute_batch(TABLE_DDL)?;
    Ok(())
}

pub fn save_binding_in_tx(
    tx: &Connection,
    server: &str,
    agent: &str,
    binding: Option<&Binding>,
) -> Result<()> {
    match binding {
        Some(b) => {
            validate_binding(server, agent, b)?;
            tx.execute(
                "INSERT INTO mcp_bindings (server, agent, version, definition_json, enabled, assigned, applied_digest, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT(server, agent) DO UPDATE SET version = excluded.version,
                     definition_json = excluded.definition_json, enabled = excluded.enabled,
                     assigned = excluded.assigned, applied_digest = excluded.applied_digest,
                     updated_at = excluded.updated_at",
                rusqlite::params![
                    server,
                    agent,
                    b.version,
                    serde_json::to_string(&b.definition)?,
                    b.enabled as i64,
                    b.assigned as i64,
                    b.applied_digest,
                    now_iso()
                ],
            )?;
        }
        None => {
            tx.execute(
                "DELETE FROM mcp_bindings WHERE server = ?1 AND agent = ?2",
                rusqlite::params![server, agent],
            )?;
        }
    }
    Ok(())
}
