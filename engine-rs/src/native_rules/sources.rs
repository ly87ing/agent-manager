//! 来源规则：技能来源（git 仓库镜像或本地目录）根下 `rules/*.md` 提供的规则模板。
//!
//! 规则内容不随程序发布：作者在内容仓库里维护，本工具作为来源读取，与技能共用同一个
//! 集合定义、镜像目录与「更新资源库」。来源规则只读、不入库；应用到某个规则文件时才写。
//! 另在库里记下每个规则文件最后一次从哪条来源规则写入、写入的摘要，据此判断「来源有新
//! 版本、而你的文件还是上次写入的那版」。读取零写：没有镜像目录或表就当没有。

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension};
use serde_json::{json, Value};

use crate::native::Context;
use crate::native_mcp::model::sha256_hex;
use crate::store;

/// 单个规则文件的读取上限：常驻规则按行计费，远小于此。
const MAX_RULE_BYTES: u64 = 256 * 1024;

pub(crate) const APPLIED_DDL: &str = "CREATE TABLE IF NOT EXISTS rule_source_applied (
    target TEXT PRIMARY KEY NOT NULL,
    rule TEXT NOT NULL,
    digest TEXT NOT NULL,
    applied_at TEXT NOT NULL
)";

/// 一条来源规则。
#[derive(Debug, Clone)]
pub(crate) struct SourceRule {
    /// `<集合名>/<文件名去掉 .md>`。
    pub(crate) id: String,
    pub(crate) collection: String,
    pub(crate) name: String,
    pub(crate) body: String,
    /// 正文 sha256：来源规则的版本。
    pub(crate) revision: String,
    /// 本地目录来源（改了即时生效，没有「更新资源库」）。
    pub(crate) local: bool,
    pub(crate) root: PathBuf,
}

impl SourceRule {
    pub(crate) fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "collection": self.collection,
            "name": self.name,
            "body": self.body,
            "revision": self.revision,
            "local": self.local,
        })
    }
}

fn valid_stem(stem: &str) -> bool {
    !stem.is_empty()
        && stem
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// 读出所有启用来源里的规则。镜像还没取回、目录不存在、不是普通文件、过大或非 UTF-8 的
/// 条目都跳过，不报错：来源规则只是可选的模板。
pub(crate) fn load(ctx: &Context) -> Result<Vec<SourceRule>> {
    let meta = crate::native_skills::load_metadata(ctx)?;
    let mut out = Vec::new();
    for coll in meta.collections.values() {
        if coll.deleted || !coll.enabled {
            continue;
        }
        let root = crate::native_skills::collection_source_root(ctx, coll);
        let Ok(entries) = std::fs::read_dir(root.join("rules")) else {
            continue;
        };
        let mut files: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
        files.sort();
        for path in files {
            let Some(file_name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let Some(stem) = file_name.strip_suffix(".md") else {
                continue;
            };
            if file_name.eq_ignore_ascii_case("readme.md") || !valid_stem(stem) {
                continue;
            }
            // 符号链接不跟：来源目录里的规则必须就是来源自己的文件
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if !meta.is_file() || meta.len() > MAX_RULE_BYTES {
                continue;
            }
            let Ok(body) = std::fs::read_to_string(&path) else {
                continue;
            };
            out.push(SourceRule {
                id: format!("{}/{stem}", coll.name),
                collection: coll.name.clone(),
                name: stem.to_string(),
                revision: sha256_hex(&body),
                body,
                local: coll.source_path.is_some(),
                root: root.clone(),
            });
        }
    }
    Ok(out)
}

/// 没有任何来源规则时，给界面的推荐来源（与技能推荐同一份），带上已登记的同名/同地址集合。
pub(crate) fn suggested_sources(ctx: &Context) -> Result<Value> {
    let meta = crate::native_skills::load_metadata(ctx)?;
    let items = crate::native_skills::recommendations_value(&meta.collections);
    let items = items
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|item| {
            let url = item["url"].as_str().unwrap_or_default();
            let name = item["name"].as_str().unwrap_or_default();
            let collection = meta
                .collections
                .values()
                .find(|c| !c.deleted && (c.name == name || c.url.as_deref() == Some(url)))
                .map(|c| c.name.clone());
            json!({
                "name": name,
                "description": item["description"],
                "url": url,
                "collection": collection,
            })
        })
        .collect::<Vec<_>>();
    Ok(Value::Array(items))
}

/// 某个规则文件最后一次从来源规则写入的记录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Applied {
    pub(crate) rule: String,
    pub(crate) digest: String,
}

pub(crate) fn load_applied_from(conn: &Connection) -> Result<BTreeMap<String, Applied>> {
    let exists: Option<String> = conn
        .query_row(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'rule_source_applied'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    if exists.is_none() {
        return Ok(BTreeMap::new());
    }
    let mut stmt = conn.prepare("SELECT target, rule, digest FROM rule_source_applied")?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            Applied {
                rule: row.get(1)?,
                digest: row.get(2)?,
            },
        ))
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// 只读：缺库即空。
pub(crate) fn load_applied(ctx: &Context) -> Result<BTreeMap<String, Applied>> {
    match store::open_readonly(ctx)? {
        Some(conn) => load_applied_from(&conn),
        None => Ok(BTreeMap::new()),
    }
}

pub(crate) fn record_applied(
    conn: &Connection,
    target: &str,
    rule: &str,
    digest: &str,
) -> Result<()> {
    conn.execute_batch(APPLIED_DDL)?;
    conn.execute(
        "INSERT INTO rule_source_applied (target, rule, digest, applied_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(target) DO UPDATE SET rule = excluded.rule, digest = excluded.digest,
            applied_at = excluded.applied_at",
        rusqlite::params![target, rule, digest, super::templates::now()],
    )?;
    Ok(())
}

/// 用别的内容写了这个规则文件：它不再跟随来源。
pub(crate) fn forget_applied(conn: &Connection, target: &str) -> Result<()> {
    if load_applied_from(conn)?.contains_key(target) {
        conn.execute(
            "DELETE FROM rule_source_applied WHERE target = ?1",
            [target],
        )?;
    }
    Ok(())
}

/// git 来源当前的提交号（尽力而为，只进审计）；本地目录来源没有。
pub(crate) fn source_commit(rule: &SourceRule) -> Option<String> {
    if rule.local {
        return None;
    }
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(&rule.root)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|commit| !commit.is_empty())
}

/// 状态里每个目标的「跟随来源」信息：上次从哪条来源规则写入、文件是否还是那一版、
/// 来源是否有新版本。
pub(crate) fn follow_json(
    applied: Option<&Applied>,
    file_digest: Option<&str>,
    rules: &[SourceRule],
) -> Value {
    let Some(applied) = applied else {
        return Value::Null;
    };
    let current = rules.iter().find(|r| r.id == applied.rule);
    let file_unchanged = file_digest == Some(applied.digest.as_str());
    let update_available =
        file_unchanged && current.is_some_and(|rule| rule.revision != applied.digest);
    json!({
        "rule": applied.rule,
        "applied_revision": applied.digest,
        "current_revision": current.map(|rule| rule.revision.clone()),
        "file_unchanged": file_unchanged,
        "update_available": update_available,
    })
}
