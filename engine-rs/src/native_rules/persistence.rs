use std::collections::BTreeMap;

use anyhow::{anyhow, bail, Result};
use rusqlite::{Connection, Transaction};

use super::model::{targets, Binding};
use crate::native_mcp::model::is_sha256_hex;

const TABLE: &str = "rule_target_bindings";

pub(crate) fn ensure_table(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS rule_target_bindings (
            target_id TEXT PRIMARY KEY NOT NULL,
            applied_digest TEXT NOT NULL,
            shared_revision TEXT NOT NULL,
            adapter_revision TEXT NOT NULL,
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

fn check_schema(conn: &Connection) -> Result<()> {
    let mut stmt = conn.prepare("PRAGMA table_info(rule_target_bindings)")?;
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
    let expected: [(&str, i64); 5] = [
        ("target_id", 1),
        ("applied_digest", 0),
        ("shared_revision", 0),
        ("adapter_revision", 0),
        ("updated_at", 0),
    ];
    if cols.len() != expected.len() {
        bail!(
            "rule_target_bindings 表结构损坏：期望 5 列，实际 {} 列",
            cols.len()
        );
    }
    for (idx, ((name, decl, notnull, pk), (want_name, want_pk))) in
        cols.iter().zip(expected.iter()).enumerate()
    {
        if name != want_name {
            bail!(
                "rule_target_bindings 表结构损坏：第 {} 列名称/顺序不符",
                idx + 1
            );
        }
        if !decl.eq_ignore_ascii_case("TEXT") {
            bail!("rule_target_bindings 表结构损坏：存在非 TEXT 声明类型的列");
        }
        if !notnull {
            bail!("rule_target_bindings 表结构损坏：存在缺少 NOT NULL 的列");
        }
        if pk != want_pk {
            bail!("rule_target_bindings 表结构损坏：主键定义不符");
        }
    }
    Ok(())
}

fn validate_binding(binding: &Binding) -> Result<()> {
    let known: Vec<&str> = targets().iter().map(|t| t.target_id).collect();
    if !known.contains(&binding.target_id.as_str()) {
        bail!("未知规则目标 target_id");
    }
    for (name, value) in [
        ("applied_digest", &binding.applied_digest),
        ("shared_revision", &binding.shared_revision),
        ("adapter_revision", &binding.adapter_revision),
    ] {
        if !is_sha256_hex(value) {
            bail!("{name} 必须是 64 位小写 sha256 hex");
        }
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
    check_schema(conn)?;
    let mut stmt = conn.prepare(
        "SELECT target_id, applied_digest, shared_revision, adapter_revision, updated_at FROM rule_target_bindings ORDER BY target_id",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(Binding {
            target_id: row.get(0)?,
            applied_digest: row.get(1)?,
            shared_revision: row.get(2)?,
            adapter_revision: row.get(3)?,
            updated_at: row.get(4)?,
        })
    })?;
    for row in rows {
        let binding = row?;
        validate_binding(&binding).map_err(|e| anyhow!("rule_target_bindings 存在非法行: {e}"))?;
        out.insert(binding.target_id.clone(), binding);
    }
    Ok(out)
}

pub(crate) fn write_binding(tx: &Transaction, binding: &Binding) -> Result<()> {
    validate_binding(binding)?;
    tx.execute(
        "INSERT INTO rule_target_bindings (target_id, applied_digest, shared_revision, adapter_revision, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT (target_id) DO UPDATE SET
            applied_digest = excluded.applied_digest,
            shared_revision = excluded.shared_revision,
            adapter_revision = excluded.adapter_revision,
            updated_at = excluded.updated_at",
        rusqlite::params![
            binding.target_id,
            binding.applied_digest,
            binding.shared_revision,
            binding.adapter_revision,
            binding.updated_at,
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_mcp::model::sha256_hex;

    fn digest(tag: &str) -> String {
        sha256_hex(tag)
    }

    fn binding(target_id: &str, tag: &str) -> Binding {
        Binding {
            target_id: target_id.to_string(),
            applied_digest: digest(&format!("applied-{tag}")),
            shared_revision: digest(&format!("shared-{tag}")),
            adapter_revision: digest(&format!("adapter-{tag}")),
            updated_at: format!("2026-09-12T00:00:0{tag}Z"),
        }
    }

    #[test]
    fn load_returns_empty_and_creates_nothing_when_table_missing() {
        let conn = Connection::open_in_memory().unwrap();
        let loaded = load_bindings_from(&conn).unwrap();
        assert!(loaded.is_empty());
        assert!(!table_exists(&conn).unwrap());
    }

    #[test]
    fn write_two_then_read_back_stably() {
        let mut conn = Connection::open_in_memory().unwrap();
        ensure_table(&conn).unwrap();
        let b1 = binding("rules:codex_global", "1");
        let b2 = binding("rules:claude_global", "2");
        let tx = conn.transaction().unwrap();
        write_binding(&tx, &b2).unwrap();
        write_binding(&tx, &b1).unwrap();
        tx.commit().unwrap();
        let loaded = load_bindings_from(&conn).unwrap();
        let keys: Vec<&String> = loaded.keys().collect();
        assert_eq!(keys, ["rules:claude_global", "rules:codex_global"]);
        assert_eq!(loaded.get("rules:codex_global"), Some(&b1));
        assert_eq!(loaded.get("rules:claude_global"), Some(&b2));
    }

    #[test]
    fn upsert_updates_existing_row() {
        let mut conn = Connection::open_in_memory().unwrap();
        ensure_table(&conn).unwrap();
        let tx = conn.transaction().unwrap();
        write_binding(&tx, &binding("rules:codex_global", "1")).unwrap();
        tx.commit().unwrap();
        let tx = conn.transaction().unwrap();
        write_binding(&tx, &binding("rules:codex_global", "3")).unwrap();
        tx.commit().unwrap();
        let loaded = load_bindings_from(&conn).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(
            loaded.get("rules:codex_global"),
            Some(&binding("rules:codex_global", "3"))
        );
    }

    #[test]
    fn write_rejects_unknown_target_bad_digest_and_empty_time() {
        let mut conn = Connection::open_in_memory().unwrap();
        ensure_table(&conn).unwrap();
        let tx = conn.transaction().unwrap();

        let mut b = binding("rules:codex_global", "1");
        b.target_id = "rules:unknown".to_string();
        assert!(write_binding(&tx, &b).is_err());

        let mut b = binding("rules:codex_global", "1");
        b.applied_digest = "not-a-digest".to_string();
        assert!(write_binding(&tx, &b).is_err());

        let mut b = binding("rules:codex_global", "1");
        b.applied_digest = "A".repeat(64);
        assert!(write_binding(&tx, &b).is_err());

        let mut b = binding("rules:codex_global", "1");
        b.updated_at = String::new();
        assert!(write_binding(&tx, &b).is_err());
    }

    #[test]
    fn load_rejects_bad_rows() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_table(&conn).unwrap();
        conn.execute(
            "INSERT INTO rule_target_bindings VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![
                "rules:unknown",
                digest("a"),
                digest("b"),
                digest("c"),
                "2026-01-01"
            ],
        )
        .unwrap();
        assert!(load_bindings_from(&conn).is_err());

        let conn2 = Connection::open_in_memory().unwrap();
        ensure_table(&conn2).unwrap();
        conn2
            .execute(
                "INSERT INTO rule_target_bindings VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![
                    "rules:codex_global",
                    "junk",
                    digest("b"),
                    digest("c"),
                    "2026-01-01"
                ],
            )
            .unwrap();
        assert!(load_bindings_from(&conn2).is_err());
    }

    #[test]
    fn load_rejects_broken_schema() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE rule_target_bindings (target_id TEXT PRIMARY KEY NOT NULL, note TEXT NOT NULL);",
        )
        .unwrap();
        assert!(load_bindings_from(&conn).is_err());
    }

    #[test]
    fn load_rejects_extra_column() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE rule_target_bindings (
                target_id TEXT PRIMARY KEY NOT NULL,
                applied_digest TEXT NOT NULL,
                shared_revision TEXT NOT NULL,
                adapter_revision TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                extra TEXT NOT NULL
            );",
        )
        .unwrap();
        assert!(load_bindings_from(&conn).is_err());
    }

    #[test]
    fn load_rejects_nullable_or_wrong_type_column() {
        let nullable = Connection::open_in_memory().unwrap();
        nullable
            .execute_batch(
                "CREATE TABLE rule_target_bindings (
                    target_id TEXT PRIMARY KEY NOT NULL,
                    applied_digest TEXT NOT NULL,
                    shared_revision TEXT NOT NULL,
                    adapter_revision TEXT NOT NULL,
                    updated_at TEXT
                );",
            )
            .unwrap();
        assert!(load_bindings_from(&nullable).is_err());

        let wrong_type = Connection::open_in_memory().unwrap();
        wrong_type
            .execute_batch(
                "CREATE TABLE rule_target_bindings (
                    target_id TEXT PRIMARY KEY NOT NULL,
                    applied_digest BLOB NOT NULL,
                    shared_revision TEXT NOT NULL,
                    adapter_revision TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );",
            )
            .unwrap();
        assert!(load_bindings_from(&wrong_type).is_err());
    }

    #[test]
    fn load_rejects_wrong_pk_and_wrong_order() {
        let no_pk = Connection::open_in_memory().unwrap();
        no_pk
            .execute_batch(
                "CREATE TABLE rule_target_bindings (
                target_id TEXT NOT NULL,
                applied_digest TEXT NOT NULL,
                shared_revision TEXT NOT NULL,
                adapter_revision TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );",
            )
            .unwrap();
        assert!(load_bindings_from(&no_pk).is_err());

        let wrong_order = Connection::open_in_memory().unwrap();
        wrong_order
            .execute_batch(
                "CREATE TABLE rule_target_bindings (
                    applied_digest TEXT NOT NULL,
                    target_id TEXT PRIMARY KEY NOT NULL,
                    shared_revision TEXT NOT NULL,
                    adapter_revision TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );",
            )
            .unwrap();
        assert!(load_bindings_from(&wrong_order).is_err());
    }

    #[test]
    fn load_error_never_echoes_row_marker_values() {
        let digest = sha256_hex("ok");
        // target_id 塞入 marker
        let conn = Connection::open_in_memory().unwrap();
        ensure_table(&conn).unwrap();
        conn.execute(
            "INSERT INTO rule_target_bindings VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params!["TARGET-MARKER-7e2b9c", digest, digest, digest, "2026-01-01"],
        )
        .unwrap();
        let err = load_bindings_from(&conn).unwrap_err();
        assert!(!format!("{err:#}").contains("TARGET-MARKER-7e2b9c"));

        // applied_digest 塞入 marker
        let conn = Connection::open_in_memory().unwrap();
        ensure_table(&conn).unwrap();
        conn.execute(
            "INSERT INTO rule_target_bindings VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![
                "rules:codex_global",
                "DIGEST-MARKER-4f8a1d",
                digest,
                digest,
                "2026-01-01"
            ],
        )
        .unwrap();
        let err = load_bindings_from(&conn).unwrap_err();
        assert!(!format!("{err:#}").contains("DIGEST-MARKER-4f8a1d"));

        // 非法 updated_at（空串）也走固定消息
        let conn = Connection::open_in_memory().unwrap();
        ensure_table(&conn).unwrap();
        conn.execute(
            "INSERT INTO rule_target_bindings VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params!["rules:codex_global", digest, digest, digest, ""],
        )
        .unwrap();
        let err = load_bindings_from(&conn).unwrap_err();
        assert!(format!("{err:#}").contains("非法行"));
    }

    #[test]
    fn schema_error_never_echoes_column_name_marker() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE rule_target_bindings (
                target_id TEXT PRIMARY KEY NOT NULL,
                applied_digest TEXT NOT NULL,
                shared_revision TEXT NOT NULL,
                adapter_revision TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                \"COL-MARKER-3b6e0f\" TEXT NOT NULL
            );",
        )
        .unwrap();
        let err = load_bindings_from(&conn).unwrap_err();
        assert!(!format!("{err:#}").contains("COL-MARKER-3b6e0f"));

        // 动态列名替换正确位置列（名称错误，列数仍为 5）
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE rule_target_bindings (
                \"RENAMED-MARKER-8c1d2a\" TEXT PRIMARY KEY NOT NULL,
                applied_digest TEXT NOT NULL,
                shared_revision TEXT NOT NULL,
                adapter_revision TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );",
        )
        .unwrap();
        let err = load_bindings_from(&conn).unwrap_err();
        assert!(!format!("{err:#}").contains("RENAMED-MARKER-8c1d2a"));
    }
}
