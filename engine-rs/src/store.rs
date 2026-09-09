//! SQLite backing store for the native core.
//!
//! - Database file: `<data_dir>/native-manager.db` (explicitly a NEW store; no
//!   migration from or compatibility with the legacy Python manager.db).
//! - Shared schema (this module owns it): schema_meta, policy_entries,
//!   operation_logs. Resource domains create their own tables; read paths must
//!   treat a missing domain table as empty and never create tables.
//! - `open_readonly` never creates directories or files: missing DB -> Ok(None).
//! - `open_write` creates the data dir, initializes the shared schema and
//!   freezes the builtin defaults into policy_entries on first write
//!   ("firstwrite固化"), so later reads see a stable persisted baseline.
//! - Write paths run on ONE connection with an explicit transaction and a busy
//!   timeout; there is no nested/double initialization (schema init happens in
//!   open_write before any user transaction starts).
//!
//! Builtin defaults are compiled in read-only: only the two reviewer-verified
//! official assistant definitions (no machine-specific config, no secrets, no
//! uninstall commands without verified evidence).

use std::path::PathBuf;

use anyhow::{anyhow, bail, Context as _, Result};
use rusqlite::{Connection, OpenFlags, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::native::Context;

pub const DB_FILE_NAME: &str = "native-manager.db";
pub const SCHEMA_VERSION: i64 = 1;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AppSettings {
    pub offline: bool,
    pub default_dry_run: bool,
    pub upstream_ttl_sec: u32,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            offline: false,
            default_dry_run: false,
            upstream_ttl_sec: 86_400,
        }
    }
}

/// Reviewer-verified official defaults (docs: claude.ai/install.sh native
/// install + manual `claude update`; chatgpt.com/codex/install.sh for both
/// install and update). Field shape keeps `providers[preferred_provider].
/// install/upgrade/uninstall` so the GUI needs no restructuring; uninstall is
/// omitted (no verified official command).
pub const BUILTIN_POLICY_JSON: &str = r#"{
  "agents": {
    "claude-code": {"enabled": true},
    "codex": {"enabled": true},
    "definitions": {
    "claude-code": {
      "display_name": "Claude Code", "bin": "claude", "skills_dir": "~/.claude/skills", "mcp_config": {"path": "~/.claude.json", "format": "json"},
      "preferred_provider": "native",
      "providers": {
        "native": {
          "install": "curl -fsSL https://claude.ai/install.sh | bash",
          "upgrade": "claude update", "managed": true
        }
      }
    },
    "codex": {
      "display_name": "Codex", "bin": "codex", "skills_dir": "~/.codex/skills", "mcp_config": {"path": "~/.codex/config.toml", "format": "toml"},
      "preferred_provider": "native",
      "providers": {
        "native": {
          "install": "curl -fsSL https://chatgpt.com/codex/install.sh | sh",
          "upgrade": "curl -fsSL https://chatgpt.com/codex/install.sh | sh", "managed": true
        }
      }
    }
      }
    },
  "mcp": {"definitions": {}},
  "skills": {
    "agent-skills": {"enabled": true},
    "repositories": {
      "agent-skills": {
        "url": "https://github.com/ly87ing/agent-skills.git",
        "skill_paths": [
          "safe-merge-review",
          "change-discipline",
          "frontend-verification",
          "artifact-hygiene",
          "reader-facing-writing",
          "technical-diagramming"
        ],
        "agents": [
          "claude-code",
          "codex"
        ]
      }
    }
  }
}"#;

pub fn builtin_policy() -> Value {
    serde_json::from_str(BUILTIN_POLICY_JSON).expect("builtin policy template is valid JSON")
}

/// Path of the native database (pure: never touches the filesystem).
pub fn db_path(ctx: &Context) -> PathBuf {
    ctx.data_dir.join(DB_FILE_NAME)
}

fn configure_connection(conn: &Connection) -> Result<()> {
    conn.busy_timeout(std::time::Duration::from_secs(30))?;
    Ok(())
}

/// Read-only connection. Missing DB -> Ok(None); existing but foreign/corrupt
/// (no shared schema_meta) -> Err, fail-closed without modifying the file.
/// Never creates directories, files, or tables.
pub fn open_readonly(ctx: &Context) -> Result<Option<Connection>> {
    let path = db_path(ctx);
    if !path.exists() {
        return Ok(None);
    }
    if !path.is_file() || path.is_symlink() {
        bail!("数据库路径不是普通文件，拒绝访问: {}", path.display());
    }
    let conn = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("无法打开数据库 {}", path.display()))?;
    configure_connection(&conn)?;
    let has_schema_meta: bool = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='schema_meta'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map(|count| count > 0)
        .unwrap_or(false);
    if !has_schema_meta {
        bail!(
            "数据库缺少共享 schema_meta 表 ({})，拒绝读取以保护原数据",
            path.display()
        );
    }
    Ok(Some(conn))
}

