//! 规则模板：用户保存的「已保存未生效」规则正文（`rule_templates` 表）。
//!
//! 模板只写库、不动文件；应用到目标时由 apply 写实际文件。内置推荐不在这里，
//! 永不入库。旧版本的 `rule_sources`（共享 + 助手专用来源）在写路径迁移成模板
//! （`migrate_legacy_sources`，由 `store::open_write` 调用），读路径按同一判定即时
//! 推导，保证迁移前后界面看到的模板一致。表结构或行损坏 fail-closed，错误不回显正文。

use std::collections::BTreeMap;

use anyhow::{bail, Result};
use rusqlite::Connection;
use serde_json::{json, Value};

use crate::native_mcp::model::sha256_hex;

const TABLE_DDL: &str = "CREATE TABLE IF NOT EXISTS rule_templates (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL UNIQUE,
    body TEXT NOT NULL,
    origin TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
)";

pub(crate) const ORIGIN_USER: &str = "user";
pub(crate) const ORIGIN_BACKUP: &str = "backup";
pub(crate) const ORIGIN_LEGACY: &str = "legacy";

const NAME_MAX_CHARS: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Template {
    pub id: String,
    pub name: String,
    pub body: String,
    pub origin: String,
    pub created_at: String,
    pub updated_at: String,
}

impl Template {
    /// 模板 revision：名称与正文一起算，任一变化都让旧 revision 过期。
    pub(crate) fn revision(&self) -> String {
        sha256_hex(&serde_json::to_string(&json!([self.name, self.body])).expect("JSON"))
    }

    pub(crate) fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "name": self.name,
            "body": self.body,
            "revision": self.revision(),
            "origin": self.origin,
            "created_at": self.created_at,
            "updated_at": self.updated_at,
        })
    }
}

/// 模板与迁移标记的时间戳：RFC 3339 UTC 毫秒（与库内其他 updated_at 同格式）。
pub(crate) fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

pub(crate) fn new_id() -> String {
    format!("tpl-{}", crate::store::uuid_v4())
}

/// 模板 id 形状：小写字母、数字与连字符，1–64 位，以字母或数字开头。
pub(crate) fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// 名称：去首尾空白后非空、不超过 100 字、不含控制字符（含换行）。
pub(crate) fn validate_name(raw: &str) -> Result<String> {
    let name = raw.trim();
    if name.is_empty() {
        bail!("模板名称不能为空");
    }
    if name.chars().count() > NAME_MAX_CHARS {
        bail!("模板名称不能超过 {NAME_MAX_CHARS} 个字");
    }
    if name.chars().any(char::is_control) {
        bail!("模板名称不能包含换行或控制字符");
    }
    Ok(name.to_string())
}

/// 入库正文：无 NUL、不命中凭据检测（数据库不保存明文凭据）。错误不回显正文。
pub(crate) fn validate_body(body: &str) -> Result<()> {
    if body.contains('\0') {
        bail!("规则正文不能包含 NUL 字符");
    }
    if let Some(kind) = crate::native::reject_secret_text(body) {
        bail!("规则正文疑似含凭据（{kind}），拒绝保存到本机库");
    }
    Ok(())
}

pub(crate) fn ensure_table(conn: &Connection) -> Result<()> {
    conn.execute_batch(TABLE_DDL)?;
    Ok(())
}

fn table_exists(conn: &Connection, table: &str) -> Result<bool> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
        [table],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

fn check_schema(conn: &Connection) -> Result<()> {
    let mut stmt = conn.prepare("PRAGMA table_info(rule_templates)")?;
    let cols: Vec<(String, String, bool)> = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)? != 0,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let expected = ["id", "name", "body", "origin", "created_at", "updated_at"];
    if cols.len() != expected.len() {
        bail!("rule_templates 表结构损坏：列数不符");
    }
    for ((name, decl, notnull), want) in cols.iter().zip(expected) {
        if name != want || !decl.eq_ignore_ascii_case("TEXT") || !notnull {
            bail!("rule_templates 表结构损坏：列定义不符");
        }
    }
    Ok(())
}

fn validate_row(t: &Template) -> Result<()> {
    if !valid_id(&t.id) {
        bail!("rule_templates 存在非法 id 行");
    }
    if validate_name(&t.name).ok().as_deref() != Some(t.name.as_str()) {
        bail!("rule_templates 存在非法名称行");
    }
    if t.body.contains('\0') {
        bail!("rule_templates 存在含 NUL 的正文行");
    }
    if ![ORIGIN_USER, ORIGIN_BACKUP, ORIGIN_LEGACY].contains(&t.origin.as_str()) {
        bail!("rule_templates 存在未知来源行");
    }
    if t.created_at.is_empty() || t.updated_at.is_empty() {
        bail!("rule_templates 存在时间为空的行");
    }
    Ok(())
}

