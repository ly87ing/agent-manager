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
    /// 配置目录内的相对文件名：一个实例有多个受管文件，各自记各自的指纹。
    pub file: String,
    pub applied_digest: String,
    pub updated_at: String,
}

pub(crate) fn ensure_table(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS profile_bindings (
            profile_id TEXT NOT NULL,
            file TEXT NOT NULL,
            applied_digest TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            PRIMARY KEY (profile_id, file)
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
    if binding.file.is_empty() || binding.file.contains('/') || binding.file.contains("..") {
        bail!("profile_bindings 存在非法文件名");
    }
    if !is_sha256_hex(&binding.applied_digest) {
        bail!("applied_digest 必须是 64 位小写 sha256 hex");
    }
    if binding.updated_at.is_empty() {
        bail!("updated_at 不能为空");
    }
    Ok(())
}

/// 键为 (profile_id, file)：一个实例有多个受管文件，各自记各自的指纹。
pub(crate) fn load_bindings_from(conn: &Connection) -> Result<BTreeMap<(String, String), Binding>> {
    let mut out = BTreeMap::new();
    if !table_exists(conn)? {
        return Ok(out);
    }
    let mut stmt = conn.prepare(
        "SELECT profile_id, file, applied_digest, updated_at FROM profile_bindings ORDER BY profile_id, file",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(Binding {
            profile_id: row.get(0)?,
            file: row.get(1)?,
            applied_digest: row.get(2)?,
            updated_at: row.get(3)?,
        })
    })?;
    for row in rows {
        let binding = row?;
        validate(&binding).map_err(|e| anyhow!("profile_bindings 存在非法行: {e}"))?;
        out.insert((binding.profile_id.clone(), binding.file.clone()), binding);
    }
    Ok(out)
}

pub(crate) fn write_binding(tx: &Transaction, binding: &Binding) -> Result<()> {
    validate(binding)?;
    ensure_table(tx)?;
    tx.execute(
        "INSERT INTO profile_bindings (profile_id, file, applied_digest, updated_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (profile_id, file) DO UPDATE SET
            applied_digest = excluded.applied_digest,
            updated_at = excluded.updated_at",
        rusqlite::params![
            binding.profile_id,
            binding.file,
            binding.applied_digest,
            binding.updated_at
        ],
    )?;
    Ok(())
}
