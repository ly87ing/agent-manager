//! 用户在界面上填写的凭据取值（本机 SQLite）。
//!
//! 为什么需要它：此前变量只从「Agent Manager 进程环境」解析，而 macOS 上从 Dock /
//! Finder 启动的应用继承的是 launchd 环境，拿不到用户在 shell 里 export 的变量——
//! 用户在终端设好、打开应用一看仍是「未配置」，GUI 场景下这条路基本不通。
//!
//! 边界：值以明文存在本机数据库里（`secret_values` 表），任何以当前用户身份运行的
//! 进程都能读到它，与写进各助手配置文件的明文同级；库文件权限与 SQLite 一致。
//! 取值只在 apply 时用来渲染目标配置，**不进** status、操作审计与配置备份。
//!
//! 解析顺序：先查本表，查不到再回退进程环境。在界面上明确填过的值优先，
//! 这样用户改完立刻生效，不受启动方式影响。

use std::cell::RefCell;
use std::collections::BTreeMap;

use anyhow::{bail, Result};
use rusqlite::{Connection, Transaction};

use crate::native::Context;

const TABLE_DDL: &str = "CREATE TABLE IF NOT EXISTS secret_values (
    name TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL,
    updated_at TEXT NOT NULL
)";

/// 变量名规则：与 `${VAR}` 占位符可用的形状一致。
pub fn valid_var_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// 本次分发可见的取值快照。
///
/// 桌面端一个进程会连续分发很多次命令，因此不能一次性缓存到进程结束——
/// 每次分发开头重新加载，用户刚填的值当场生效。
///
/// 按线程存：一次 `execute` 的 refresh 与后续 lookup 都在同一个线程里，而同一进程
/// 内可能有多份互不相干的上下文（集成测试并行、桌面端多次分发），全局一份会互相顶掉。
thread_local! {
    static CACHE: RefCell<Option<BTreeMap<String, String>>> = const { RefCell::new(None) };
}

/// 分发入口调用：重新加载本机已保存的取值。库不存在或表还没建都按「空」处理，
/// 读失败同样降级为空而不是让整条命令失败——凭据缺失自有下游的 fail-closed 兜底。
pub fn refresh(ctx: &Context) {
    let loaded = load_all(ctx).unwrap_or_default();
    CACHE.with(|cache| *cache.borrow_mut() = Some(loaded));
}

fn load_all(ctx: &Context) -> Result<BTreeMap<String, String>> {
    let Some(conn) = crate::store::open_readonly(ctx)? else {
        return Ok(BTreeMap::new());
    };
    read_all(&conn)
}

pub fn read_all(conn: &Connection) -> Result<BTreeMap<String, String>> {
    let exists: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'secret_values'",
        [],
        |row| row.get(0),
    )?;
    let mut out = BTreeMap::new();
    if exists == 0 {
        return Ok(out);
    }
    let mut stmt = conn.prepare("SELECT name, value FROM secret_values ORDER BY name")?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    for row in rows {
        let (name, value) = row?;
        out.insert(name, value);
    }
    Ok(out)
}

/// 取一个变量的值：先本机已保存的，再回退进程环境。
pub fn lookup(name: &str) -> Option<String> {
    let stored = CACHE.with(|cache| {
        cache
            .borrow()
            .as_ref()
            .and_then(|map| map.get(name).cloned())
    });
    stored.or_else(|| std::env::var(name).ok())
}

/// 该变量当前能不能取到值。
pub fn is_set(name: &str) -> bool {
    lookup(name).is_some()
}

/// 这个变量的取值是不是存在本机库里（用于界面区分「已在这里填过」与「来自环境」）。
pub fn is_stored(name: &str) -> bool {
    CACHE.with(|cache| {
        cache
            .borrow()
            .as_ref()
            .is_some_and(|map| map.contains_key(name))
    })
}

pub fn ensure_table(conn: &Connection) -> Result<()> {
    conn.execute_batch(TABLE_DDL)?;
    Ok(())
}

/// 写入/覆盖一个变量的取值。空值按删除处理——界面上清空即撤销配置。
pub fn set(tx: &Transaction, name: &str, value: &str, now: &str) -> Result<()> {
    if !valid_var_name(name) {
        bail!("变量名不合法：要以字母或下划线开头，只能用字母、数字与下划线");
    }
    if value.is_empty() {
        tx.execute("DELETE FROM secret_values WHERE name = ?1", [name])?;
        return Ok(());
    }
    tx.execute(
        "INSERT INTO secret_values (name, value, updated_at) VALUES (?1, ?2, ?3)
         ON CONFLICT (name) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        rusqlite::params![name, value, now],
    )?;
    Ok(())
}

pub fn delete(tx: &Transaction, name: &str) -> Result<()> {
    tx.execute("DELETE FROM secret_values WHERE name = ?1", [name])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn var_name_rules_match_placeholder_shape() {
        assert!(valid_var_name("GROK_API_KEY"));
        assert!(valid_var_name("_x1"));
        assert!(!valid_var_name(""));
        assert!(!valid_var_name("1BAD"));
        assert!(!valid_var_name("has-dash"));
        assert!(!valid_var_name("has space"));
    }

    #[test]
    fn set_then_read_back_and_empty_value_deletes() {
        let mut conn = Connection::open_in_memory().unwrap();
        ensure_table(&conn).unwrap();
        let tx = conn.transaction().unwrap();
        set(&tx, "TOK", "v1", "2026-09-18T00:00:00Z").unwrap();
        tx.commit().unwrap();
        assert_eq!(
            read_all(&conn).unwrap().get("TOK").map(String::as_str),
            Some("v1")
        );

        let tx = conn.transaction().unwrap();
        set(&tx, "TOK", "", "2026-09-18T00:01:00Z").unwrap();
        tx.commit().unwrap();
        assert!(read_all(&conn).unwrap().is_empty());
    }

    #[test]
    fn set_rejects_illegal_name_without_writing() {
        let mut conn = Connection::open_in_memory().unwrap();
        ensure_table(&conn).unwrap();
        let tx = conn.transaction().unwrap();
        assert!(set(&tx, "bad-name", "v", "2026-09-18T00:00:00Z").is_err());
        tx.commit().unwrap();
        assert!(read_all(&conn).unwrap().is_empty());
    }

    #[test]
    fn missing_table_reads_as_empty() {
        let conn = Connection::open_in_memory().unwrap();
        assert!(read_all(&conn).unwrap().is_empty());
    }
}