/// 已入库的模板；缺表视为空（读路径不建表）。
fn load_stored(conn: &Connection) -> Result<Vec<Template>> {
    if !table_exists(conn, "rule_templates")? {
        return Ok(Vec::new());
    }
    check_schema(conn)?;
    let mut stmt = conn.prepare(
        "SELECT id, name, body, origin, created_at, updated_at FROM rule_templates ORDER BY created_at, id",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(Template {
            id: row.get(0)?,
            name: row.get(1)?,
            body: row.get(2)?,
            origin: row.get(3)?,
            created_at: row.get(4)?,
            updated_at: row.get(5)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        let template = row?;
        validate_row(&template)?;
        out.push(template);
    }
    Ok(out)
}

/// 界面与 apply 看到的模板全集：已入库模板 + 尚未迁移的旧来源即时推导出的模板。
pub(crate) fn load_from(conn: &Connection) -> Result<Vec<Template>> {
    let mut out = load_stored(conn)?;
    if !legacy_migrated(conn)? {
        if let Some((sources, updated_at)) = read_legacy_sources(conn) {
            for template in legacy_templates(&sources, &updated_at) {
                if !out.iter().any(|t| t.id == template.id) {
                    out.push(template);
                }
            }
        }
    }
    Ok(out)
}

/// 同名时追加序号，保证名称唯一。
pub(crate) fn unique_name(existing: &[Template], base: &str) -> String {
    if !existing.iter().any(|t| t.name == base) {
        return base.to_string();
    }
    (2..)
        .map(|n| format!("{base} ({n})"))
        .find(|candidate| !existing.iter().any(|t| &t.name == candidate))
        .expect("序号无穷")
}

pub(crate) fn insert(conn: &Connection, t: &Template) -> Result<()> {
    conn.execute(
        "INSERT INTO rule_templates (id, name, body, origin, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![t.id, t.name, t.body, t.origin, t.created_at, t.updated_at],
    )?;
    Ok(())
}

pub(crate) fn update(conn: &Connection, t: &Template) -> Result<()> {
    conn.execute(
        "UPDATE rule_templates SET name = ?2, body = ?3, updated_at = ?4 WHERE id = ?1",
        rusqlite::params![t.id, t.name, t.body, t.updated_at],
    )?;
    Ok(())
}

pub(crate) fn delete(conn: &Connection, id: &str) -> Result<()> {
    conn.execute("DELETE FROM rule_templates WHERE id = ?1", [id])?;
    Ok(())
}

/// 整体替换全部模板（配置备份恢复用）。
pub(crate) fn replace_all(conn: &Connection, templates: &[Template]) -> Result<()> {
    ensure_table(conn)?;
    conn.execute("DELETE FROM rule_templates", [])?;
    for template in templates {
        insert(conn, template)?;
    }
    Ok(())
}

/// 按 id 覆盖写入（旧来源模板在备份恢复时用固定 id 更新）。
pub(crate) fn upsert(conn: &Connection, t: &Template) -> Result<()> {
    ensure_table(conn)?;
    conn.execute(
        "INSERT INTO rule_templates (id, name, body, origin, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(id) DO UPDATE SET name = excluded.name, body = excluded.body,
            origin = excluded.origin, updated_at = excluded.updated_at",
        rusqlite::params![t.id, t.name, t.body, t.origin, t.created_at, t.updated_at],
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// 旧来源（rule_sources：rules.shared / rules.claude / rules.codex）
// ---------------------------------------------------------------------------

pub(crate) const LEGACY_KEY_SHARED: &str = "rules.shared";
pub(crate) const LEGACY_KEY_CLAUDE: &str = "rules.claude";
pub(crate) const LEGACY_KEY_CODEX: &str = "rules.codex";
pub(crate) const LEGACY_KEYS: [&str; 3] = [LEGACY_KEY_SHARED, LEGACY_KEY_CLAUDE, LEGACY_KEY_CODEX];

/// 旧版本编译内置的来源默认值：只用于判定旧库行「是否改过」。共享规则冻结在规则迁出本仓
/// （2026-09-23）时的内容；两个助手专用来源当时是空的。
pub(crate) const LEGACY_DEFAULT_SHARED: &str =
    include_str!("../../assets/legacy-rules/shared-core.md");
const LEGACY_DEFAULT_CLAUDE: &str = "";
const LEGACY_DEFAULT_CODEX: &str = "";

/// 迁移完成标记（schema_meta）：迁移后用户删掉旧来源模板也不会再冒出来。
const LEGACY_MARKER_KEY: &str = "rules_legacy_migrated_at";

pub(crate) const LEGACY_ID_CLAUDE: &str = "legacy-claude";
pub(crate) const LEGACY_ID_CODEX: &str = "legacy-codex";

fn legacy_default(key: &str) -> &'static str {
    match key {
        LEGACY_KEY_SHARED => LEGACY_DEFAULT_SHARED,
        LEGACY_KEY_CLAUDE => LEGACY_DEFAULT_CLAUDE,
        _ => LEGACY_DEFAULT_CODEX,
    }
}

/// 旧版本写入规则文件的渲染：逐行 trim_end，删除空白行，shared 在前 adapter 在后，
/// 末尾恰好一个换行。只用于把旧来源转换成模板，生效内容不再这样拼接。
pub(crate) fn legacy_render(shared: &str, adapter: &str) -> String {
    let mut lines: Vec<&str> = Vec::new();
    for text in [shared, adapter] {
        for line in text.lines() {
            let trimmed = line.trim_end();
            if !trimmed.trim().is_empty() {
                lines.push(trimmed);
            }
        }
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

/// 由旧来源推导模板：共享或该助手专用来源被用户存过、且与旧内置不同，才生成
/// 「旧规则来源（Claude/Codex）」= 按旧规则渲染的正文。没存过的键按旧内置处理。
pub(crate) fn legacy_templates(
    stored: &BTreeMap<String, String>,
    created_at: &str,
) -> Vec<Template> {
    let changed = |key: &str| stored.get(key).is_some_and(|v| v != legacy_default(key));
    let value = |key: &str| {
        stored
            .get(key)
            .map(String::as_str)
            .unwrap_or_else(|| legacy_default(key))
    };
    let mut out = Vec::new();
    for (id, name, adapter_key) in [
        (LEGACY_ID_CLAUDE, "旧规则来源（Claude）", LEGACY_KEY_CLAUDE),
        (LEGACY_ID_CODEX, "旧规则来源（Codex）", LEGACY_KEY_CODEX),
    ] {
        if changed(LEGACY_KEY_SHARED) || changed(adapter_key) {
            out.push(Template {
                id: id.to_string(),
                name: name.to_string(),
                body: legacy_render(value(LEGACY_KEY_SHARED), value(adapter_key)),
                origin: ORIGIN_LEGACY.to_string(),
                created_at: created_at.to_string(),
                updated_at: created_at.to_string(),
            });
        }
    }
    out
}

fn legacy_migrated(conn: &Connection) -> Result<bool> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM schema_meta WHERE key = ?1",
        [LEGACY_MARKER_KEY],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

/// 读旧来源表（宽松：缺表或读不了一律视为没有旧来源，不因旧数据挡住新功能）。
/// 返回已存键值与最近更新时间。
fn read_legacy_sources(conn: &Connection) -> Option<(BTreeMap<String, String>, String)> {
    let mut stmt = conn
        .prepare("SELECT key, value, updated_at FROM rule_sources")
        .ok()?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .ok()?
        .collect::<rusqlite::Result<Vec<_>>>()
        .ok()?;
    let mut map = BTreeMap::new();
    let mut latest = String::new();
    for (key, value, updated_at) in rows {
        if LEGACY_KEYS.contains(&key.as_str()) {
            if updated_at > latest {
                latest = updated_at;
            }
            map.insert(key, value);
        }
    }
    Some((map, latest))
}

/// 写路径迁移（store::open_write 的写事务内调用）：旧来源里用户改过的内容转成
/// 旧来源模板，并写入迁移标记；不改任何实际文件，也不删旧表。幂等：已迁移或
/// 没有旧来源表即零写。
pub(crate) fn migrate_legacy_sources(conn: &Connection) -> Result<()> {
    if !table_exists(conn, "rule_sources")? || legacy_migrated(conn)? {
        return Ok(());
    }
    let Some((sources, updated_at)) = read_legacy_sources(conn) else {
        return Ok(());
    };
    let legacy = legacy_templates(&sources, &updated_at);
    let mut created = Vec::new();
    if !legacy.is_empty() {
        ensure_table(conn)?;
        // 模板表被外部弄坏时不迁移（读路径同样 fail-closed 报错），不因此挡住其他域的写入
        let Ok(mut existing) = load_stored(conn) else {
            return Ok(());
        };
        for mut template in legacy {
            if existing.iter().any(|t| t.id == template.id) {
                continue;
            }
            template.name = unique_name(&existing, &template.name);
            insert(conn, &template)?;
            created.push(template.id.clone());
            existing.push(template);
        }
    }
    conn.execute(
        "INSERT INTO schema_meta (key, value) VALUES (?1, ?2)",
        rusqlite::params![LEGACY_MARKER_KEY, now()],
    )?;
    if !created.is_empty() {
        crate::store::record_operation(
            conn,
            None,
            "rules:migrate-legacy",
            None,
            Some(&json!({ "action": "migrate-legacy", "templates": created })),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stored(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn legacy_render_handles_crlf_trailing_ws_blank_and_leading_spaces() {
        let shared = "alpha  \r\n\r\n   \t\n  beta gamma\t\n";
        let adapter = "delta\r\n \r\nepsilon  ";
        assert_eq!(
            legacy_render(shared, adapter),
            "alpha\n  beta gamma\ndelta\nepsilon\n"
        );
        assert_eq!(legacy_render("", ""), "\n");
    }

    #[test]
    fn old_default_apply_wrote_exactly_the_shared_core() {
        // 旧版本按默认来源写入的文件就是共享规则本身（助手专用来源为空），与迁出时的 core.md 一致
        assert_eq!(
            legacy_render(LEGACY_DEFAULT_SHARED, LEGACY_DEFAULT_CLAUDE),
            LEGACY_DEFAULT_SHARED
        );
        assert_eq!(
            legacy_render(LEGACY_DEFAULT_SHARED, LEGACY_DEFAULT_CODEX),
            LEGACY_DEFAULT_SHARED
        );
    }

    #[test]
    fn never_changed_sources_yield_no_templates() {
        assert!(legacy_templates(&stored(&[]), "t").is_empty());
        let same = stored(&[
            (LEGACY_KEY_SHARED, LEGACY_DEFAULT_SHARED),
            (LEGACY_KEY_CLAUDE, LEGACY_DEFAULT_CLAUDE),
        ]);
        assert!(
            legacy_templates(&same, "t").is_empty(),
            "存过但与内置相同不算改过"
        );
    }

    #[test]
    fn changed_shared_yields_both_agent_templates() {
        let out = legacy_templates(
            &stored(&[(LEGACY_KEY_SHARED, "团队规则\n\n第二行  \n")]),
            "t",
        );
        let ids: Vec<&str> = out.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, [LEGACY_ID_CLAUDE, LEGACY_ID_CODEX]);
        assert_eq!(out[0].name, "旧规则来源（Claude）");
        assert_eq!(out[1].name, "旧规则来源（Codex）");
        for template in &out {
            assert_eq!(template.body, "团队规则\n第二行\n");
            assert_eq!(template.origin, ORIGIN_LEGACY);
        }
    }

    #[test]
    fn changed_adapter_yields_only_that_agent_template() {
        let out = legacy_templates(&stored(&[(LEGACY_KEY_CLAUDE, "只给 Claude\n")]), "t");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id, LEGACY_ID_CLAUDE);
        assert_eq!(out[0].body, format!("{LEGACY_DEFAULT_SHARED}只给 Claude\n"));
    }

    #[test]
    fn names_ids_and_bodies_are_validated() {
        assert_eq!(validate_name("  我的模板 ").unwrap(), "我的模板");
        assert!(validate_name("   ").is_err());
        assert!(validate_name("a\nb").is_err());
        assert!(validate_name(&"长".repeat(101)).is_err());
        assert!(valid_id("tpl-0a1b"));
        assert!(valid_id(LEGACY_ID_CLAUDE));
        assert!(!valid_id("Bad"));
        assert!(!valid_id("-x"));
        assert!(!valid_id(""));
        assert!(validate_body("a\0b").is_err());
        let err = validate_body(&format!("key sk-{}", "a".repeat(24))).unwrap_err();
        assert!(!format!("{err:#}").contains("sk-"), "错误不得回显正文");
    }

    #[test]
    fn unique_name_appends_sequence() {
        let t = |name: &str| Template {
            id: "x".into(),
            name: name.into(),
            body: String::new(),
            origin: ORIGIN_USER.into(),
            created_at: "t".into(),
            updated_at: "t".into(),
        };
        assert_eq!(unique_name(&[], "a"), "a");
        assert_eq!(unique_name(&[t("a"), t("a (2)")], "a"), "a (3)");
    }
}
