//! profile_bindings 表：记录每个实例目标文件上次写入后的基准与写过的路径（`env.<名>` /
//! `model`）。写入门槛是调用方传入的 `--base`，这里只是写入记录：定义里删掉的变量据此在下次
//! 写入时撤掉，从没由本工具写过的键不碰。
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
    /// 上次写入的路径（Claude：`env.<名>` 与 `model`；Codex 整份文件不记）。
    pub owned: Vec<String>,
}

pub(crate) fn ensure_table(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS profile_bindings (
            profile_id TEXT NOT NULL,
            file TEXT NOT NULL,
            applied_digest TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            owned_json TEXT NOT NULL DEFAULT '[]',
            PRIMARY KEY (profile_id, file)
        );",
    )?;
    // 旧库补列：ALTER TABLE 只能追加到末尾，新建库与补列库列顺序一致
    if !has_owned_column(conn)? {
        conn.execute_batch(
            "ALTER TABLE profile_bindings ADD COLUMN owned_json TEXT NOT NULL DEFAULT '[]'",
        )?;
    }
    Ok(())
}

fn has_owned_column(conn: &Connection) -> Result<bool> {
    let mut stmt = conn.prepare("PRAGMA table_info(profile_bindings)")?;
    let names = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(names.iter().any(|name| name == "owned_json"))
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
    // 只读路径不补列：旧库缺列时按空记录理解
    let sql = if has_owned_column(conn)? {
        "SELECT profile_id, file, applied_digest, updated_at, owned_json FROM profile_bindings ORDER BY profile_id, file"
    } else {
        "SELECT profile_id, file, applied_digest, updated_at, '[]' FROM profile_bindings ORDER BY profile_id, file"
    };
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
        ))
    })?;
    for row in rows {
        let (profile_id, file, applied_digest, updated_at, owned_raw) = row?;
        let owned = serde_json::from_str::<Vec<String>>(&owned_raw)
            .map_err(|_| anyhow!("profile_bindings 存在非法行: owned_json 不是字符串数组"))?;
        let binding = Binding {
            profile_id,
            file,
            applied_digest,
            updated_at,
            owned,
        };
        validate(&binding).map_err(|e| anyhow!("profile_bindings 存在非法行: {e}"))?;
        out.insert((binding.profile_id.clone(), binding.file.clone()), binding);
    }
    Ok(out)
}

pub(crate) fn write_binding(tx: &Transaction, binding: &Binding) -> Result<()> {
    validate(binding)?;
    ensure_table(tx)?;
    tx.execute(
        "INSERT INTO profile_bindings (profile_id, file, applied_digest, updated_at, owned_json)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT (profile_id, file) DO UPDATE SET
            applied_digest = excluded.applied_digest,
            updated_at = excluded.updated_at,
            owned_json = excluded.owned_json",
        rusqlite::params![
            binding.profile_id,
            binding.file,
            binding.applied_digest,
            binding.updated_at,
            serde_json::to_string(&binding.owned)?
        ],
    )?;
    Ok(())
}
