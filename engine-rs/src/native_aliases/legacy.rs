//! 旧版本保存的别名（`alias_sources` 表的 aliases.shell 行）。
//!
//! 旧版本把别名存在库里再「同步」进 rc 文件。现在 rc 文件里的受管区块才是生效的别名：
//! 旧行里用户存过、与内置推荐不同、而实际区块里没有（同名同值）的别名，以「已保存未生效」
//! 建议出现，可加入实际区块（`aliases:apply --legacy`）或丢弃（`aliases:discard-legacy`，
//! 删掉库里这一条，没有剩余候选时整行删除）。
//!
//! 读路径即时推导、零写；旧表读取宽松：缺表、结构不符或行损坏一律视为没有旧来源，不挡住
//! 别名页。写路径清理：写入区块时，把写入前后已在实际区块里的条目从库里删掉，之后用户从
//! 区块里删掉它也不会再冒出来。旧 `alias_target_bindings` 不再是写入门槛，原样留在库里。
//! 审计只记建议 id 与名称，不记取值。

use anyhow::{anyhow, Result};
use rusqlite::{Connection, TransactionBehavior};
use serde_json::{json, Map, Value};

use super::{args, model};
use crate::native::{Context, Request};
use crate::native_mcp::model::{canonical_compact, now_iso, sha256_hex};
use crate::store;

/// 旧来源键（旧版本 `sources:get/set` 与配置备份里的键）。
pub(crate) const LEGACY_KEY: &str = "aliases.shell";

const TABLE_DDL: &str = "CREATE TABLE IF NOT EXISTS alias_sources (
    key TEXT PRIMARY KEY NOT NULL,
    value_json TEXT NOT NULL,
    revision TEXT NOT NULL,
    updated_at TEXT NOT NULL
)";

/// 宽松读取旧行：缺表、非法 JSON 或结构非法一律视为没有（不回显内容）。
pub(crate) fn read_row(conn: &Connection) -> Option<Map<String, Value>> {
    let text: String = conn
        .query_row(
            "SELECT value_json FROM alias_sources WHERE key = ?1",
            [LEGACY_KEY],
            |row| row.get(0),
        )
        .ok()?;
    let Ok(Value::Object(obj)) = serde_json::from_str::<Value>(&text) else {
        return None;
    };
    model::validate_source(&obj).ok()?;
    Some(obj)
}

/// 只读读取：缺库为 None，零写。
pub(crate) fn read_row_readonly(ctx: &Context) -> Result<Option<Map<String, Value>>> {
    Ok(match store::open_readonly(ctx)? {
        Some(conn) => read_row(&conn),
        None => None,
    })
}

/// 一条旧版本保存的别名（取值已按旧行的 timezone/lang 解析）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LegacyAlias {
    pub id: String,
    pub name: String,
    pub value: String,
}