/// Shared schema. Domain tables are created by their owners, not here.
fn init_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS schema_meta (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS policy_entries (
            key TEXT PRIMARY KEY,
            value_json TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS operation_logs (
            id TEXT PRIMARY KEY,
            run_id TEXT,
            operation_type TEXT NOT NULL,
            target TEXT,
            detail_json TEXT,
            created_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS app_settings (
            key TEXT PRIMARY KEY,
            value_json TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        ",
    )?;
    Ok(())
}

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// Write connection: creates the data dir (0700 on unix), initializes the
/// shared schema once, and freezes the builtin defaults into policy_entries
/// when the store is first created. WAL + busy_timeout for cross-process
/// safety. The single returned connection is the only write handle — callers
/// run their transaction on it directly.
pub fn open_write(ctx: &Context) -> Result<Connection> {
    let path = db_path(ctx);
    if ctx.data_dir.exists() && (ctx.data_dir.is_symlink() || !ctx.data_dir.is_dir()) {
        bail!("数据目录不是普通目录，拒绝访问: {}", ctx.data_dir.display());
    }
    if path.exists() && (!path.is_file() || path.is_symlink()) {
        bail!("数据库路径不是普通文件，拒绝访问: {}", path.display());
    }
    let existing = path.is_file();
    let data_dir_existed = ctx.data_dir.exists();
    std::fs::create_dir_all(&ctx.data_dir)
        .with_context(|| format!("无法创建数据目录 {}", ctx.data_dir.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if !data_dir_existed {
            std::fs::set_permissions(&ctx.data_dir, std::fs::Permissions::from_mode(0o700))?;
        }
    }
    let mut conn =
        Connection::open(&path).with_context(|| format!("无法打开数据库 {}", path.display()))?;
    #[cfg(unix)]
    if !existing {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }
    configure_connection(&conn)?;
    if !existing {
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        // 另一个进程可能在本进程检查 path 后已经完成初始化；锁内复查避免重复插入。
        let already_initialized: bool = tx
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='schema_meta'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(0)
            > 0;
        if already_initialized {
            tx.commit()?;
            return Ok(conn);
        }
        init_schema(&tx)?;
        let now = now_iso();
        tx.execute(
            "INSERT INTO schema_meta (key, value) VALUES ('schema_version', ?1)",
            [SCHEMA_VERSION.to_string()],
        )?;
        tx.execute(
            "INSERT INTO schema_meta (key, value) VALUES ('initialized_at', ?1)",
            [now.clone()],
        )?;
        // firstwrite 固化：内置默认进入 policy_entries，之后读取以库内为准
        write_policy(&tx, &builtin_policy())?;
        tx.execute(
            "INSERT INTO operation_logs (id, run_id, operation_type, target, detail_json, created_at)
             VALUES (?1, NULL, 'db:init', NULL, NULL, ?2)",
            [uuid_v4(), now],
        )?;
        tx.commit()?;
    } else {
        // BEGIN IMMEDIATE 等待并发首次初始化提交后再验证；不会把半成品误判为外来库。
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let version: Option<String> = tx
            .query_row(
                "SELECT value FROM schema_meta WHERE key='schema_version'",
                [],
                |row| row.get(0),
            )
            .ok();
        match version.and_then(|raw| raw.parse::<i64>().ok()) {
            Some(ver) if ver <= SCHEMA_VERSION => tx.commit()?,
            Some(ver) => {
                tx.rollback()?;
                bail!("数据库版本 ({ver}) 高于当前程序支持版本 ({SCHEMA_VERSION})，拒绝访问")
            }
            None => {
                tx.rollback()?;
                bail!(
                    "数据库缺少 schema_version 记录 ({})，拒绝写入以保护原数据",
                    path.display()
                )
            }
        }
    }
    Ok(conn)
}

pub fn default_settings() -> AppSettings {
    AppSettings::default()
}

pub fn settings_from_connection(conn: &Connection) -> Result<AppSettings> {
    let mut settings = default_settings();
    let mut stmt = conn.prepare("SELECT key, value_json FROM app_settings")?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    for row in rows {
        let (key, raw) = row?;
        let mut object = serde_json::to_value(&settings)?
            .as_object()
            .cloned()
            .unwrap();
        object.insert(key, serde_json::from_str(&raw)?);
        settings = serde_json::from_value(Value::Object(object))?;
    }
    Ok(settings)
}

pub fn effective_settings(ctx: &Context) -> Result<AppSettings> {
    match open_readonly(ctx)? {
        Some(conn) => settings_from_connection(&conn),
        None => Ok(default_settings()),
    }
}

pub fn write_settings(conn: &Connection, settings: &AppSettings) -> Result<()> {
    let object = serde_json::to_value(settings)?
        .as_object()
        .cloned()
        .unwrap();
    // 序列化后的完整对象先校验，之后才开始写第一行。
    let validated: AppSettings = serde_json::from_value(Value::Object(object.clone()))?;
    let values = serde_json::to_value(validated)?
        .as_object()
        .cloned()
        .unwrap();
    for (key, item) in values {
        conn.execute("INSERT INTO app_settings (key, value_json, updated_at) VALUES (?1, ?2, ?3) ON CONFLICT(key) DO UPDATE SET value_json=excluded.value_json, updated_at=excluded.updated_at", rusqlite::params![key, serde_json::to_string(&item)?, now_iso()])?;
    }
    Ok(())
}

/// Rebuild the policy tree from policy_entries (one row per top-level key).
/// Shared schema only; missing rows simply mean an empty tree.
pub fn policy_from_connection(conn: &Connection) -> Result<Value> {
    let mut stmt = conn.prepare("SELECT key, value_json FROM policy_entries")?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut map = Map::new();
    for row in rows {
        let (key, raw) = row?;
        let value: Value = serde_json::from_str(&raw)
            .with_context(|| format!("配置项 {key} 的 JSON 数据损坏，拒绝输出半份配置"))?;
        map.insert(key, value);
    }
    Ok(Value::Object(map))
}

/// Effective policy: persisted store when the DB exists (builtin defaults were
/// frozen into it at first write); builtin defaults when it does not.
pub fn effective_policy(ctx: &Context) -> Result<Value> {
    match open_readonly(ctx)? {
        Some(conn) => policy_from_connection(&conn),
        None => Ok(builtin_policy()),
    }
}

/// Replace the whole policy tree with `value` (object; one row per top-level
/// key). Callers pass `&transaction` to compose with other writes atomically —
/// this function never opens its own transaction.
pub fn write_policy(conn: &Connection, value: &Value) -> Result<()> {
    let object = value
        .as_object()
        .ok_or_else(|| anyhow!("策略必须是 JSON 对象"))?;
    let now = now_iso();
    conn.execute("DELETE FROM policy_entries", [])?;
    for (key, item) in object {
        let raw = serde_json::to_string(item)?;
        conn.execute(
            "INSERT INTO policy_entries (key, value_json, updated_at) VALUES (?1, ?2, ?3)",
            rusqlite::params![key, raw, now],
        )?;
    }
    Ok(())
}

pub fn uuid_v4() -> String {
    // RFC 4122 v4 from /dev/urandom; no external dependency.
    let mut bytes = [0u8; 16];
    let filled = std::fs::File::open("/dev/urandom")
        .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut bytes))
        .is_ok();
    if !filled {
        // 兜底：时间戳+PID 混合（仅用于日志 id，非安全场景）
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        for (i, chunk) in bytes.chunks_mut(8).enumerate() {
            chunk.copy_from_slice(
                &(nanos
                    .wrapping_add((std::process::id() as u128) << 64)
                    .wrapping_add(i as u128))
                .to_le_bytes()[..8],
            );
        }
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15]
    )
}

