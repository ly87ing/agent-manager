//! profile_bindings 表：记录每个实例目标文件写入后的 sha256，用于检测外部修改。
//! 行校验失败即 fail-closed，错误不回显数据库读出的原值。

use anyhow::{anyhow, bail, Result};
use rusqlite::{Connection, Transaction};
use std::collections::BTreeMap;

use crate::native_mcp::model::is_sha256_hex;

const TABLE: &str = "profile_bindings";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Binding {
    pub profile_id: String,
    pub applied_digest: String,
    pub updated_at: String,
}

pub(crate) fn ensure_table(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS profile_bindings (
            profile_id TEXT PRIMARY KEY NOT NULL,
            applied_digest TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );",
    )?;
    Ok(())
}

fn table_exists(conn: &Connection) -> Result<bool> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
        [TABLE],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

fn validate(binding: &Binding) -> Result<()> {
    if !super::model::valid_profile_id(&binding.profile_id) {
        bail!("profile_bindings 存在非法 profile_id");
    }
    if !is_sha256_hex(&binding.applied_digest) {
        bail!("applied_digest 必须是 64 位小写 sha256 hex");
    }
    if binding.updated_at.is_empty() {
        bail!("updated_at 不能为空");
    }
    Ok(())
}

pub(crate) fn load_bindings_from(conn: &Connection) -> Result<BTreeMap<String, Binding>> {
    let mut out = BTreeMap::new();
    if !table_exists(conn)? {
        return Ok(out);
    }
    let mut stmt = conn.prepare(
        "SELECT profile_id, applied_digest, updated_at FROM profile_bindings ORDER BY profile_id",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(Binding {
            profile_id: row.get(0)?,
            applied_digest: row.get(1)?,
            updated_at: row.get(2)?,
        })
    })?;
    for row in rows {
        let binding = row?;
        validate(&binding).map_err(|e| anyhow!("profile_bindings 存在非法行: {e}"))?;
        out.insert(binding.profile_id.clone(), binding);
    }
    Ok(out)
}

pub(crate) fn write_binding(tx: &Transaction, binding: &Binding) -> Result<()> {
    validate(binding)?;
    ensure_table(tx)?;
    tx.execute(
        "INSERT INTO profile_bindings (profile_id, applied_digest, updated_at)
         VALUES (?1, ?2, ?3)
         ON CONFLICT (profile_id) DO UPDATE SET
            applied_digest = excluded.applied_digest,
            updated_at = excluded.updated_at",
        rusqlite::params![
            binding.profile_id,
            binding.applied_digest,
            binding.updated_at
        ],
    )?;
    Ok(())
}