/// 旧行里与内置推荐不同的别名。没存过就没有。
pub(crate) fn candidates(row: Option<&Map<String, Value>>) -> Vec<LegacyAlias> {
    let Some(row) = row else {
        return Vec::new();
    };
    let builtin = model::legacy_builtin_aliases();
    model::source_aliases(row)
        .into_iter()
        .filter(|(name, value)| !builtin.iter().any(|(n, v)| n == name && v == value))
        .map(|(name, value)| LegacyAlias {
            id: model::suggestion_id(model::LEGACY_PREFIX, &[LEGACY_KEY, &name, &value]),
            name,
            value,
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Pruned {
    Unchanged,
    Updated,
    /// 没有剩余候选，整行删除。
    Removed,
}

/// 写路径清理：从旧行里删掉 `drop` 命中的候选；没有剩余候选时整行删除。
pub(crate) fn prune_in_tx(
    conn: &Connection,
    row: Option<&Map<String, Value>>,
    drop: &dyn Fn(&LegacyAlias) -> bool,
) -> Result<Pruned> {
    let Some(obj) = row else {
        return Ok(Pruned::Unchanged);
    };
    let all = candidates(row);
    if !all.iter().any(drop) {
        return Ok(Pruned::Unchanged);
    }
    if all.iter().all(drop) {
        conn.execute("DELETE FROM alias_sources WHERE key = ?1", [LEGACY_KEY])?;
        return Ok(Pruned::Removed);
    }
    let mut next = obj.clone();
    if let Some(Value::Object(aliases)) = next.get_mut("aliases") {
        for candidate in all.iter().filter(|c| drop(c)) {
            aliases.remove(&candidate.name);
        }
    }
    let text = canonical_compact(&Value::Object(next));
    conn.execute(
        "UPDATE alias_sources SET value_json = ?2, revision = ?3, updated_at = ?4 WHERE key = ?1",
        rusqlite::params![LEGACY_KEY, text, sha256_hex(&text), now_iso()],
    )?;
    Ok(Pruned::Updated)
}

/// 配置备份导出：仍有候选时导出旧行正文，换机器导入后同样作为未生效建议出现。
pub(crate) fn exportable(row: Option<&Map<String, Value>>) -> Option<String> {
    let obj = row?;
    if candidates(row).is_empty() {
        return None;
    }
    Some(canonical_compact(&Value::Object(obj.clone())))
}

/// 配置备份导入：旧格式来源写回旧表（调用方已做结构与凭据校验），按已保存未生效处理。
pub(crate) fn write_row_in_tx(conn: &Connection, obj: &Map<String, Value>) -> Result<()> {
    conn.execute_batch(TABLE_DDL)?;
    let text = canonical_compact(&Value::Object(obj.clone()));
    conn.execute(
        "INSERT INTO alias_sources (key, value_json, revision, updated_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, revision = excluded.revision, updated_at = excluded.updated_at",
        rusqlite::params![LEGACY_KEY, text, sha256_hex(&text), now_iso()],
    )?;
    Ok(())
}

const GONE: &str = "这条已保存的快捷命令已不存在（可能已加入或已丢弃），请刷新后重试";

/// aliases:discard-legacy：只写库，删掉库里这条旧版别名；不动 rc 文件，不取 apply 锁。
pub(crate) fn discard(ctx: &Context, req: &Request) -> Result<Value> {
    let id = args::parse_discard_args(req)?;
    // 只读预检：不存在的 id 在创建任何东西之前拒绝。
    let row = read_row_readonly(ctx)?;
    candidates(row.as_ref())
        .into_iter()
        .find(|c| c.id == id)
        .ok_or_else(|| anyhow!(GONE))?;

    let mut conn = store::open_write(ctx)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let row = read_row(&tx);
    let item = candidates(row.as_ref())
        .into_iter()
        .find(|c| c.id == id)
        .ok_or_else(|| anyhow!(GONE))?;
    let pruned = prune_in_tx(&tx, row.as_ref(), &|c| c.id == item.id)?;
    let source_removed = pruned == Pruned::Removed;
    store::record_operation(
        &tx,
        Some(&req.run_id),
        "aliases:discard-legacy",
        Some(&item.name),
        Some(&json!({ "legacy": id, "name": item.name, "source_removed": source_removed })),
    )?;
    tx.commit()?;
    Ok(json!({ "domain": "aliases", "discarded": id, "source_removed": source_removed }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(value: Value) -> Map<String, Value> {
        value.as_object().unwrap().clone()
    }

    #[test]
    fn only_aliases_differing_from_builtin_are_candidates() {
        let builtin_claude = model::legacy_builtin_aliases()
            .into_iter()
            .find(|(n, _)| n == "claude")
            .unwrap()
            .1;
        // 与内置同名同值（内置模板 + 内置时区）不算；改过时区后同一模板解析出的新值算
        let same = row(json!({
            "aliases": {"claude": "TZ=\"{timezone}\" LANG=\"{lang}\" LC_ALL=\"{lang}\" claude", "ll": "ls -l"}
        }));
        let names: Vec<String> = candidates(Some(&same))
            .into_iter()
            .map(|c| c.name)
            .collect();
        assert_eq!(names, ["ll"]);
        let shanghai = row(json!({
            "timezone": "Asia/Shanghai",
            "aliases": {"claude": "TZ=\"{timezone}\" LANG=\"{lang}\" LC_ALL=\"{lang}\" claude"}
        }));
        let all = candidates(Some(&shanghai));
        assert_eq!(all.len(), 1);
        assert_ne!(all[0].value, builtin_claude);
        assert!(all[0].value.contains("Asia/Shanghai"));
        assert!(model_valid_id(&all[0].id));
        assert!(candidates(None).is_empty());
        assert!(exportable(Some(&row(json!({"aliases": {}})))).is_none());
    }

    fn model_valid_id(id: &str) -> bool {
        model::valid_suggestion_id(id, model::LEGACY_PREFIX)
    }

    #[test]
    fn lenient_read_skips_bad_rows() {
        let conn = Connection::open_in_memory().unwrap();
        assert!(read_row(&conn).is_none(), "缺表视为没有");
        conn.execute_batch(TABLE_DDL).unwrap();
        for text in ["{not json", r#"{"aliases": {"bad name": "x"}}"#, "[]"] {
            conn.execute(
                "INSERT OR REPLACE INTO alias_sources VALUES ('aliases.shell', ?1, 'r', 't')",
                [text],
            )
            .unwrap();
            assert!(read_row(&conn).is_none(), "{text}");
        }
    }

    #[test]
    fn prune_updates_then_removes_row() {
        let conn = Connection::open_in_memory().unwrap();
        let obj = row(json!({"aliases": {"a": "1", "b": "2"}}));
        write_row_in_tx(&conn, &obj).unwrap();
        let current = read_row(&conn);
        let pruned = prune_in_tx(&conn, current.as_ref(), &|c| c.name == "a").unwrap();
        assert_eq!(pruned, Pruned::Updated);
        let current = read_row(&conn);
        let left: Vec<String> = candidates(current.as_ref())
            .into_iter()
            .map(|c| c.name)
            .collect();
        assert_eq!(left, ["b"]);
        let pruned = prune_in_tx(&conn, current.as_ref(), &|c| c.name == "b").unwrap();
        assert_eq!(pruned, Pruned::Removed);
        assert!(read_row(&conn).is_none());
        assert_eq!(
            prune_in_tx(&conn, None, &|_| true).unwrap(),
            Pruned::Unchanged
        );
    }
}
