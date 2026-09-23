//! 旧版本保存的 hooks（`hook_sources` 表：hooks.claude / hooks.codex）。
//!
//! 旧版本把 hooks 存在库里再「同步」进助手配置。现在实际配置才是生效的 hooks：旧库里
//! 用户存过、与内置不同、而主助手实际配置里没有的 hook，以「旧版已保存的 hooks（未生效）」
//! 建议出现，可逐条加入实际配置（`hooks:apply --legacy`）或丢弃（`hooks:discard-legacy`，
//! 删掉库里这一条，来源空了整行删除）。已在实际配置里的不重复提示。
//!
//! 读路径即时推导、零写；旧表读取宽松：缺表、结构不符或行损坏一律视为没有旧来源，不挡住
//! hooks 页。写路径清理：对主助手目标写入时，把写入前后已在实际配置里的条目从库里删掉，
//! 之后用户从实际配置里删掉它也不会再冒出来。旧 `hook_target_bindings` 不再是写入门槛，
//! 原样留在库里。审计只记建议 id 与来源键，不记 command。

use std::collections::BTreeMap;

use anyhow::{anyhow, Result};
use rusqlite::{Connection, TransactionBehavior};
use serde_json::{json, Map, Value};

use super::args;
use super::model::{
    self, builtin_source, collect_signatures, remove_signature, suggestion_id, HookKind, Signature,
    LEGACY_PREFIX,
};
use crate::native::{Context, Request};
use crate::native_mcp::model::sha256_hex;
use crate::store;

const TABLE_DDL: &str = "CREATE TABLE IF NOT EXISTS hook_sources (
    key TEXT PRIMARY KEY NOT NULL,
    value_json TEXT NOT NULL,
    revision TEXT NOT NULL,
    updated_at TEXT NOT NULL
)";

/// 读到的旧来源：来源键 → 通过结构校验的来源对象。
pub(crate) type Rows = BTreeMap<String, Map<String, Value>>;

/// 宽松读取旧来源：缺表、列不符、非法 JSON 或结构非法的行一律跳过（不回显内容）。
pub(crate) fn read_rows(conn: &Connection) -> Rows {
    let mut out = Rows::new();
    let Ok(mut stmt) = conn.prepare("SELECT key, value_json FROM hook_sources") else {
        return out;
    };
    let Ok(rows) = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    }) else {
        return out;
    };
    for (key, text) in rows.flatten() {
        if key != model::LEGACY_KEY_CLAUDE && key != model::LEGACY_KEY_CODEX {
            continue;
        }
        let Ok(Value::Object(obj)) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        if model::validate_source(&key, &obj).is_ok() {
            out.insert(key, obj);
        }
    }
    out
}

/// 只读读取：缺库为空，零写。
pub(crate) fn read_rows_readonly(ctx: &Context) -> Result<Rows> {
    Ok(match store::open_readonly(ctx)? {
        Some(conn) => read_rows(&conn),
        None => Rows::new(),
    })
}

/// 一条旧版本保存的 hook。
#[derive(Debug, Clone)]
pub(crate) struct LegacyHook {
    pub id: String,
    pub key: &'static str,
    pub kind: HookKind,
    pub event: String,
    /// 所在条目除 hooks 以外的字段（matcher 等），加入实际配置时原样带上。
    pub entry_fields: Map<String, Value>,
    pub hook: Value,
    pub signature: Signature,
}

impl LegacyHook {
    pub(crate) fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "source_key": self.key,
            "event": self.signature.0,
            "matcher": self.signature.1,
            "command": self.signature.2,
        })
    }
}

/// 旧行里与内置不同的 hook（按签名去重，保持出现顺序）。没存过就没有。
pub(crate) fn candidates(rows: &Rows, kind: HookKind) -> Vec<LegacyHook> {
    let key = kind.legacy_key();
    let Some(obj) = rows.get(key) else {
        return Vec::new();
    };
    let builtin = collect_signatures(&Value::Object(builtin_source(kind).0));
    let mut out: Vec<LegacyHook> = Vec::new();
    let Some(events) = obj.get("hooks").and_then(Value::as_object) else {
        return out;
    };
    for (event, entries) in events {
        for entry in entries.as_array().into_iter().flatten() {
            let Some(entry) = entry.as_object() else {
                continue;
            };
            let mut entry_fields = entry.clone();
            entry_fields.remove("hooks");
            let matcher = entry
                .get("matcher")
                .and_then(Value::as_str)
                .map(str::to_string);
            for hook in entry
                .get("hooks")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                // 签名只对 command 类型成立（旧版本也只写过 command 类型）
                if hook.get("type").and_then(Value::as_str) != Some("command") {
                    continue;
                }
                let Some(command) = hook.get("command").and_then(Value::as_str) else {
                    continue;
                };
                let signature = (event.clone(), matcher.clone(), command.to_string());
                if builtin.contains(&signature) || out.iter().any(|c| c.signature == signature) {
                    continue;
                }
                out.push(LegacyHook {
                    id: suggestion_id(
                        LEGACY_PREFIX,
                        &[key, event, matcher.as_deref().unwrap_or(""), command],
                    ),
                    key,
                    kind,
                    event: event.clone(),
                    entry_fields: entry_fields.clone(),
                    hook: hook.clone(),
                    signature,
                });
            }
        }
    }
    out
}

