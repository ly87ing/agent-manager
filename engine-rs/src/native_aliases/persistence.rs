//! alias_target_bindings 表：严格 schema 与行校验，错误不回显数据库读出的
//! 原始值（对齐 hook_target_bindings 的脱敏约束）。
//!
//! 外部修改检测只看 marker 区块：`applied_block_digest` 是写入后区块正文的
//! sha256。区块外的字节本来就逐字节保留，用户在自己的 ~/.zshrc 里加一行不该
//! 让同步停摆。`applied_digest`（整文件 sha256）保留作为写入记录，并在
//! `applied_block_digest` 为空（本改动之前写下的旧行）时充当回退比较对象。

use anyhow::{anyhow, bail, Result};
use rusqlite::{Connection, Transaction};

use super::model::targets;
use crate::native_mcp::model::is_sha256_hex;

const TABLE: &str = "alias_target_bindings";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Binding {
    pub target_id: String,
    pub applied_digest: String,
    /// 区块正文 sha256；空串表示本改动之前写下的旧行，比较时回退到整文件。
    pub applied_block_digest: String,
    pub source_revision: String,
    pub updated_at: String,
}

pub(crate) fn ensure_table(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS alias_target_bindings (
            target_id TEXT PRIMARY KEY NOT NULL,
            applied_digest TEXT NOT NULL,
            source_revision TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            -- 末列：旧库补列时 ALTER TABLE 只能追加到末尾，新建库与补列库的
            -- 列顺序必须一致，否则严格 schema 校验会把补过列的库判成损坏。
            applied_block_digest TEXT NOT NULL DEFAULT ''
        );",
    )?;
    migrate_add_block_digest(conn)?;
    Ok(())
}

/// 旧库补列：本改动之前建的表没有 applied_block_digest。补出来的行取空串，
/// 读取时按旧语义回退比较，下一次成功写入即升级为区块摘要。
fn migrate_add_block_digest(conn: &Connection) -> Result<()> {
    if has_block_digest_column(conn)? {
        return Ok(());
    }
    conn.execute_batch(
        "ALTER TABLE alias_target_bindings
         ADD COLUMN applied_block_digest TEXT NOT NULL DEFAULT ''",
    )?;
    Ok(())
}

fn has_block_digest_column(conn: &Connection) -> Result<bool> {
    let mut stmt = conn.prepare("PRAGMA table_info(alias_target_bindings)")?;
    let found = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .iter()
        .any(|name| name == "applied_block_digest");
    Ok(found)
}

fn table_exists(conn: &Connection) -> Result<bool> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
        [TABLE],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

fn check_schema(conn: &Connection, has_block: bool) -> Result<()> {
    let mut stmt = conn.prepare("PRAGMA table_info(alias_target_bindings)")?;
    let cols: Vec<(String, String, bool, i64)> = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)? != 0,
                row.get::<_, i64>(5)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let full: [(&str, i64); 5] = [
        ("target_id", 1),
        ("applied_digest", 0),
        ("source_revision", 0),
        ("updated_at", 0),
        ("applied_block_digest", 0),
    ];
    // 旧表少最后一列；两种形状都算合法，其余仍严格校验。
    let expected: &[(&str, i64)] = if has_block { &full } else { &full[..4] };
    if cols.len() != expected.len() {
        bail!(
            "alias_target_bindings 表结构损坏：期望 {} 列，实际 {} 列",
            expected.len(),
            cols.len()
        );
    }
    for (idx, ((name, decl, notnull, pk), (_, want_pk))) in
        cols.iter().zip(expected.iter()).enumerate()
    {
        // 错误消息只含受信任的字段类别/期望，不回显数据库读出的原始列名/类型。
        let known_column = expected.iter().any(|(want_name, _)| name == want_name);
        if !known_column {
            bail!("alias_target_bindings 表结构损坏：存在未知列或列顺序不符");
        }
        if *name != expected[idx].0 {
            bail!("alias_target_bindings 表结构损坏：列顺序/名称不符");
        }
        if !decl.eq_ignore_ascii_case("TEXT") {
            bail!("alias_target_bindings 表结构损坏：存在列类型不是 TEXT");
        }
        if !notnull {
            bail!("alias_target_bindings 表结构损坏：存在列缺少 NOT NULL");
        }
        if pk != want_pk {
            bail!("alias_target_bindings 表结构损坏：主键定义不符");
        }
    }
    Ok(())
}

fn validate_binding(binding: &Binding) -> Result<()> {
    let known: Vec<&str> = targets().iter().map(|t| t.target_id).collect();
    if !known.contains(&binding.target_id.as_str()) {
        // 不回显数据库读出的 target_id 原值，避免被塞入的正文进入诊断。
        bail!("未知 aliases 目标 target_id（非受管目标）");
    }
    for (name, value) in [
        ("applied_digest", &binding.applied_digest),
        ("source_revision", &binding.source_revision),
    ] {
        if !is_sha256_hex(value) {
            bail!("{name} 必须是 64 位小写 sha256 hex");
        }
    }
    // 空串 = 旧行（补列时的默认值），读取侧据此回退到整文件比较。
    if !binding.applied_block_digest.is_empty() && !is_sha256_hex(&binding.applied_block_digest) {
        bail!("applied_block_digest 必须是 64 位小写 sha256 hex 或空串");
    }
    if binding.updated_at.is_empty() {
        bail!("updated_at 不能为空");
    }
    Ok(())
}

pub(crate) fn load_bindings_from(
    conn: &Connection,
) -> Result<std::collections::BTreeMap<String, Binding>> {
    let mut out = std::collections::BTreeMap::new();
    if !table_exists(conn)? {
        return Ok(out);
    }
    // 只读路径不得写库：旧表（没有 applied_block_digest）在这里按空串读出，
    // 比较时回退到整文件语义，补列留给写路径的 ensure_table。
    let has_block = has_block_digest_column(conn)?;
    check_schema(conn, has_block)?;
    let sql = if has_block {
        "SELECT target_id, applied_digest, source_revision, updated_at, applied_block_digest FROM alias_target_bindings ORDER BY target_id"
    } else {
        "SELECT target_id, applied_digest, source_revision, updated_at, '' FROM alias_target_bindings ORDER BY target_id"
    };
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map([], |row| {
        Ok(Binding {
            target_id: row.get(0)?,
            applied_digest: row.get(1)?,
            source_revision: row.get(2)?,
            updated_at: row.get(3)?,
            applied_block_digest: row.get(4)?,
        })
    })?;
    for row in rows {
        let binding = row?;
        validate_binding(&binding).map_err(|e| anyhow!("alias_target_bindings 存在非法行: {e}"))?;
        out.insert(binding.target_id.clone(), binding);
    }
    Ok(out)
}

pub(crate) fn write_binding(tx: &Transaction, binding: &Binding) -> Result<()> {
    validate_binding(binding)?;
    tx.execute(
        "INSERT INTO alias_target_bindings (target_id, applied_digest, source_revision, updated_at, applied_block_digest)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT (target_id) DO UPDATE SET
            applied_digest = excluded.applied_digest,
            applied_block_digest = excluded.applied_block_digest,
            source_revision = excluded.source_revision,
            updated_at = excluded.updated_at",
        rusqlite::params![
            binding.target_id,
            binding.applied_digest,
            binding.source_revision,
            binding.updated_at,
            binding.applied_block_digest,
        ],
    )?;
    Ok(())
}