/// Append one audit entry. Never stores secret-bearing payloads for sources;
/// callers choose the detail.
pub fn record_operation(
    conn: &Connection,
    run_id: Option<&str>,
    operation_type: &str,
    target: Option<&str>,
    detail: Option<&Value>,
) -> Result<()> {
    let detail_json = detail.map(serde_json::to_string).transpose()?;
    conn.execute(
        "INSERT INTO operation_logs (id, run_id, operation_type, target, detail_json, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![
            uuid_v4(),
            run_id,
            operation_type,
            target,
            detail_json,
            now_iso()
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::Context;

    fn ctx(name: &str) -> (std::path::PathBuf, Context) {
        let root =
            std::env::temp_dir().join(format!("am-settings-{}-{}", std::process::id(), name));
        let _ = std::fs::remove_dir_all(&root);
        (
            root.clone(),
            Context {
                data_dir: root.join("data"),
                home_dir: root.join("home"),
            },
        )
    }

    #[test]
    fn settings_default_read_is_non_mutating() {
        let (root, context) = ctx("default");
        assert_eq!(
            effective_settings(&context).unwrap(),
            AppSettings::default()
        );
        assert_eq!(AppSettings::default().upstream_ttl_sec, 86400);
        assert!(!context.data_dir.exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn settings_round_trip_and_serde_rejects_without_partial_write() {
        let (root, context) = ctx("roundtrip");
        let conn = open_write(&context).unwrap();
        let settings = AppSettings {
            offline: true,
            default_dry_run: true,
            upstream_ttl_sec: 123,
        };
        let tx = conn.unchecked_transaction().unwrap();
        write_settings(&tx, &settings).unwrap();
        tx.commit().unwrap();
        drop(conn);
        assert_eq!(effective_settings(&context).unwrap(), settings);
        let conn = open_write(&context).unwrap();
        let before: String = conn
            .query_row(
                "SELECT group_concat(key || '=' || value_json, ',') FROM app_settings",
                [],
                |r| r.get(0),
            )
            .unwrap();
        for raw in [
            r#"{"offline":"yes","default_dry_run":false,"upstream_ttl_sec":1}"#,
            r#"{"offline":false,"default_dry_run":false,"upstream_ttl_sec":-1}"#,
            r#"{"offline":false,"default_dry_run":false,"upstream_ttl_sec":1,"extra":true}"#,
        ] {
            assert!(serde_json::from_str::<AppSettings>(raw).is_err());
        }
        let after: String = conn
            .query_row(
                "SELECT group_concat(key || '=' || value_json, ',') FROM app_settings",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(before, after);
        let _ = std::fs::remove_dir_all(root);
    }
}