/// 建议：旧行里有、主助手实际 hooks 里没有的。
pub(crate) fn suggestions(rows: &Rows, kind: HookKind, actual_hooks: &Value) -> Vec<LegacyHook> {
    let present = collect_signatures(actual_hooks);
    candidates(rows, kind)
        .into_iter()
        .filter(|c| !present.contains(&c.signature))
        .collect()
}

/// 按 id 在两个旧来源里找候选。
pub(crate) fn find(rows: &Rows, id: &str) -> Option<LegacyHook> {
    [HookKind::Claude, HookKind::Codex]
        .into_iter()
        .flat_map(|kind| candidates(rows, kind))
        .find(|c| c.id == id)
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
    rows: &Rows,
    kind: HookKind,
    drop: &dyn Fn(&Signature) -> bool,
) -> Result<Pruned> {
    let key = kind.legacy_key();
    let Some(obj) = rows.get(key) else {
        return Ok(Pruned::Unchanged);
    };
    let all = candidates(rows, kind);
    if !all.iter().any(|c| drop(&c.signature)) {
        return Ok(Pruned::Unchanged);
    }
    if all.iter().all(|c| drop(&c.signature)) {
        conn.execute("DELETE FROM hook_sources WHERE key = ?1", [key])?;
        return Ok(Pruned::Removed);
    }
    let mut hooks = obj
        .get("hooks")
        .cloned()
        .unwrap_or_else(|| Value::Object(Map::new()));
    for candidate in all.iter().filter(|c| drop(&c.signature)) {
        hooks = remove_signature(&hooks, &candidate.signature);
    }
    let mut next = obj.clone();
    next.insert("hooks".to_string(), hooks);
    let text = crate::native_sources::canonical_json(&next);
    conn.execute(
        "UPDATE hook_sources SET value_json = ?2, revision = ?3, updated_at = ?4 WHERE key = ?1",
        rusqlite::params![
            key,
            text,
            sha256_hex(&text),
            crate::native_mcp::model::now_iso()
        ],
    )?;
    Ok(Pruned::Updated)
}

/// 配置备份导出：仍有候选的旧行（canonical 正文），换机器导入后同样作为未生效建议出现。
pub(crate) fn exportable(rows: &Rows) -> BTreeMap<String, String> {
    rows.iter()
        .filter(|(key, _)| {
            let kind = if key.as_str() == model::LEGACY_KEY_CLAUDE {
                HookKind::Claude
            } else {
                HookKind::Codex
            };
            !candidates(rows, kind).is_empty()
        })
        .map(|(key, obj)| (key.clone(), crate::native_sources::canonical_json(obj)))
        .collect()
}

/// 配置备份导入：旧格式来源写回旧表（调用方已做结构与凭据校验），按旧版本保存的 hooks 处理。
pub(crate) fn write_row_in_tx(
    conn: &Connection,
    key: &str,
    obj: &Map<String, Value>,
) -> Result<()> {
    conn.execute_batch(TABLE_DDL)?;
    let text = crate::native_sources::canonical_json(obj);
    conn.execute(
        "INSERT INTO hook_sources (key, value_json, revision, updated_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, revision = excluded.revision, updated_at = excluded.updated_at",
        rusqlite::params![key, text, sha256_hex(&text), crate::native_mcp::model::now_iso()],
    )?;
    Ok(())
}

const GONE: &str = "这条旧版 hooks 已不存在（可能已加入或已丢弃），请刷新后重试";

