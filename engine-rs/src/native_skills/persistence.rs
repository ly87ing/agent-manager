use anyhow::Result;
use rusqlite::{params, Connection};
use std::collections::BTreeMap;

use super::BindingEntry;
use crate::native::Context;
use crate::store;

pub const SKILL_BINDINGS_DDL: &str = "CREATE TABLE IF NOT EXISTS skill_bindings (
    collection TEXT NOT NULL,
    skill TEXT NOT NULL,
    agent TEXT NOT NULL,
    version TEXT NOT NULL,
    enabled INTEGER NOT NULL,
    assigned INTEGER NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (collection, skill, agent)
);";

pub fn load_bindings_from_conn(
    conn: &Connection,
) -> Result<BTreeMap<(String, String, String), BindingEntry>> {
    let mut out = BTreeMap::new();
    let table_exists: bool = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='skill_bindings'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map(|c| c > 0)
        .unwrap_or(false);
    if !table_exists {
        return Ok(out);
    }
    let mut stmt = conn.prepare(
        "SELECT collection, skill, agent, version, enabled, assigned FROM skill_bindings",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            BindingEntry {
                version: row.get::<_, String>(3)?,
                enabled: row.get::<_, i64>(4)? != 0,
                assigned: row.get::<_, i64>(5)? != 0,
            },
        ))
    })?;
    for row in rows {
        let (coll, skill, agent, entry) = row?;
        out.insert((coll, skill, agent), entry);
    }
    Ok(out)
}

pub(crate) fn load_bindings_readonly(
    ctx: &Context,
) -> Result<BTreeMap<(String, String, String), BindingEntry>> {
    match store::open_readonly(ctx)? {
        Some(conn) => load_bindings_from_conn(&conn),
        None => Ok(BTreeMap::new()),
    }
}

pub fn write_binding(
    tx: &rusqlite::Transaction,
    coll: &str,
    skill: &str,
    agent: &str,
    version: &str,
    enabled: bool,
    assigned: bool,
) -> Result<()> {
    tx.execute(SKILL_BINDINGS_DDL, [])?;
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    tx.execute(
        "INSERT INTO skill_bindings (collection, skill, agent, version, enabled, assigned, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT (collection, skill, agent) DO UPDATE SET
             version = excluded.version,
             enabled = excluded.enabled,
             assigned = excluded.assigned,
             updated_at = excluded.updated_at;",
        params![coll, skill, agent, version, enabled as i64, assigned as i64, now],
    )?;
    Ok(())
}