/// hooks:discard-legacy：只写库，删掉库里这条旧版 hook；不动任何助手文件，不取 apply 锁。
pub(crate) fn discard(ctx: &Context, req: &Request) -> Result<Value> {
    let id = args::parse_discard_args(req)?;
    // 只读预检：不存在的 id 在创建任何东西之前拒绝。
    find(&read_rows_readonly(ctx)?, &id).ok_or_else(|| anyhow!(GONE))?;

    let mut conn = store::open_write(ctx)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let rows = read_rows(&tx);
    let item = find(&rows, &id).ok_or_else(|| anyhow!(GONE))?;
    let pruned = prune_in_tx(&tx, &rows, item.kind, &|sig| sig == &item.signature)?;
    let source_removed = pruned == Pruned::Removed;
    store::record_operation(
        &tx,
        Some(&req.run_id),
        "hooks:discard-legacy",
        Some(item.key),
        Some(&json!({ "legacy": id, "source_removed": source_removed })),
    )?;
    tx.commit()?;
    Ok(json!({ "domain": "hooks", "discarded": id, "source_removed": source_removed }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(key: &str, value: Value) -> Rows {
        let mut out = Rows::new();
        out.insert(key.to_string(), value.as_object().unwrap().clone());
        out
    }

    fn hook(command: &str) -> Value {
        json!({"type": "command", "command": command})
    }

    #[test]
    fn stored_hooks_become_candidates_and_present_ones_are_not_suggested() {
        let rows = rows(
            "hooks.claude",
            json!({
                "hooks": {"Stop": [{"matcher": "A", "timeout_hint": 1, "hooks": [hook("x"), hook("x"), hook("y")]}]},
                "removed_signatures": []
            }),
        );
        let all = candidates(&rows, HookKind::Claude);
        assert_eq!(all.len(), 2, "同签名去重");
        assert_eq!(
            all[0].entry_fields,
            *json!({"matcher": "A", "timeout_hint": 1})
                .as_object()
                .unwrap()
        );
        assert!(candidates(&rows, HookKind::Codex).is_empty());
        let actual = json!({"Stop": [{"matcher": "A", "hooks": [hook("x")]}]});
        let open = suggestions(&rows, HookKind::Claude, &actual);
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].signature.2, "y");
        assert_eq!(find(&rows, &open[0].id).unwrap().signature.2, "y");
    }

    #[test]
    fn builtin_default_row_has_no_candidates() {
        let (hooks, _) = builtin_source(HookKind::Claude);
        let rows = rows(
            "hooks.claude",
            json!({"hooks": hooks, "removed_signatures": []}),
        );
        assert!(candidates(&rows, HookKind::Claude).is_empty());
        assert!(exportable(&rows).is_empty());
    }

    #[test]
    fn lenient_read_skips_bad_rows() {
        let conn = Connection::open_in_memory().unwrap();
        assert!(read_rows(&conn).is_empty(), "缺表视为没有");
        conn.execute_batch(TABLE_DDL).unwrap();
        for (key, text) in [
            ("hooks.claude", "{not json"),
            ("hooks.codex", r#"{"hooks": [], "x": 1}"#),
            ("claude", r#"{"hooks": {}}"#),
        ] {
            conn.execute(
                "INSERT INTO hook_sources VALUES (?1, ?2, 'r', 't')",
                rusqlite::params![key, text],
            )
            .unwrap();
        }
        assert!(read_rows(&conn).is_empty());
    }

    #[test]
    fn prune_updates_then_removes_row() {
        let conn = Connection::open_in_memory().unwrap();
        let obj = json!({"hooks": {"Stop": [{"hooks": [hook("x"), hook("y")]}]}});
        write_row_in_tx(&conn, "hooks.codex", obj.as_object().unwrap()).unwrap();
        let rows = read_rows(&conn);
        let pruned = prune_in_tx(&conn, &rows, HookKind::Codex, &|s| s.2 == "x").unwrap();
        assert_eq!(pruned, Pruned::Updated);
        let rows = read_rows(&conn);
        let left: Vec<String> = candidates(&rows, HookKind::Codex)
            .into_iter()
            .map(|c| c.signature.2)
            .collect();
        assert_eq!(left, ["y"]);
        let pruned = prune_in_tx(&conn, &rows, HookKind::Codex, &|s| s.2 == "y").unwrap();
        assert_eq!(pruned, Pruned::Removed);
        assert!(read_rows(&conn).is_empty());
        let unchanged = prune_in_tx(&conn, &Rows::new(), HookKind::Codex, &|_| true).unwrap();
        assert_eq!(unchanged, Pruned::Unchanged);
    }
}
